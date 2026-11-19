//! Kobo open orchestration (port of C# `RunKoboSsh` / Swift launcher):
//! probe → predicted-path exists check → cached/live find with
//! refresh-on-miss → identity match (title + author, deduped) → the shell
//! decides (auto-open / tie-break / different-author offer) → `open_path`
//! launches KOReader.
//! Transport is `ssh::run_shell_blocks`; matching math is `super::*`.

use super::ssh::{run_shell_blocks, SshAuth};
use super::{
    cached_paths, classify, koreader_open_cmd, parse_candidate, predicted_path, save_paths,
    Candidate, Tier,
};

pub enum OpenOutcome {
    Opened { path: String, output: String },
    Failed { message: String, code: i32 },
}

impl OpenOutcome {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            OpenOutcome::Opened { path, output } => {
                serde_json::json!({"status": "opened", "path": path, "output": output})
            }
            OpenOutcome::Failed { message, code } => {
                serde_json::json!({"status": "failed", "message": message, "code": code})
            }
        }
    }
}

/// Identification result. `Ambiguous` is gone on purpose: a tie is a list the
/// shell can show, not a dead end, and "no title-level match" is `Missing`
/// (which offers Sync) rather than a dump of shared-word noise.
pub enum Resolution {
    Found {
        candidates: Vec<Candidate>,
        author_matched: bool,
        auto: Option<String>,
        candidate_count: usize,
    },
    Missing {
        message: String,
        predicted: String,
        size_bytes: i64,
        candidate_count: usize,
    },
    Failed {
        message: String,
        code: i32,
    },
}

