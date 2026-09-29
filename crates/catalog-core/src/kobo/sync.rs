//! Kobo WiFi sync (port of Swift `KoboSync`): fetch a book's EPUB from
//! the Calibre library, push it over the shell channel as base64
//! heredocs (stock Kobo has no sftp/scp; raw binary on shared stdin
//! races the shell), size-verify, atomic `.part` + `mv`, then open.
//! New-file-only, guarded re-check — mirrors the Swift sequence.
//!
//! Chunked + resumable (2026-09-28): the old single-heredoc push sent a
//! whole book (47 MB of base64 for a 35 MB EPUB) through one shell
//! invocation — busybox ash buffers the entire heredoc before `base64 -d`
//! even starts, and at the observed Kobo rate (~40 KB/s) anything over a
//! few MB outran the timeout with nothing kept. Now each 1 MiB decoded
//! chunk rides its own connection + heredoc with a per-chunk deadline and
//! cumulative size check; the `.part` is kept on failure so a retry (or a
//! force-quit + retry) resumes from the last whole chunk instead of zero.
//! Progress (`done`, `total` decoded bytes) is published to statics the
//! FFI `catalog_kobo_progress` getter reads for the shell's status bar.

use super::open::OpenOutcome;
use super::ssh::{run_shell_blocks, SshAuth};
use crate::db::FileSource;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Sync progress for the shell status bar (`catalog_kobo_progress` reads
/// these): decoded bytes landed on the Kobo vs total. Relaxed ordering —
/// single writer (the sync worker), polled readers.
static SYNC_DONE: AtomicU64 = AtomicU64::new(0);
static SYNC_TOTAL: AtomicU64 = AtomicU64::new(0);

/// `(done, total)` decoded bytes for the in-flight (or last) push.
pub fn progress() -> (u64, u64) {
    (SYNC_DONE.load(Ordering::Relaxed), SYNC_TOTAL.load(Ordering::Relaxed))
}

fn set_progress(done: u64, total: u64) {
    SYNC_DONE.store(done, Ordering::Relaxed);
    SYNC_TOTAL.store(total, Ordering::Relaxed);
}

/// Cumulative channel-phase millis since the sync started (connect, auth,
/// xfer) — the `[kobo]` log line's stall attribution. Reset with progress.
pub fn phase_ms() -> (u64, u64, u64) {
    super::ssh::phase_ms()
}

/// 1 MiB decoded per chunk: one SSH connection + heredoc each, so a stall
/// fails fast (one chunk deadline) with everything so far kept for resume.
/// Small enough that busybox ash buffers it comfortably; large enough that
/// auth overhead (~1s/chunk) stays trivial.
const CHUNK: u64 = 1_048_576;
/// Per-chunk deadline: at the worst observed Kobo rate (~40 KB/s) a full
/// chunk lands in ~26s + auth; 120s is the backstop, not the budget.
const CHUNK_TIMEOUT: u64 = 120;

/// Resume decision for an existing `.part` of `existing` bytes against a
/// `need`-byte push: the byte offset to continue from, or `None` when the
/// partial can't be trusted (overshoot, or a torn trailing chunk) and the
/// caller must restart from zero. `Some(need)` means already complete.
fn resume_offset(existing: u64, need: u64) -> Option<u64> {
    if existing > need || !existing.is_multiple_of(CHUNK) {
        None
    } else {
        Some(existing)
    }
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
    let need = bytes.len() as u64;
    // 1. Free-space guard (+1MB margin).
    let df = run_shell_blocks(ip, &["df -k /mnt/onboard | tail -n 1".to_string()], 10, auth.clone()).await;
    if df.code != 0 {
        return Err(SyncError::Transport(df.output));
    }
    let fields: Vec<&str> = df.output.split_whitespace().collect();
    let avail_kb: u64 = fields.get(3).and_then(|f| f.parse().ok()).unwrap_or(0);
    if avail_kb * 1024 < need + 1_048_576 {
        return Err(SyncError::StorageFull);
    }
    // 2. Author dir + don't clobber. A `.part` from an interrupted push
    // is resume fuel, not garbage — only a torn trailing chunk (or an
    // overshoot, which should be impossible) forces a restart.
    let dir = predicted.rfind('/').map(|i| &predicted[..i]).unwrap_or("");
    let part = format!("{predicted}.part");
    let prep = run_shell_blocks(
        ip,
        &[format!(
            "mkdir -p '{}' && if [ -f '{}' ]; then echo present; else echo absent; fi",
            esc(dir),
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
    let size_probe = run_shell_blocks(
        ip,
        &[format!("if [ -f '{}' ]; then wc -c < '{}'; else echo 0; fi", esc(&part), esc(&part))],
        10,
        auth.clone(),
    )
    .await;
    if size_probe.code != 0 {
        return Err(SyncError::Transport(size_probe.output));
    }
    let existing: u64 = size_probe
        .output
        .lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .next_back()
        .unwrap_or(0);
    let start = match resume_offset(existing, need) {
        Some(off) => off,
        None => {
            let _ = run_shell_blocks(ip, &[format!("rm -f '{}'", esc(&part))], 10, auth.clone()).await;
            0
        }
    };
    set_progress(start, need);
    // 3. Chunked base64 heredoc push: one connection per 1 MiB decoded,
    // cumulative size check after each. Delimiter uses underscores (outside
    // the base64 alphabet), so payload lines can never collide with it.
    let mut offset = start;
    while offset < need {
        let end = (offset + CHUNK).min(need);
        let mut blocks = vec![format!("base64 -d >> '{}' <<'KOBO_CHUNK_DONE'", esc(&part))];
        let mut acc = String::new();
        for line in base64_lines(&bytes[offset as usize..end as usize]) {
            acc.push_str(&line);
            acc.push('\n');
            if acc.len() >= 65536 {
                blocks.push(std::mem::take(&mut acc));
            }
        }
        if !acc.is_empty() {
            blocks.push(std::mem::take(&mut acc));
        }
        blocks.push("KOBO_CHUNK_DONE".to_string());
        blocks.push(format!("n=$(wc -c < '{}'); echo \"chunk:$n\"", esc(&part)));
        let push = run_shell_blocks(ip, &blocks, CHUNK_TIMEOUT, auth.clone()).await;
        if push.code == 124 {
            return Err(SyncError::Transport(format!(
                "{} (chunk timed out at {offset}/{need} bytes — retry resumes)",
                push.output.chars().take(200).collect::<String>()
            )));
        }
        let landed: u64 = push
            .output
            .lines()
            .filter_map(|l| l.trim().strip_prefix("chunk:"))
            .filter_map(|v| v.trim().parse::<u64>().ok())
            .next_back()
            .unwrap_or(u64::MAX);
        if push.code != 0 || landed != end {
            return Err(SyncError::Transport(format!(
                "Kobo kept {landed} of {end} bytes — retry resumes ({offset}/{need} done)"
            )));
        }
        offset = end;
        set_progress(offset, need);
    }
    // 4. Size-verify + atomic mv (unchanged semantics: the final path
    // appears all at once, never half-written). Per-chunk checks already
    // passed, so this is the belt-and-suspenders gate — and the `.part`
    // stays for resume even here.
    let fin = run_shell_blocks(
        ip,
        &[format!(
            "n=$(wc -c < '{}'); if [ \"$n\" -eq {need} ]; then mv '{}' '{}' && echo \"moved:$n\"; else echo \"size-mismatch:$n\"; exit 1; fi",
            esc(&part),
            esc(&part),
            esc(predicted)
        )],
        30,
        auth.clone(),
    )
    .await;
    if fin.code != 0 || !fin.output.contains("moved:") {
        return Err(SyncError::Transport(format!(
            "{} — retry resumes",
            fin.output.chars().take(250).collect::<String>()
        )));
    }
    set_progress(need, need);
    // 5. Index cache learns the new path.
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
    set_progress(0, 0); // fetch phase: alive, no total yet (bar runs busy)
    super::ssh::reset_phase_ms();
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
    fn resume_offset_vectors() {
        // Fresh: start at zero. Whole chunks: continue. Exact: done.
        assert_eq!(resume_offset(0, 100), Some(0));
        assert_eq!(resume_offset(CHUNK, 3 * CHUNK), Some(CHUNK));
        assert_eq!(resume_offset(3 * CHUNK, 3 * CHUNK), Some(3 * CHUNK));
        // Torn trailing chunk or overshoot: restart, never append mid-chunk.
        assert_eq!(resume_offset(100, 3 * CHUNK), None);
        assert_eq!(resume_offset(CHUNK + 7, 3 * CHUNK), None);
        assert_eq!(resume_offset(4 * CHUNK, 3 * CHUNK), None);
    }

    #[test]
    fn progress_roundtrip() {
        set_progress(7, 42);
        assert_eq!(progress(), (7, 42));
        set_progress(0, 0);
        assert_eq!(progress(), (0, 0));
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
