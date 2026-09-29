//! russh transport for Kobo: shell channel, combined password/key auth.
//!
//! Auth walks deliberate credentials first (explicit file, user-typed
//! password, the user's own ssh_config), then ambient (agent,
//! conventional defaults) — the way `ssh` itself resolves. No external
//! `ssh` binary is required; every caller (open, sync, probe, handoff)
//! rides this transport: shell channel (EXEC is swallowed by Dropbear),
//! `MARK:$?` trailer, deadline reads, trust-LAN host keys, `255` = auth
//! rejection, `124` = overrun, `-1` = transport failure. Passwords travel
//! only in memory and are never logged.
//!
//! Auth walks deliberate credentials first (explicit file, user-typed
//! password, the user's own ssh_config), then ambient (agent,
//! conventional defaults) — the way `ssh` itself resolves. Every request
//! counts against a budget so exotic setups stop with a clear 255
//! instead of tripping the server's MaxAuthTries disconnect.

use russh::client::{self, AuthResult};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{HashAlg, PrivateKey};
use russh::ChannelMsg;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::SshResult;

#[derive(Clone)]
pub enum SshAuth {
    Password(String),
    KeyFile(PathBuf),
}

pub fn default_key_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".ssh").join("id_ed25519")
}

/// Filename order for the conventional default identities: ed25519 first
/// (preserves the long-standing default), then RSA/ECDSA — the subset of
/// OpenSSH's default identity list that russh can load.
fn default_key_names() -> [&'static str; 3] {
    ["id_ed25519", "id_rsa", "id_ecdsa"]
}

/// Conventional default identity files that actually exist. Hosts with a
/// single key type try exactly one file; hosts with several fall back in
/// `default_key_names` order, mirroring OpenSSH's multi-identity behaviour.
pub fn default_key_files() -> Vec<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    default_key_names()
        .iter()
        .map(|n| PathBuf::from(&home).join(".ssh").join(n))
        .filter(|p| p.is_file())
        .collect()
}

/// Cap on auth requests per connection: OpenSSH servers disconnect after
/// MaxAuthTries (default 6). Stop with a clear 255 instead.
const MAX_AUTH_REQUESTS: u32 = 5;

/// Max agent identities offered per connection (each is a server attempt).
const MAX_AGENT_KEYS: usize = 4;

/// Well-known Win10+ OpenSSH agent pipe.
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Glob for ssh_config Host patterns: `*`, `?`, case-insensitive.
fn host_pat_matches(pat: &str, host: &str) -> bool {
    let (p, h) = (pat.as_bytes(), host.as_bytes());
    let (mut px, mut hx) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while hx < h.len() {
        if px < p.len()
            && (p[px] == b'?' || p[px].eq_ignore_ascii_case(&h[hx]))
        {
            px += 1;
            hx += 1;
        } else if px < p.len() && p[px] == b'*' {
            star = Some(px);
            px += 1;
            mark = hx;
        } else if let Some(s) = star {
            px = s + 1;
            mark += 1;
            hx = mark;
        } else {
            return false;
        }
    }
    while px < p.len() && p[px] == b'*' {
        px += 1;
    }
    px == p.len()
}

/// Minimal ssh_config reader: IdentityFile values from Host blocks
/// matching `host`. Values accumulate across blocks like OpenSSH;
/// options before the first Host line are global. `Match` blocks are
/// ignored, `!` negation is honored, keywords are case-insensitive,
/// `key=value` and quoted values are accepted.
pub fn config_identity_files(config: &str, host: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut applies = true; // global scope until the first Host/Match
    for raw in config.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut kv = line.splitn(2, |c: char| c == '=' || c.is_whitespace());
        let kw = kv.next().unwrap_or("").to_ascii_lowercase();
        let mut val = kv.next().unwrap_or("").trim().to_string();
        if let Some(s) = val.strip_prefix('=') {
            val = s.trim().to_string();
        }
        if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            val = val[1..val.len() - 1].to_string();
        }
        match kw.as_str() {
            "host" => {
                let mut pos = false;
                let mut neg = false;
                for pat in val.split_whitespace() {
                    if let Some(n) = pat.strip_prefix('!') {
                        if host_pat_matches(n, host) {
                            neg = true;
                        }
                    } else if host_pat_matches(pat, host) {
                        pos = true;
                    }
                }
                applies = pos && !neg;
            }
            "match" => applies = false,
            "identityfile" if applies && !val.is_empty() => out.push(val),
            _ => {}
        }
    }
    out
}