impl Resolution {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Resolution::Found { candidates, author_matched, auto, candidate_count } => {
                serde_json::json!({
                    "status": "found",
                    "auto": auto,
                    "author_matched": author_matched,
                    "candidate_count": candidate_count,
                    "candidates": candidates.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
                })
            }
            Resolution::Missing { message, predicted, size_bytes, candidate_count } => {
                serde_json::json!({
                    "status": "missing",
                    "message": message,
                    "predicted": predicted,
                    "size_bytes": size_bytes,
                    "candidate_count": candidate_count,
                })
            }
            Resolution::Failed { message, code } => {
                serde_json::json!({"status": "failed", "message": message, "code": code})
            }
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn last_name(author: &str) -> String {
    let t = author.trim();
    if let Some((last, _)) = t.split_once(',') {
        return last.trim().to_string();
    }
    t.split_whitespace().last().unwrap_or(t).to_string()
}

async fn find_books(ip: &str, auth: &SshAuth) -> Result<Vec<String>, (String, i32)> {
    let list = run_shell_blocks(
        ip,
        &["find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" -not -name \"._*\" | sort".to_string()],
        30,
        auth.clone(),
    )
    .await;
    if list.code != 0
        && !(list.output.contains("/mnt/onboard/") && list.output.contains(".epub"))
    {
        return Err((format!("Failed to list books on Kobo: {}", list.output), list.code));
    }
    // Absolute paths only — the shell echoes our command + prompt
    // lines into the buffer; those never start with /.
    Ok(list
        .output
        .lines()
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .map(str::to_string)
        .collect())
}

/// Cached index when we have one, otherwise a live find (which refreshes it).
async fn load_paths(ip: &str, auth: &SshAuth) -> Result<Vec<String>, (String, i32)> {
    let cached = cached_paths().unwrap_or_default();
    if !cached.is_empty() {
        return Ok(cached);
    }
    let found = find_books(ip, auth).await?;
    if found.is_empty() {
        return Err((
            "No books found on Kobo (find returned empty) — is /mnt/onboard mounted?".to_string(),
            1,
        ));
    }
    save_paths(&found);
    Ok(found)
}

/// Identify a book on the device. Never opens anything and never prompts —
/// the shell owns those decisions.
pub async fn resolve_on_kobo(
    ip: &str,
    title: &str,
    author: &str,
    auth: SshAuth,
    library: Option<(&crate::db::FileSource, i64)>,
) -> Resolution {
    // 1. Reachability probe.
    let probe = run_shell_blocks(ip, &["echo ok".to_string()], 5, auth.clone()).await;
    if probe.code == 255 {
        return Resolution::Failed { message: probe.output, code: 255 };
    }
    if probe.code != 0 || !probe.output.to_lowercase().contains("ok") {
        let hint = if probe.output.trim().is_empty() {
            format!("Kobo is sleeping or unreachable at {ip} — press power button to wake, then try again.")
        } else if probe.output.contains("sleeping") {
            probe.output.clone()
        } else {
            format!(
                "Kobo is sleeping or unreachable at {ip} — press power button to wake.\n{}",
                probe.output
            )
        };
        let code = if probe.code != 0 { probe.code } else { 7 };
        return Resolution::Failed { message: hint, code };
    }

    let title_trim = title.trim();
    let author_trim = author.trim();
    let who = {
        let last = last_name(author_trim);
        if last.is_empty() { author_trim.to_string() } else { last }
    };

    // 2. Predicted path — this app's own sync target, so it is authoritative.
    let predicted = predicted_path(
        title_trim,
        &super::author_sort(author_trim),
        Some(&super::natural_name(author_trim)),
    );
    let check = run_shell_blocks(
        ip,
        &[format!(
            "if [ -f '{}' ]; then echo \"exists:{predicted}\"; else echo \"missing\"; fi",
            esc(&predicted)
        )],
        6,
        auth.clone(),
    )
    .await;
    // Response line starts with "exists:" — the echoed command line
    // contains `echo "exists:..."` mid-line, so Contains would false-positive.
    if check.output.lines().any(|l| l.starts_with("exists:")) {
        let candidates = parse_candidate(&predicted)
            .map(|identity| vec![Candidate { identity, tier: Tier::Exact }])
            .unwrap_or_default();
        return Resolution::Found {
            candidates,
            author_matched: true,
            auto: Some(predicted),
            candidate_count: 1,
        };
    }

    // 3. Fallback index: cached, else a live find.
    let mut paths = match load_paths(ip, &auth).await {
        Ok(p) => p,
        Err((message, code)) => return Resolution::Failed { message, code },
    };
    let mut m = classify(&paths, title_trim, author_trim);

    // 4. Refresh-on-miss once: the cached index may predate the synced book.
    if m.candidates.is_empty() {
        match find_books(ip, &auth).await {
            Ok(fresh) if !fresh.is_empty() => {
                save_paths(&fresh);
                paths = fresh;
                m = classify(&paths, title_trim, author_trim);
            }
            Ok(_) => {}
            Err((message, code)) => return Resolution::Failed { message, code },
        }
    }

    if m.candidates.is_empty() {
        // Consent size for a later Sync & Open, fetched only on this path.
        let size_bytes = match &library {
            Some((src, book_id)) => super::sync::epub_size(src, *book_id).await.unwrap_or(-1),
            None => -1,
        };
        return Resolution::Missing {
            message: format!(
                "No match for \"{title_trim}\" by {who} on the Kobo — {} books checked, none with this title.\nExpected: {}\nCheck the Calibre Kobo template or re-sync Kobo.",
                m.total_paths,
                predicted.replace("/mnt/onboard/", "")
            ),
            predicted,
            size_bytes,
            candidate_count: m.total_paths,
        };
    }

    Resolution::Found {
        candidates: m.candidates,
        author_matched: m.author_matched,
        auto: m.auto,
        candidate_count: m.total_paths,
    }
}

/// Launch KOReader on an explicit device path (already chosen by the shell).
pub async fn open_path_on_kobo(ip: &str, path: &str, auth: SshAuth) -> OpenOutcome {
    let open = run_shell_blocks(ip, &[koreader_open_cmd(path)], 10, auth).await;
    if open.code != 0 {
        return OpenOutcome::Failed {
            message: format!("Failed to open on Kobo: {}", open.output),
            code: open.code,
        };
    }
    OpenOutcome::Opened { path: path.to_string(), output: open.output }
}
