//! russh transport for Kobo (Dropbear): shell channel, password/key auth.
//!
//! Mirrors the SSH.NET `SshSync` semantics in `KoboLauncher.cs`, which the
//! `ssh`-binary `ssh_sync` in `kobo.rs` cannot match (it is `BatchMode`,
//! key-only — per-IP passwords have no transport there):
//! shell channel (EXEC is swallowed by Dropbear), `MARK:$?` trailer,
//! deadline reads, trust-LAN host keys, `255` = auth rejection,
//! `124` = overrun, `-1` = transport failure. Passwords travel only in
//! memory and are never logged.

use russh::client::{self, AuthResult};
use russh::ChannelMsg;
use std::path::PathBuf;
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
    let auth_result = match auth {
        SshAuth::Password(pw) => handle.authenticate_password("root", pw).await,
        SshAuth::KeyFile(path) => {
            let key = match russh::keys::load_secret_key(&path, None) {
                Ok(k) => k,
                Err(e) => {
                    return fail(
                        format!("ssh key not found: {} ({e})", path.display()),
                        -1,
                    )
                }
            };
            handle
                .authenticate_publickey(
                    "root",
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None),
                )
                .await
        }
    };
    match auth_result {
        Ok(AuthResult::Success) => {}
        _ => {
            return fail(
                format!("SSH password rejected for {ip} — check Settings → Kobo."),
                255,
            )
        }
    }
    let mut channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => return fail(format!("ssh channel failed: {e}"), -1),
    };
    if channel.request_shell(true).await.is_err() {
        return fail("ssh shell refused".to_string(), -1);
    }
    let marker = format!("KOBO_EXIT_{}_{}", std::process::id(), MARK_CTR.fetch_add(1, Ordering::Relaxed));
    for block in blocks {
        let mut lined = block.clone();
        lined.push('\n');
        if channel.data(lined.as_bytes()).await.is_err() {
            return fail("ssh write failed".to_string(), -1);
        }
    }
    let trailer = format!("echo {marker}:$?\n");
    if channel.data(trailer.as_bytes()).await.is_err() {
        return fail("ssh write failed".to_string(), -1);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs + 10);
    let mut buf = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return fail(format!("{}\n(timed out)", buf.trim()), 124);
        }
        match tokio::time::timeout(remaining, channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                buf.push_str(&String::from_utf8_lossy(&data));
                if let Some((out, code)) = split_trailer(&buf, &marker) {
                    return fail(out, code);
                }
            }
            // Closed/EOF/exit without a trailer: keep polling to the
            // deadline (mirrors the SSH.NET deadline loop), then 124.
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(_) => return fail(format!("{}\n(timed out)", buf.trim()), 124),
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
}