/// Expand a leading `~` against HOME/USERPROFILE. Anything else —
/// including `%`-tokens — passes through and simply fails to load later.
fn expand_ssh_path(s: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if let Some(rest) = s.strip_prefix("~/") {
        return PathBuf::from(home).join(rest);
    }
    if s == "~" {
        return PathBuf::from(home);
    }
    PathBuf::from(s)
}

fn ssh_config_text() -> Option<String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return None;
    }
    std::fs::read_to_string(PathBuf::from(home).join(".ssh").join("config")).ok()
}

fn load_key(path: &Path) -> Option<Arc<PrivateKey>> {
    russh::keys::load_secret_key(path, None).ok().map(Arc::new)
}

fn key_label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

type DynAgent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

#[cfg(unix)]
async fn connect_agent() -> Option<DynAgent> {
    AgentClient::connect_env().await.ok().map(|a| a.dynamic())
}

#[cfg(windows)]
async fn connect_agent() -> Option<DynAgent> {
    AgentClient::connect_named_pipe(WINDOWS_AGENT_PIPE)
        .await
        .ok()
        .map(|a| a.dynamic())
}

#[cfg(not(any(unix, windows)))]
async fn connect_agent() -> Option<DynAgent> {
    None
}

async fn try_key_file(
    handle: &mut client::Handle<TrustAll>,
    key: &Arc<PrivateKey>,
    ladder: &[Option<HashAlg>],
    budget: &mut u32,
) -> bool {
    for alg in ladder {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match handle
            .authenticate_publickey(
                "root",
                russh::keys::PrivateKeyWithHashAlg::new(Arc::clone(key), *alg),
            )
            .await
        {
            Ok(AuthResult::Success) => return true,
            _ => continue,
        }
    }
    false
}

async fn try_agent_identities(
    handle: &mut client::Handle<TrustAll>,
    ladder: &[Option<HashAlg>],
    budget: &mut u32,
    notes: &mut Vec<String>,
) -> bool {
    let Some(mut agent) = connect_agent().await else {
        return false;
    };
    let ids = match agent.request_identities().await {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut refused = 0u32;
    for pubkey in ids.iter().take(MAX_AGENT_KEYS) {
        let mut ok = false;
        for alg in ladder {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            match handle
                .authenticate_publickey_with("root", pubkey.clone(), *alg, &mut agent)
                .await
            {
                Ok(AuthResult::Success) => {
                    ok = true;
                    break;
                }
                _ => continue,
            }
        }
        if ok {
            return true;
        }
        refused += 1;
    }
    if refused > 0 {
        notes.push(format!("agent: {refused} refused"));
    }
    false
}

struct TrustAll;
impl client::Handler for TrustAll {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Trust-LAN policy, mirrors `StrictHostKeyChecking=accept-new`
        // for Kobo devices on the home network.
        Ok(true)
    }
}

/// Split a `MARK:$?` trailer: the shell echoes our own
/// `echo MARK:$?` line first (followed by `$?`, not digits) — only an
/// occurrence followed by an integer is the real trailer.
pub fn split_trailer(buf: &str, marker: &str) -> Option<(String, i32)> {
    let norm = buf.replace("\r\n", "\n").replace('\r', "\n");
    let tag = format!("{marker}:");
    let mut search = 0;
    while let Some(rel) = norm[search..].find(&tag) {
        let mi = search + rel;
        let tail = &norm[mi + tag.len()..];
        let digits: String =
            tail.chars().take_while(|c| *c == '-' || c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            if let Ok(code) = digits.parse::<i32>() {
                return Some((norm[..mi].trim().to_string(), code));
            }
        }
        search = mi + 1;
    }
    None
}

static MARK_CTR: AtomicU64 = AtomicU64::new(0);

/// Cumulative phase timings (ms) across `run_shell_blocks` calls: TCP+KEX
/// connect, credential auth walk, and the channel phase (writes + drain +
/// trailer wait). Reset per sync (`reset_phase_ms`); read back for the
/// shell's `[kobo]` log line (`phase_ms`). Diagnostics only.
static T_CONNECT_MS: AtomicU64 = AtomicU64::new(0);
static T_AUTH_MS: AtomicU64 = AtomicU64::new(0);
static T_XFER_MS: AtomicU64 = AtomicU64::new(0);

