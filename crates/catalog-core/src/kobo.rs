//! Port of `ReaderCatalogGUI/KoboPath.swift` + `KoboIndex.swift` +
//! the `KoboLauncher` enum in `ReaderCatalogApp.swift`.
//!
//! `KoboPath` replicates `calibre/devices/kobo/driver.py`
//! `create_upload_path` (MAX_PATH_LEN=185, FAT sanitize, title_sort).
//! The launcher shells `/usr/bin/ssh` exactly like Swift's `Process`
//! heredoc path (Dropbear requires stdin, one-shot `ssh host cmd`
//! returns empty).

pub mod handoff;

pub const MAX_PATH_LEN: usize = 185;
pub const PREFIX: &str = "/mnt/onboard";

// ---------------------------------------------------------------------------
// KoboPath (KoboPath.swift)
// ---------------------------------------------------------------------------

fn is_sanitize_char(c: char) -> bool {
    (c as u32) <= 0x1F || matches!(c, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' | '+')
}

/// calibre title_sort: move leading A/An/The to the end with a comma.
pub fn title_sort(title: &str) -> String {
    let t = title.trim();
    for prefix in ["A ", "An ", "The "] {
        if t.len() > prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let rest = t[prefix.len()..].trim();
            return format!("{}, {}", rest, prefix.trim());
        }
    }
    t.to_string()
}

/// calibre author_to_author_sort (comma method, simplified).
pub fn author_sort(natural: &str) -> String {
    let a = natural.trim();
    if a.is_empty() {
        return String::new();
    }
    if a.contains(',') {
        return a.to_string();
    }
    let tokens: Vec<&str> = a.split_whitespace().collect();
    if tokens.len() < 2 {
        return a.to_string();
    }
    format!("{}, {}", tokens.last().unwrap(), tokens[..tokens.len() - 1].join(" "))
}

pub fn natural_name(sort: &str) -> String {
    let mut parts = sort.splitn(2, ',');
    match (parts.next(), parts.next()) {
        (Some(last), Some(first)) => format!("{} {}", first.trim(), last.trim()),
        _ => sort.to_string(),
    }
}

fn fold_diacritics(s: &str) -> String {
    // Minimal fold covering observed library cases (Carré→Carre) plus
    // common Latin-1; full unicode-normalization would need `deunicode`.
    s.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
            'È' | 'É' | 'Ê' | 'Ë' => 'E',
            'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
            'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
            'Ç' => 'C',
            'Ñ' => 'N',
            'ÿ' | 'ý' => 'y',
            'œ' => 'e',
            _ => c,
        })
        .collect()
}

/// calibre sanitize_file_name + Kobo ascii handling ('→_).
pub fn sanitize(name: &str) -> String {
    let ascii = fold_diacritics(name).replace('\'', "_");
    let mut one: String = ascii.chars().map(|c| if is_sanitize_char(c) { '_' } else { c }).collect();
    // collapse whitespace → single space, strip
    let collapsed = one.split_whitespace().collect::<Vec<_>>().join(" ");
    one = collapsed.trim().to_string();
    let (mut bname, ext) = match one.rfind('.') {
        Some(i) => (one[..i].to_string(), one[i + 1..].to_string()),
        None => (one.clone(), String::new()),
    };
    if !bname.is_empty() && bname.chars().all(|c| c == '.') {
        bname = "_".to_string();
    }
    let mut result = bname.replace("..", "_");
    if !ext.is_empty() {
        result.push('.');
        result.push_str(&ext);
    } else if one.is_empty() {
        result = one.clone();
    }
    if result.ends_with('.') || result.ends_with(' ') {
        result.pop();
        result.push('_');
    }
    if result.starts_with('.') {
        result = format!("_{}", &result[1..]);
    }
    result
}

fn encoded_len(s: &str) -> usize {
    // macOS utf-16 filename length rule: 2 bytes per code unit.
    s.encode_utf16().count() * 2
}

fn shorten_component(s: &str, by: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let l = chars.len();
    if l <= by {
        return s.to_string();
    }
    let keep = (l - by) / 2;
    if keep == 0 {
        return chars[..l - by.min(l - 1)].iter().collect();
    }
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[l - keep..].iter().collect();
    format!("{head}{tail}")
}

/// Predicted Kobo path (mirrors `KoboPath.predictedPath`).
pub fn predicted_path(title: &str, author_sort_value: &str, authors_natural: Option<&str>) -> String {
    let a_sort = sanitize(author_sort_value);
    let t_sort = sanitize(&title_sort(title));
    let authors = sanitize(authors_natural.unwrap_or(&natural_name(author_sort_value)));
    let file = sanitize(&format!("{t_sort} - {authors}.kepub.epub"));
    let mut comps = [a_sort, file];
    let prefix_len = encoded_len(PREFIX);
    let max_comps = MAX_PATH_LEN * 2 - prefix_len - 2;
    if encoded_len(&comps.join("/")) > max_comps {
        // Simple proportional shorten (mirrors Swift fallback).
        let over = encoded_len(&format!("{PREFIX}/{}", comps.join("/"))) - MAX_PATH_LEN * 2;
        let by_chars = (over / 2).max(10);
        let short_title = shorten_component(&t_sort, by_chars);
        comps[1] = sanitize(&format!("{short_title} - {authors}.kepub.epub"));
    }
    format!("{PREFIX}/{}", comps.join("/"))
}

