//! Kobo WiFi sync (port of Swift `KoboSync`): fetch a book's EPUB from
//! the Calibre library, push it over the shell channel as a base64
//! heredoc (stock Kobo has no sftp/scp; raw binary on shared stdin
//! races the shell), size-verify, atomic `.part` + `mv`, then open.
//! New-file-only, guarded re-check — mirrors the Swift sequence.

use super::open::OpenOutcome;
use super::ssh::{run_shell_blocks, SshAuth};
use crate::db::FileSource;

#[derive(Debug)]
pub enum SyncError {
    NoEpub,
    EmptyFile,
    StorageFull,
    AlreadyThere,
    Transport(String),
}

impl SyncError {
    pub fn message(&self) -> String {
        match self {
            SyncError::NoEpub => {
                "No EPUB format in Calibre for this book — sync needs an EPUB.".to_string()
            }
            SyncError::EmptyFile => "Calibre's EPUB file is empty — not syncing.".to_string(),
            SyncError::StorageFull => "Kobo storage is full — free space and try again.".to_string(),
            SyncError::AlreadyThere => {
                "The book appeared on the Kobo since — just tap Read on Kobo again.".to_string()
            }
            SyncError::Transport(m) => format!("Sync failed: {}", m.chars().take(300).collect::<String>()),
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Minimal base64 encoder (76-char lines, like Swift
/// `.lineLength76Characters`) — avoids a new dependency for one call.
pub fn base64_lines(data: &[u8]) -> Vec<String> {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out.as_bytes()
        .chunks(76)
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('\'', "'\\''")
}

pub struct EpubBlob {
    pub bytes: Vec<u8>,
    pub uncompressed_size: i64,
    pub predicted: String,
    pub title: String,
}

/// Pick the EPUB format row for a book: `(file_rel, uncompressed_size)`.
/// Missing `data` table or no EPUB row → `NoEpub` (never a crash —
/// the slim fixture has no `data` table at all).
fn pick_epub_format(
    db: &rusqlite::Connection,
    book_id: i64,
    rel: &str,
) -> Result<(String, i64), SyncError> {
    let mut stmt = db
        .prepare("SELECT name, format, uncompressed_size FROM data WHERE book = ?1")
        .map_err(|_| SyncError::NoEpub)?;
    let rows = stmt
        .query_map([book_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2).unwrap_or(0),
            ))
        })
        .map_err(|_| SyncError::NoEpub)?;
    for row in rows {
        let (name, format, size) = row.map_err(|_| SyncError::NoEpub)?;
        if format.eq_ignore_ascii_case("EPUB") {
            return Ok((format!("{rel}/{name}.{}", format.to_ascii_lowercase()), size));
        }
    }
    Err(SyncError::NoEpub)
}

/// EPUB size probe for Sync & Open consent (no bytes fetched).
/// Returns uncompressed_size, or -1 when unknown.
pub async fn epub_size(src: &FileSource, book_id: i64) -> Result<i64, SyncError> {
    let db_bytes = src.read_db_bytes().await.map_err(|_| SyncError::NoEpub)?;
    if db_bytes.is_empty() {
        return Err(SyncError::NoEpub);
    }
    let db = crate::db::open_memory_db(&db_bytes).map_err(|_| SyncError::NoEpub)?;
    let rel: String = db
        .query_row("SELECT path FROM books WHERE id = ?1", [book_id], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| SyncError::NoEpub)?;
    pick_epub_format(&db, book_id, &rel).map(|(_, size)| size)
}

/// Locate a book's EPUB in the Calibre library (local or SMB) and read it.
pub async fn epub_for_book(src: &FileSource, book_id: i64) -> Result<EpubBlob, SyncError> {
    let db_bytes = src.read_db_bytes().await.map_err(|_| SyncError::NoEpub)?;
    if db_bytes.is_empty() {
        return Err(SyncError::NoEpub);
    }
    let db = crate::db::open_memory_db(&db_bytes).map_err(|_| SyncError::NoEpub)?;
    let (rel, title): (String, String) = db
        .query_row(
            "SELECT path, title FROM books WHERE id = ?1",
            [book_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| SyncError::NoEpub)?;
    let (author_sort, natural): (String, String) = db
        .query_row(
            "SELECT a.sort, a.name FROM authors a JOIN books_authors_link bal ON bal.author = a.id WHERE bal.book = ?1 LIMIT 1",
            [book_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| SyncError::NoEpub)?;
    let (file_rel, size) = pick_epub_format(&db, book_id, &rel)?;
    let bytes = match src {
        FileSource::Local { .. } => {
            let p = src.book_path(&file_rel);
            std::fs::read(&p).map_err(|_| SyncError::NoEpub)?
        }
        FileSource::Smb { loc, conn } => {
            let url = src.book_path(&file_rel);
            let remote = crate::covers::parse_smb_url(&url)
                .map(|(_, _, r)| r)
                .ok_or(SyncError::NoEpub)?;
            let (h, s, _) = crate::covers::parse_smb_url(&url).unwrap_or_default();
            if h != loc.host || s != loc.share {
                return Err(SyncError::NoEpub);
            }
            crate::smb::download_file_bytes(conn.clone(), &loc.share, &remote)
                .await
                .map_err(|_| SyncError::NoEpub)?
        }
    };
    if bytes.is_empty() {
        return Err(SyncError::EmptyFile);
    }
    let predicted = super::predicted_path(&title, &author_sort, Some(&natural));
    Ok(EpubBlob { bytes, uncompressed_size: size, predicted, title })
}

async fn push_bytes(
    ip: &str,
    predicted: &str,
    bytes: &[u8],
    auth: &SshAuth,
) -> Result<(), SyncError> {
    let need = bytes.len() as i64;
    // 1. Free-space guard (+1MB margin).
    let df = run_shell_blocks(ip, &["df -k /mnt/onboard | tail -n 1".to_string()], 10, auth.clone()).await;
    if df.code != 0 {
        return Err(SyncError::Transport(df.output));
    }
    let fields: Vec<&str> = df.output.split_whitespace().collect();
    let avail_kb: i64 = fields.get(3).and_then(|f| f.parse().ok()).unwrap_or(0);
    if avail_kb * 1024 < need + 1_048_576 {
        return Err(SyncError::StorageFull);
    }
    // 2. Author dir + don't clobber; clear stale .part.
    let dir = predicted.rfind('/').map(|i| &predicted[..i]).unwrap_or("");
    let part = format!("{predicted}.part");
    let prep = run_shell_blocks(
        ip,
        &[format!(
            "mkdir -p '{}' && rm -f '{}' && if [ -f '{}' ]; then echo present; else echo absent; fi",
            esc(dir),
            esc(&part),
            esc(predicted)
        )],
        10,
        auth.clone(),
    )
    .await;
    if prep.code != 0 {
        return Err(SyncError::Transport(prep.output));
    }
    if prep.output.contains("present") {
        return Err(SyncError::AlreadyThere);
    }
    // 3. Base64 heredoc push (delimiter outside the b64 alphabet, so
    // payload lines can never collide), size-verify, atomic mv.
    // Delimiter uses underscores (outside the base64 alphabet), so payload
    // lines can never collide with it.
    let mut blocks = vec![format!("base64 -d > '{}' <<'KOBO_EOF_DONE'", esc(&part))];
    let mut acc = String::new();
    for line in base64_lines(bytes) {
        acc.push_str(&line);
        acc.push('\n');
        if acc.len() >= 65536 {
            blocks.push(acc.clone());
            acc.clear();
        }
    }
    if !acc.is_empty() {
        blocks.push(acc);
    }
    blocks.push("KOBO_EOF_DONE".to_string());
    blocks.push(format!(
        "n=$(wc -c < '{}'); if [ \"$n\" -eq {need} ]; then mv '{}' '{}' && echo \"moved:$n\"; else rm -f '{}'; echo \"size-mismatch:$n\"; exit 1; fi",
        esc(&part),
        esc(&part),
        esc(predicted),
        esc(&part)
    ));
    let push = run_shell_blocks(ip, &blocks, 180, auth.clone()).await;
    if push.code != 0 || !push.output.contains("moved:") {
        return Err(SyncError::Transport(push.output));
    }
    // 4. Index cache learns the new path.
    let mut paths = super::cached_paths().unwrap_or_default();
    if !paths.iter().any(|p| p == predicted) {
        paths.push(predicted.to_string());
        super::save_paths(&paths);
    }
    Ok(())
}

/// Full Sync & Open: fetch EPUB, push, then KOReader-open. Mirrors
/// Swift `syncAndOpen` (size consent happens in UI from `Missing.size_bytes`).
pub async fn sync_and_open(
    src: &FileSource,
    book_id: i64,
    ip: &str,
    auth: SshAuth,
) -> OpenOutcome {
    let blob = match epub_for_book(src, book_id).await {
        Ok(b) => b,
        Err(e) => {
            return OpenOutcome::Failed { message: e.message(), code: 1 };
        }
    };
    if let Err(e) = push_bytes(ip, &blob.predicted, &blob.bytes, &auth).await {
        return OpenOutcome::Failed { message: e.message(), code: 1 };
    }
    let open = run_shell_blocks(ip, &[super::koreader_open_cmd(&blob.predicted)], 10, auth).await;
    if open.code != 0 {
        return OpenOutcome::Failed {
            message: format!("Synced but failed to open on Kobo: {}", open.output),
            code: open.code,
        };
    }
    OpenOutcome::Opened { path: blob.predicted, output: open.output }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_known_vectors() {
        assert_eq!(base64_lines(b"Man"), vec!["TWFu".to_string()]);
        assert_eq!(base64_lines(b"Ma"), vec!["TWE=".to_string()]);
        assert_eq!(base64_lines(b"M"), vec!["TQ==".to_string()]);
        assert_eq!(base64_lines(b""), Vec::<String>::new());
    }

    #[test]
    fn b64_wraps_at_76() {
        let lines = base64_lines(&[b'a'; 57]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 76);
        let lines2 = base64_lines(&[b'a'; 58]);
        assert_eq!(lines2.len(), 2);
    }

    #[test]
    fn epub_picker_formats() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE data(book INTEGER, name TEXT, format TEXT, uncompressed_size INTEGER);
             INSERT INTO data VALUES (1, 'Emma', 'PDF', 100);
             INSERT INTO data VALUES (1, 'Emma', 'EPUB', 200);
             INSERT INTO data VALUES (2, 'X', 'MOBI', 50);",
        )
        .unwrap();
        assert_eq!(
            pick_epub_format(&db, 1, "Austen/Emma (1)").unwrap(),
            ("Austen/Emma (1)/Emma.epub".to_string(), 200)
        );
        assert!(matches!(pick_epub_format(&db, 2, "r"), Err(SyncError::NoEpub)));
        assert!(matches!(pick_epub_format(&db, 9, "r"), Err(SyncError::NoEpub)));
    }

    #[test]
    fn epub_picker_needs_data_table() {
        // Slim DBs (like the fixture) have no `data` table → clean
        // NoEpub, never a crash.
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE books(id INTEGER PRIMARY KEY)").unwrap();
        assert!(matches!(pick_epub_format(&db, 1, "r"), Err(SyncError::NoEpub)));
    }
}