/// Zero the phase clocks (sync start).
pub fn reset_phase_ms() {
    T_CONNECT_MS.store(0, Ordering::Relaxed);
    T_AUTH_MS.store(0, Ordering::Relaxed);
    T_XFER_MS.store(0, Ordering::Relaxed);
}

/// `(connect, auth, xfer)` cumulative millis since the last reset.
pub fn phase_ms() -> (u64, u64, u64) {
    (
        T_CONNECT_MS.load(Ordering::Relaxed),
        T_AUTH_MS.load(Ordering::Relaxed),
        T_XFER_MS.load(Ordering::Relaxed),
    )
}

fn stamp(stat: &AtomicU64, since: std::time::Instant) {
    stat.fetch_add(u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX), Ordering::Relaxed);
}

/// Run pre-formed shell blocks, then the `MARK:$?` trailer, on one
/// shell channel. Blocks stream as plain text (base64 payloads ride
/// here — never raw binary on shared stdin). One invocation.
pub async fn run_shell_blocks(
    ip: &str,
    blocks: &[String],
    timeout_secs: u64,
    auth: SshAuth,
) -> SshResult {
    let fail = |output: String, code: i32| SshResult { output, code };
    let t0 = std::time::Instant::now();
    let addr: std::net::SocketAddr = match format!("{ip}:22").parse() {
        Ok(a) => a,
        Err(_) => return fail(format!("bad kobo ip: {ip}"), -1),
    };
    let config = Arc::new(client::Config::default());
    let mut handle = match tokio::time::timeout(
        Duration::from_secs(timeout_secs + 5),
        client::connect(config, addr, TrustAll),
    )
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return fail(format!("ssh connect failed: {e}"), -1),
        Err(_) => return fail("ssh connect timed out".to_string(), -1),
    };
    let t1 = std::time::Instant::now();
    stamp(&T_CONNECT_MS, t0);
    // Server-advertised RSA hash when available (OpenSSH sends
    // server-sig-algs; the Kobo does). Legacy ladder otherwise —
    // best_supported waits <=1s for EXT_INFO that never comes.
    let rsa_ladder: Vec<Option<HashAlg>> = match handle.best_supported_rsa_hash().await {
        Ok(Some(alg)) => vec![alg],
        _ => vec![Some(HashAlg::Sha512), None],
    };

    // Combined auth: deliberate credentials first (explicit file,
    // user-typed password, the user's own ssh_config), ambient after
    // (agent, conventional defaults).
    let mut budget: u32 = MAX_AUTH_REQUESTS;
    let mut notes: Vec<String> = Vec::new();
    let mut tried_paths: Vec<PathBuf> = Vec::new();
    let mut tried_names: Vec<String> = Vec::new();
    let mut authed = false;

    // Offer one file: dedup, load, spend budget. Missing ambient files
    // are normal (most hosts lack one of the three) and stay silent;
    // only offered keys appear in the failure report.
    async fn offer_file(
        handle: &mut client::Handle<TrustAll>,
        ladder: &[Option<HashAlg>],
        budget: &mut u32,
        tried_paths: &mut Vec<PathBuf>,
        tried_names: &mut Vec<String>,
        cand: &Path,
    ) -> bool {
        if tried_paths.iter().any(|t| t.as_path() == cand) {
            return false;
        }
        tried_paths.push(cand.to_path_buf());
        let Some(key) = load_key(cand) else {
            return false;
        };
        tried_names.push(key_label(cand));
        try_key_file(handle, &key, ladder, budget).await
    }

    // ssh_config IdentityFiles for this host (cheap, local).
    let cfg_files: Vec<PathBuf> = ssh_config_text()
        .map(|t| {
            config_identity_files(&t, ip)
                .into_iter()
                .map(|s| expand_ssh_path(&s))
                .collect()
        })
        .unwrap_or_default();

    // 1. explicit file (the KeyFile variant doubles as the long-standing
    // default when callers pass default_key_file()).
    if let SshAuth::KeyFile(p) = &auth {
        if load_key(p).is_none() {
            notes.push(format!("{} unreadable", key_label(p)));
        }
        if offer_file(
            &mut handle,
            &rsa_ladder,
            &mut budget,
            &mut tried_paths,
            &mut tried_names,
            p,
        )
        .await
        {
            authed = true;
        }
    }
    // 2. user-typed password (Settings row / KOBO_PASSWORD).
    if !authed {
        if let SshAuth::Password(pw) = &auth {
            if budget == 0 {
                notes.push("stopped: attempt budget spent".to_string());
            } else {
                budget -= 1;
                match handle.authenticate_password("root", pw).await {
                    Ok(AuthResult::Success) => authed = true,
                    _ => notes.push("password rejected".to_string()),
                }
            }
        }
    }
    // 3. the user's own ssh_config.
    if !authed {
        for cand in &cfg_files {
            if budget == 0 {
                notes.push("stopped: attempt budget spent".to_string());
                break;
            }
            if offer_file(
                &mut handle,
                &rsa_ladder,
                &mut budget,
                &mut tried_paths,
                &mut tried_names,
                cand,
            )
            .await
            {
                authed = true;
                break;
            }
        }
    }
    // 4. ssh-agent.
    if !authed
        && budget > 0
        && try_agent_identities(&mut handle, &rsa_ladder, &mut budget, &mut notes).await
    {
        authed = true;
    }
    // 5. conventional defaults.
    if !authed {
        for cand in &default_key_files() {
            if budget == 0 {
                notes.push("stopped: attempt budget spent".to_string());
                break;
            }
            if offer_file(
                &mut handle,
                &rsa_ladder,
                &mut budget,
                &mut tried_paths,
                &mut tried_names,
                cand,
            )
            .await
            {
                authed = true;
                break;
            }
        }
    }
    if !authed {
        if !tried_names.is_empty() {
            notes.push(format!("keys tried: {}", tried_names.join(", ")));
        }
        let detail = if notes.is_empty() {
            "no credentials offered".to_string()
        } else {
            notes.join("; ")
        };
        stamp(&T_AUTH_MS, t1);
        return fail(
            format!("SSH auth failed for {ip} ({detail}) — check Settings → Kobo."),
            255,
        );
    }
    stamp(&T_AUTH_MS, t1);
    let t3 = std::time::Instant::now();
    let channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            stamp(&T_XFER_MS, t3);
            return fail(format!("ssh channel failed: {e}"), -1);
        }
    };
    if channel.request_shell(true).await.is_err() {
        stamp(&T_XFER_MS, t3);
        return fail("ssh shell refused".to_string(), -1);
    }
    let marker = format!("KOBO_EXIT_{}_{}", std::process::id(), MARK_CTR.fetch_add(1, Ordering::Relaxed));
    // Drain-while-sending: the old loop wrote every block before reading
    // anything, while the shell echoes input back. ~1 MB of undrained echo
    // backpressured the upload to ~50 KB/s (pipe, heredoc+eMMC, and decode
    // each probed at 1+ MB/s against the same Kobo). Now the writer streams
    // on the write half while this loop drains the read half from the
    // first byte; one deadline covers both (writes were previously
    // unbounded).
    let (mut rh, wh) = channel.split();
    let mut payload: Vec<String> = blocks.to_vec();
    payload.push(format!("echo {marker}:$?\n"));
    let mut writer = tokio::spawn(async move {
        for block in &payload {
            let mut lined = block.clone();
            lined.push('\n');
            if wh.data(lined.as_bytes()).await.is_err() {
                return Err("ssh write failed".to_string());
            }
        }
        Ok::<(), String>(())
    });
    let mut writer_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs + 10);
    let mut buf = String::new();
    loop {
        if let Some((out, code)) = split_trailer(&buf, &marker) {
            stamp(&T_XFER_MS, t3);
            return fail(out, code);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            stamp(&T_XFER_MS, t3);
            return fail(format!("{}\n(timed out)", buf.trim()), 124);
        }
        tokio::select! {
            msg = rh.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        buf.push_str(&String::from_utf8_lossy(&data));
                    }
                    // Closed/EOF/exit without a trailer: keep polling to
                    // the deadline (mirrors the SSH.NET deadline loop).
                    _ => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
            res = &mut writer, if !writer_done => {
                match res {
                    Ok(Ok(())) => writer_done = true,
                    Ok(Err(e)) => {
                        stamp(&T_XFER_MS, t3);
                        return fail(e, -1);
                    }
                    Err(_) => {
                        stamp(&T_XFER_MS, t3);
                        return fail("ssh write failed".to_string(), -1);
                    }
                }
            }
            _ = tokio::time::sleep(remaining) => {
                stamp(&T_XFER_MS, t3);
                return fail(format!("{}\n(timed out)", buf.trim()), 124);
            }
        }
    }
}

