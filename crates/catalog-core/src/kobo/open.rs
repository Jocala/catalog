//! Kobo open orchestration (port of C# `RunKoboSsh` / Swift launcher):
//! probe → predicted-path exists check → cached/live find with
//! refresh-on-miss → strict/title-only fail-closed match → KOReader open.
//! Transport is `ssh::run_shell_blocks`; matching math is `super::*`.

use super::ssh::{run_shell_blocks, SshAuth};
use super::{cached_paths, koreader_open_cmd, predicted_path, save_paths, strict_match};

pub enum OpenOutcome {
    Opened { path: String, output: String },
    Missing { message: String, predicted: String, size_bytes: i64 },
    Ambiguous { message: String },
    Failed { message: String, code: i32 },
}

impl OpenOutcome {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            OpenOutcome::Opened { path, output } => {
                serde_json::json!({"status": "opened", "path": path, "output": output})
            }
            OpenOutcome::Missing { message, predicted, size_bytes } => {
                serde_json::json!({"status": "missing", "message": message,
                    "predicted": predicted, "size_bytes": size_bytes})
            }
            OpenOutcome::Ambiguous { message } => {
                serde_json::json!({"status": "ambiguous", "message": message})
            }
            OpenOutcome::Failed { message, code } => {
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
        &["find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort".to_string()],
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

pub async fn open_on_kobo(
    ip: &str,
    title: &str,
    author: &str,
    auth: SshAuth,
    library: Option<(&crate::db::FileSource, i64)>,
) -> OpenOutcome {
    // 1. Reachability probe.
    let probe = run_shell_blocks(ip, &["echo ok".to_string()], 5, auth.clone()).await;
    if probe.code == 255 {
        return OpenOutcome::Failed { message: probe.output, code: 255 };
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
        return OpenOutcome::Failed { message: hint, code };
    }

    let title_trim = title.trim();
    let author_trim = author.trim();
    let last = last_name(author_trim);

    // 2. Predicted path (mirrors Swift/Calibre math in `super`).
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
    let found_predicted = check.output.lines().any(|l| l.starts_with("exists:"));
    let mut candidates: Vec<String> = Vec::new();
    if !found_predicted {
        // 3. Fallback: cached index or live find, refresh-on-miss once.
        let cached = cached_paths().unwrap_or_default();
        candidates = if cached.is_empty() {
            match find_books(ip, &auth).await {
                Ok(c) => c,
                Err((m, c)) => return OpenOutcome::Failed { message: m, code: c },
            }
        } else {
            cached
        };
        if candidates.is_empty() {
            return OpenOutcome::Failed {
                message: "No books found on Kobo (find returned empty) — is /mnt/onboard mounted?"
                    .to_string(),
                code: 1,
            };
        }
        if !candidates.is_empty() {
            save_paths(&candidates);
        }
    }

    let ctx = MatchCtx {
        title_trim,
        author_trim,
        last,
        predicted: predicted.clone(),
        candidate_count: candidates.len(),
        library,
    };
    let chosen = if found_predicted {
        predicted.clone()
    } else {
        let (strict, title_only) = strict_match(&candidates, title_trim, author_trim);
        match resolve(&ctx, strict, title_only).await {
            Ok(p) => p,
            Err(OpenOutcome::Missing { .. }) => {
                // Refresh-on-miss: cached index may predate synced books.
                let fresh = match find_books(ip, &auth).await {
                    Ok(c) => c,
                    Err((m, c)) => return OpenOutcome::Failed { message: m, code: c },
                };
                if !fresh.is_empty() {
                    save_paths(&fresh);
                    candidates = fresh;
                }
                let (strict2, title_only2) =
                    strict_match(&candidates, title_trim, author_trim);
                let ctx2 = MatchCtx {
                    title_trim,
                    author_trim,
                    last: ctx.last.clone(),
                    predicted: predicted.clone(),
                    candidate_count: candidates.len(),
                    library,
                };
                match resolve(&ctx2, strict2, title_only2).await {
                    Ok(p) => p,
                    Err(o) => return o,
                }
            }
            Err(o) => return o,
        }
    };

    // 4. Open via KOReader.
    let open = run_shell_blocks(ip, &[koreader_open_cmd(&chosen)], 10, auth).await;
    if open.code != 0 {
        return OpenOutcome::Failed {
            message: format!("Failed to open on Kobo: {}", open.output),
            code: open.code,
        };
    }
    OpenOutcome::Opened { path: chosen, output: open.output }
}

struct MatchCtx<'a> {
    title_trim: &'a str,
    author_trim: &'a str,
    last: String,
    predicted: String,
    candidate_count: usize,
    library: Option<(&'a crate::db::FileSource, i64)>,
}

async fn resolve(
    ctx: &MatchCtx<'_>,
    strict: Vec<&String>,
    title_only: Vec<&String>,
) -> Result<String, OpenOutcome> {
    let title_trim = ctx.title_trim;
    let author_trim = ctx.author_trim;
    let last = ctx.last.as_str();
    let predicted = ctx.predicted.as_str();
    let candidate_count = ctx.candidate_count;
    if strict.len() == 1 {
        return Ok(strict[0].clone());
    }
    if strict.is_empty() && title_only.len() == 1 {
        return Ok(title_only[0].clone());
    }
    if strict.is_empty() && title_only.is_empty() {
        let who = if last.is_empty() { author_trim.to_string() } else { last.to_string() };
        // Consent size for a later Sync & Open, fetched only on this path.
        let size_bytes = match &ctx.library {
            Some((src, book_id)) => super::sync::epub_size(src, *book_id).await.unwrap_or(-1),
            None => -1,
        };
        return Err(OpenOutcome::Missing {
            message: format!(
                "No match for: \"{title_trim}\" by {who} (predicted: {} not found → strict {candidate_count} books, 0 matched)\nCheck Calibre Kobo template or re-sync Kobo.",
                predicted.replace("/mnt/onboard/", "")
            ),
            predicted: predicted.to_string(),
            size_bytes,
        });
    }
    let preview_of = |list: &[&String]| {
        let mut p: Vec<String> = list
            .iter()
            .take(10)
            .map(|s| format!("  {}", s.replace("/mnt/onboard/", "")))
            .collect();
        if list.len() > 10 {
            p.push(format!("  ... +{} more", list.len() - 10));
        }
        p.join("\n")
    };
    if strict.is_empty() {
        return Err(OpenOutcome::Ambiguous {
            message: format!(
                "{} matches for \"{title_trim}\" (title-only, author {last} not found) — ambiguous, not opening:\n{}\nRefine title or check series prefix.",
                title_only.len(),
                preview_of(&title_only)
            ),
        });
    }
    Err(OpenOutcome::Ambiguous {
        message: format!(
            "{} matches for \"{title_trim}\" by {last} (strict) — ambiguous, not opening:\n{}",
            strict.len(),
            preview_of(&strict)
        ),
    })
}