// ---------------------------------------------------------------------------
// KoboIndex (KoboIndex.swift) — kobo_index.txt in the platform data dir.
// ---------------------------------------------------------------------------

pub fn cached_paths() -> Option<Vec<String>> {
    let data = std::fs::read_to_string(crate::settings::kobo_index_file()).ok()?;
    let lines: Vec<String> =
        data.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

pub fn save_paths(paths: &[String]) {
    let file = crate::settings::kobo_index_file();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, paths.join("\n") + "\n");
}

pub fn contains_path(path: &str, cache: Option<&[String]>) -> bool {
    if let Some(c) = cache {
        return c.iter().any(|p| p == path);
    }
    cached_paths().map(|c| c.iter().any(|p| p == path)).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Launcher (KoboLauncher — ssh heredoc, strict fail-closed matching)
// ---------------------------------------------------------------------------

fn norm(s: &str) -> String {
    fold_diacritics(s)
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn last_name(author: &str) -> String {
    let t = author.trim();
    if let Some((last, _)) = t.split_once(',') {
        last.trim().to_string()
    } else {
        t.split_whitespace().last().unwrap_or(t).to_string()
    }
}

/// Strict match: ALL title tokens (minus a/an/the) + author last name,
/// diacritic-insensitive. Returns (strict, title_only) partitions.
pub fn strict_match<'a>(candidates: &'a [String], title: &str, author: &str) -> (Vec<&'a String>, Vec<&'a String>) {
    let tokens: Vec<String> = title
        .split_whitespace()
        .map(norm)
        .filter(|t| !t.is_empty() && t != "a" && t != "an" && t != "the")
        .collect();
    let required: Vec<String> = if tokens.is_empty() {
        let n = norm(title);
        if n.is_empty() { vec![] } else { vec![n] }
    } else {
        tokens
    };
    let last = norm(&last_name(author));
    let strict: Vec<&String> = candidates
        .iter()
        .filter(|p| {
            let n = norm(p);
            required.iter().all(|t| n.contains(t)) && (last.is_empty() || n.contains(&last))
        })
        .collect();
    let title_only: Vec<&String> = if strict.is_empty() {
        candidates
            .iter()
            .filter(|p| {
                let n = norm(p);
                required.iter().all(|t| n.contains(t))
            })
            .collect()
    } else {
        vec![]
    };
    (strict, title_only)
}

pub struct SshResult {
    pub output: String,
    pub code: i32,
}

/// Internal SSH executor mirroring Swift `sshSync` (stdin heredoc,
/// file-backed stdout to avoid pipe deadlock, local deadline).
pub async fn ssh_sync(ip: &str, remote_cmd: &str, timeout_secs: u64) -> SshResult {
    use tokio::io::AsyncWriteExt;
    // Use "ssh" not "/usr/bin/ssh" for Windows parity.
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args([
        "-T",
        "-o",
        &format!("ConnectTimeout={timeout_secs}"),
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        &format!("root@{ip}"),
    ]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return SshResult { output: format!("ssh launch failed: {e}"), code: -1 },
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{remote_cmd}\n").as_bytes()).await;
    }
    // wait_with_output takes ownership; on timeout the child is dropped
    // (kill_on_drop) since we cannot reclaim the handle afterwards.
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs + 2),
        child.wait_with_output(),
    )
    .await;
    match out {
        Ok(Ok(o)) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            SshResult { output: text, code: o.status.code().unwrap_or(-1) }
        }
        Ok(Err(e)) => SshResult { output: format!("ssh wait failed: {e}"), code: -1 },
        Err(_) => SshResult { output: "(timed out)".into(), code: 124 },
    }
}

pub fn koreader_open_cmd(chosen: &str) -> String {
    let esc = chosen.replace('\'', "'\\''").replace('"', "\\\"");
    format!(
        "if [ -x /mnt/onboard/.adds/koreader/koreader.sh ]; then K=/mnt/onboard/.adds/koreader/koreader.sh; else K=/mnt/onboard/koreader/koreader.sh; fi; \
         if [ ! -f '{esc}' ]; then echo \"not found: {esc}\"; exit 1; fi; \
         nohup \"$K\" '{esc}' >/tmp/koreader-open.log 2>&1 & sleep 1; \
         ps | grep -E \"koreader|reader.lua\" | head -n 5; echo \"launched: {esc}\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_sort_moves_article() {
        assert_eq!(title_sort("The Emma"), "Emma, The");
        assert_eq!(title_sort("A Study"), "Study, A");
        assert_eq!(title_sort("Emma"), "Emma");
    }

    #[test]
    fn sanitize_kobo_fat() {
        assert_eq!(sanitize("Smiley's"), "Smiley_s");
        assert_eq!(sanitize("Carré, John le"), "Carre, John le");
    }

    #[test]
    fn predicted_path_shape() {
        let p = predicted_path("Emma", "Austen, Jane", None);
        assert!(p.starts_with("/mnt/onboard/Austen, Jane/"));
        assert!(p.ends_with("Emma - Jane Austen.kepub.epub"));
    }

    #[test]
    fn strict_match_fail_closed() {
        let cands = vec!["/mnt/onboard/Austen, Jane/Emma - Jane Austen.kepub.epub".to_string()];
        let (strict, _) = strict_match(&cands, "Emma", "Austen, Jane");
        assert_eq!(strict.len(), 1);
        let (strict2, _) = strict_match(&cands, "Corpse", "Bruen");
        assert!(strict2.is_empty());
    }
}