pub async fn ssh_sync_russh(
    ip: &str,
    remote_cmd: &str,
    timeout_secs: u64,
    auth: SshAuth,
) -> SshResult {
    run_shell_blocks(ip, &[remote_cmd.to_string()], timeout_secs, auth).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailer_skips_echo_finds_digits() {
        let buf = "echo ok\necho KOBO_EXIT_1:$?\nok\nKOBO_EXIT_1:0\n";
        assert_eq!(
            split_trailer(buf, "KOBO_EXIT_1"),
            Some(("echo ok\necho KOBO_EXIT_1:$?\nok".to_string(), 0))
        );
    }

    #[test]
    fn trailer_negative_code() {
        assert_eq!(
            split_trailer("oops\nKOBO_EXIT_9:-1\n", "KOBO_EXIT_9"),
            Some(("oops".to_string(), -1))
        );
    }

    #[test]
    fn trailer_missing_is_none() {
        assert_eq!(split_trailer("echo ok\n", "KOBO_EXIT_1"), None);
        assert_eq!(split_trailer("echo KOBO_EXIT_1:$?\n", "KOBO_EXIT_1"), None);
    }

    #[test]
    fn trailer_crlf_normalized() {
        assert_eq!(
            split_trailer("ok\r\nKOBO_EXIT_2:7\r\n", "KOBO_EXIT_2"),
            Some(("ok".to_string(), 7))
        );
    }

    #[test]
    fn default_key_order_prefers_ed25519() {
        assert_eq!(default_key_names(), ["id_ed25519", "id_rsa", "id_ecdsa"]);
    }

    #[test]
    fn host_pat_exact_and_case() {
        assert!(host_pat_matches("192.168.1.75", "192.168.1.75"));
        assert!(!host_pat_matches("192.168.1.74", "192.168.1.75"));
        assert!(host_pat_matches("KOBO", "kobo"));
    }

    #[test]
    fn host_pat_wildcards() {
        assert!(host_pat_matches("*", "anything"));
        assert!(host_pat_matches("192.168.1.*", "192.168.1.75"));
        assert!(!host_pat_matches("192.168.2.*", "192.168.1.75"));
        assert!(host_pat_matches("kobo?", "kobo2"));
        assert!(!host_pat_matches("kobo?", "kobo22"));
        assert!(host_pat_matches("a*b*c", "axbyc"));
    }

    #[test]
    fn config_global_and_host() {
        let cfg = "IdentityFile ~/.ssh/global_rsa\nHost kobo\n  IdentityFile ~/.ssh/kobo_ed\n";
        assert_eq!(
            config_identity_files(cfg, "kobo"),
            vec!["~/.ssh/global_rsa".to_string(), "~/.ssh/kobo_ed".to_string()]
        );
        assert_eq!(
            config_identity_files(cfg, "other"),
            vec!["~/.ssh/global_rsa".to_string()]
        );
    }

    #[test]
    fn config_wildcard_negation_match_ignored() {
        let cfg = "Host *.lan\n  IdentityFile ~/.ssh/lan_key\nHost bad\n  IdentityFile ~/.ssh/nope\nHost !blocked.lan *.lan\n  IdentityFile ~/.ssh/star\nMatch host kobo\n  IdentityFile ~/.ssh/match_key\n";
        assert!(config_identity_files(cfg, "kobo.lan").contains(&"~/.ssh/lan_key".to_string()));
        assert!(config_identity_files(cfg, "kobo.lan").contains(&"~/.ssh/star".to_string()));
        assert!(!config_identity_files(cfg, "kobo.lan").contains(&"~/.ssh/nope".to_string()));
        assert!(!config_identity_files(cfg, "kobo.lan").contains(&"~/.ssh/match_key".to_string()));
        assert!(!config_identity_files(cfg, "blocked.lan").contains(&"~/.ssh/star".to_string()));
    }

    #[test]
    fn config_equals_quoted_forms() {
        let cfg = "Host kobo\n  IdentityFile=~/.ssh/eq_rsa\n  identityfile \"~/.ssh/quoted\"\n";
        assert_eq!(
            config_identity_files(cfg, "kobo"),
            vec!["~/.ssh/eq_rsa".to_string(), "~/.ssh/quoted".to_string()]
        );
    }

    #[test]
    fn expand_tilde() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        assert_eq!(
            expand_ssh_path("~/.ssh/id_rsa"),
            PathBuf::from(&home).join(".ssh").join("id_rsa")
        );
        assert_eq!(expand_ssh_path("/abs/key"), PathBuf::from("/abs/key"));
    }
}
