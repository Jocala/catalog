//! Port of `CatalogCore/Services/SmbService.swift`.
//!
//! Swift uses a vendored SMB2/3 client (NTLM, Session, TreeConnect,
//! Create/Read/QueryDirectory) over Network.framework. The Rust port keeps
//! the exact retry/auth-classify behaviour and the public op surface
//! (test/connect/listShares/listDirectory/download/downloadData/recursive)
//! behind a pluggable [`SmbBackend`] trait, so a real SMB2 transport crate
//! can be dropped in without touching callers. No credentials are
//! hardcoded — they come from `jreader_settings::Settings` at runtime.

use std::time::Duration;
use std::collections::HashMap;
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
}

#[derive(Debug, Error)]
pub enum SmbError {
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("network: {0}")]
    Network(String),
    #[error("sharing violation (Calibre likely holds the file open): {0}")]
    SharingViolation(String),
    #[error("backend not configured: {0}")]
    NotConfigured(String),
    #[error("io: {0}")]
    Io(String),
}

// Mirrors SmbService.retryablePOSIX = {50,51,60,64,65} (ENETDOWN,
// ENETUNREACH, ETIMEDOUT, EHOSTDOWN, EHOSTUNREACH).
pub fn is_retryable_errno(code: i32) -> bool {
    matches!(code, 50 | 51 | 60 | 64 | 65)
}

/// Mirrors `SmbService.isTransientNetworkError`: POSIX codes plus the
/// textual NWError matches ("timed out", "broken pipe", "network is
/// down", ...), since the Swift SMBClient wraps POSIX as generic Error.
pub fn is_transient_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("timed out")
        || m.contains("timeout")
        || m.contains("broken pipe")
        || m.contains("connection reset")
        || m.contains("connection refused")
        || m.contains("network is down")
        || m.contains("network is unreachable")
        || m.contains("no route to host")
        || m.contains("host is down")
        || m.contains("host is unreachable")
}

/// Mirrors `SmbService.isAuthFailure` (logon/logon_failure/access_denied/auth).
pub fn is_auth_failure_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("logon") || m.contains("logon_failure") || m.contains("access_denied") || m.contains("auth")
}

/// Classify a backend error string the way `classifySMBError` does in
/// `SmbCatalogDB.swift` (sharing → network-with-hint, auth, not-found).
pub fn classify_error(msg: &str) -> SmbError {
    let m = msg.to_ascii_lowercase();
    if m.contains("sharing") {
        return SmbError::SharingViolation(msg.to_string());
    }
    if is_auth_failure_message(&m) {
        return SmbError::Auth(msg.to_string());
    }
    if m.contains("not found")
        || m.contains("no such")
        || m.contains("object_name")
        || m.contains("object name")
        || m.contains("object_path")
        || m.contains("bad_netpath")
        || m.contains("not_found")
        || m.contains("status_")
    {
        return SmbError::NotFound(msg.to_string());
    }
    SmbError::Network(msg.to_string())
}

/// Minimal async SMB transport surface. Implement with a real SMB2 crate
/// (login/connectShare/listDirectory/ranged-read) or with `smbclient`.
#[async_trait::async_trait]
pub trait SmbBackend: Send + Sync {
    async fn login(&mut self, user: Option<&str>, password: Option<&str>, domain: Option<&str>) -> Result<(), String>;
    async fn connect_share(&mut self, share: &str) -> Result<(), String>;
    async fn list_shares(&mut self) -> Result<Vec<String>, String>;
    async fn list_directory(&mut self, path: &str) -> Result<Vec<SmbEntry>, String>;
    /// Share-tolerant read: open read-only with share read+write+delete
    /// (mirrors `downloadShared`), retrying sharing violations.
    async fn read_file_shared(&mut self, path: &str) -> Result<Vec<u8>, String>;
    async fn logoff(&mut self) -> Result<(), String>;
}

pub struct SmbClient<B> {
    backend: B,
}

impl<B: SmbBackend> SmbClient<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Release the backend (session pooling extracts connected backends).
    pub fn into_backend(self) -> B {
        self.backend
    }

    async fn login_params(
        &mut self,
        user: &str,
        password: &str,
        domain: &str,
    ) -> Result<(), SmbError> {
        let u = if user.is_empty() { None } else { Some(user) };
        let p = if password.is_empty() { None } else { Some(password) };
        let d = if domain.is_empty() { None } else { Some(domain) };
        self.backend.login(u, p, d).await.map_err(|m| classify_error(&m))
    }

    /// Mirrors `withFreshClient` retry rule: one retry on transient
    /// network errors (800 ms beat), plus for `op == "test"` a retry on
    /// any non-auth error (first-SYN-after-idle stall fix).
    fn should_retry(op: &str, msg: &str) -> bool {
        let auth = is_auth_failure_message(msg);
        is_transient_message(msg) || (op == "test" && !auth)
    }

    async fn connect_share_retry(&mut self, op: &str, share: &str) -> Result<(), SmbError> {
        match self.backend.connect_share(share).await {
            Ok(()) => Ok(()),
            Err(m) => {
                if !Self::should_retry(op, &m) {
                    return Err(classify_error(&m));
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                self.backend.connect_share(share).await.map_err(|m2| classify_error(&m2))
            }
        }
    }

    pub async fn test_connection(
        &mut self,
        user: &str,
        password: &str,
        domain: &str,
        share: &str,
    ) -> Result<(), SmbError> {
        self.login_params(user, password, domain).await?;
        self.connect_share_retry("test", share).await?;
        let _ = self.backend.logoff().await;
        Ok(())
    }

    pub async fn list_shares(
        &mut self,
        user: &str,
        password: &str,
        domain: &str,
    ) -> Result<Vec<String>, SmbError> {
        self.login_params(user, password, domain).await?;
        let shares = match self.backend.list_shares().await {
            Ok(s) => s,
            Err(m) => {
                if !Self::should_retry("shares", &m) {
                    return Err(classify_error(&m));
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                self.backend.list_shares().await.map_err(|m2| classify_error(&m2))?
            }
        };
        let mut out: Vec<String> = shares.into_iter().filter(|s| !s.ends_with('$')).collect();
        out.sort();
        let _ = self.backend.logoff().await;
        Ok(out)
    }

    pub async fn list_directory(
        &mut self,
        user: &str,
        password: &str,
        domain: &str,
        share: &str,
        path: &str,
    ) -> Result<Vec<SmbEntry>, SmbError> {
        self.login_params(user, password, domain).await?;
        self.connect_share_retry("list", share).await?;
        let files = match self.backend.list_directory(path).await {
            Ok(f) => f,
            Err(m) => {
                if !Self::should_retry("list", &m) {
                    let _ = self.backend.logoff().await;
                    return Err(classify_error(&m));
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                let f = self.backend.list_directory(path).await.map_err(|m2| classify_error(&m2))?;
                f
            }
        };
        let _ = self.backend.logoff().await;
        Ok(files
            .into_iter()
            .filter(|e| e.name != "." && e.name != "..")
            .collect())
    }

    /// Memory-only fetch of a remote file (mirrors `downloadData`):
    /// share-tolerant open + up to 3 attempts on sharing violation.
    pub async fn download_data(
        &mut self,
        user: &str,
        password: &str,
        domain: &str,
        share: &str,
        remote_path: &str,
    ) -> Result<Vec<u8>, SmbError> {
        self.login_params(user, password, domain).await?;
        self.connect_share_retry("download", share).await?;
        let mut last = String::new();
        for _ in 1..=3 {
            match self.backend.read_file_shared(remote_path).await {
                Ok(d) if !d.is_empty() => {
                    let _ = self.backend.logoff().await;
                    return Ok(d);
                }
                Ok(_) => last = "empty file".to_string(),
                Err(m) if m.to_ascii_lowercase().contains("sharing") => {
                    last = m;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(m) => {
                    let _ = self.backend.logoff().await;
                    return Err(classify_error(&m));
                }
            }
        }
        let _ = self.backend.logoff().await;
        Err(SmbError::NotFound(if last.is_empty() { "empty file".into() } else { last }))
    }
}

/// Credentials + server for one SMB session. All fields come from
/// Settings or CLI flags — never hardcoded.
#[derive(Debug, Clone, Default)]
pub struct SmbConn {
    pub host: String,
    pub user: String,
    pub password: String,
    pub domain: String,
}

impl SmbConn {
    /// `DOMAIN\user` form NTLM (via sspi) expects; bare user when no domain.
    /// Empty user + empty password means guest (Swift anonymous parity).
    fn login_name(&self) -> String {
        if self.domain.trim().is_empty() {
            self.user.clone()
        } else {
            format!("{}\\{}", self.domain.trim(), self.user)
        }
    }
}

fn smb_err(e: smb::Error) -> String {
    // Debug carries NTSTATUS/enum-variant names (SharingViolation,
    // LogonFailure, ObjectNameNotFound…) that classify_error matches on.
    format!("{e:?}: {e}")
}

/// [`SmbBackend`] over the pure-Rust `smb` crate (NTLM via sspi, signing
/// and encryption negotiated by default). One backend per operation set,
/// mirroring Swift's fresh-client-per-op rule.
pub struct SmbCrateBackend {
    conn: SmbConn,
    client: Option<smb::Client>,
    share: Option<String>,
}

impl SmbCrateBackend {
    pub fn new(conn: SmbConn) -> Self {
        Self { conn, client: None, share: None }
    }

    fn unc(&self, share: &str, path: &str) -> Result<smb::UncPath, String> {
        use std::str::FromStr;
        let rel = path.replace('/', "\\");
        let rel = rel.trim_matches('\\');
        let s = if rel.is_empty() {
            format!("\\\\{}\\{}", self.conn.host, share)
        } else {
            format!("\\\\{}\\{}\\{}", self.conn.host, share, rel)
        };
        smb::UncPath::from_str(&s).map_err(|e| format!("bad UNC {s}: {e}"))
    }

    async fn ensure_client(&mut self) -> Result<&smb::Client, String> {
        if self.client.is_none() {
            self.client = Some(smb::Client::new(smb::ClientConfig::default()));
        }
        Ok(self.client.as_ref().expect("just set"))
    }

    async fn tree_share(&mut self) -> Result<String, String> {
        self.share.clone().ok_or_else(|| "not connected: connect_share first".to_string())
    }

    /// Chunked read with per-chunk progress (chunks done/total mirrored
    /// so an outer timeout can tell "wedged before the first byte" from
    /// "crawling mid-stream"). Returns the bytes plus chunks completed.
    /// Each chunk carries its own 10s bound (a healthy chunk takes
    /// ~60ms); a stalled chunk fails the attempt instead of burning the
    /// whole outer fetch budget.
    async fn read_file_shared_progress(
        &mut self,
        path: &str,
        progress: Option<&std::sync::Mutex<SmbFetchDiag>>,
    ) -> (Result<Vec<u8>, String>, u64) {
        use smb::{GetLen, ReadAt};
        let share = match self.tree_share().await {
            Ok(s) => s,
            Err(e) => return (Err(e), 0),
        };
        let unc = match self.unc(&share, path) {
            Ok(u) => u,
            Err(e) => return (Err(e), 0),
        };
        let client = match self.ensure_client().await {
            Ok(c) => c,
            Err(e) => return (Err(e), 0),
        };
        let args = smb::FileCreateArgs::make_open_existing(
            smb::FileAccessMask::new().with_generic_read(true),
        );
        let file = match client.create_file(&unc, &args).await.map_err(smb_err) {
            Ok(smb::Resource::File(f)) => f,
            Ok(_) => return (Err("not a file".to_string()), 0),
            Err(e) => return (Err(e), 0),
        };
        let len = match file.get_len().await.map_err(smb_err) {
            Ok(l) => l,
            Err(e) => return (Err(e), 0),
        };
        const CHUNK: u64 = 1 << 20;
        const CHUNK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        let total = len.div_ceil(CHUNK);
        let mut out = Vec::with_capacity(len.min(256 << 20) as usize);
        let mut off = 0u64;
        let mut done = 0u64;
        while off < len {
            let n = CHUNK.min(len - off) as usize;
            let mut buf = vec![0u8; n];
            let read = match tokio::time::timeout(CHUNK_TIMEOUT, file.read_at(&mut buf, off)).await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => return (Err(smb_err(e)), done),
                Err(_) => {
                    return (
                        Err(format!(
                            "chunk read timed out after 10s (chunk {}/{}, {} bytes)",
                            done + 1,
                            total,
                            len
                        )),
                        done,
                    )
                }
            };
            if read == 0 {
                break;
            }
            buf.truncate(read);
            out.extend_from_slice(&buf);
            off += read as u64;
            done += 1;
            note(progress, |d| {
                d.chunks_done = done;
                d.chunks_total = total;
            });
        }
        if let Err(e) = file.close().await.map_err(smb_err) {
            return (Err(e), done);
        }
        note(progress, |d| {
            d.chunks_done = done;
            d.chunks_total = total;
        });
        (Ok(out), done)
    }
}

#[async_trait::async_trait]
impl SmbBackend for SmbCrateBackend {
    async fn login(
        &mut self,
        user: Option<&str>,
        password: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), String> {
        // Store creds; auth is validated at connect_share (the smb crate
        // authenticates during share/session setup, like Swift's login).
        // Fresh client per login set, mirroring withFreshClient.
        self.conn.user = user.unwrap_or("").to_string();
        self.conn.password = password.unwrap_or("").to_string();
        self.conn.domain = domain.unwrap_or("").to_string();
        self.client = Some(smb::Client::new(smb::ClientConfig::default()));
        self.share = None;
        Ok(())
    }

    async fn connect_share(&mut self, share: &str) -> Result<(), String> {
        let unc = self.unc(share, "")?;
        let (user, pass) = (self.conn.login_name(), self.conn.password.clone());
        self.ensure_client()
            .await?
            .share_connect(&unc, &user, pass)
            .await
            .map_err(smb_err)?;
        self.share = Some(share.to_string());
        Ok(())
    }

    async fn list_shares(&mut self) -> Result<Vec<String>, String> {
        let host = self.conn.host.clone();
        let out = self
            .ensure_client()
            .await?
            .list_shares(&host)
            .await
            .map_err(smb_err)?;
        // NdrPtr derefs to Option<NdrAlign<NdrString>> (Display).
        Ok(out
            .iter()
            .map(|s| {
                s.netname
                    .as_ref()
                    .map(|a| a.value.to_string())
                    .unwrap_or_default()
            })
            .collect())
    }

    async fn list_directory(&mut self, path: &str) -> Result<Vec<SmbEntry>, String> {
        let share = self.tree_share().await?;
        let unc = self.unc(&share, path)?;
        let client = self.ensure_client().await?;
        let args = smb::FileCreateArgs::make_open_existing(
            smb::FileAccessMask::new().with_generic_read(true),
        );
        let dir = match client.create_file(&unc, &args).await.map_err(smb_err)? {
            smb::Resource::Directory(d) => std::sync::Arc::new(d),
            smb::Resource::File(_) => return Err("not a directory".to_string()),
            smb::Resource::Pipe(_) => return Err("not a directory".to_string()),
        };
        use futures_util::StreamExt;
        let mut stream = smb::Directory::query::<smb::FileDirectoryInformation>(&dir, "*")
            .await
            .map_err(smb_err)?;
        let mut out = vec![];
        while let Some(item) = stream.next().await {
            let info = item.map_err(smb_err)?;
            out.push(SmbEntry {
                name: info.file_name.to_string(),
                is_directory: info.file_attributes.directory(),
                size: info.end_of_file,
            });
        }
        Ok(out)
    }

    /// Share-tolerant read: generic-read open (the crate's default share
    /// mode is full sharing, unlike the old vendored FileReader which
    /// opened share-read-only and hit violations on Calibre-locked files).
    /// Chunked 1 MiB ranged reads; the wrapper retries violations.
    /// Each chunk carries its own 10s bound (same philosophy as the
    /// crate's per-leg timeout; a healthy chunk takes ~60ms): a stalled
    /// chunk fails the attempt at ~10s instead of burning the whole
    /// outer fetch budget, and the message keeps the "timed out" marker
    /// so the fetch-level fresh retry still triggers.
    async fn read_file_shared(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.read_file_shared_progress(path, None).await.0
    }

    async fn logoff(&mut self) -> Result<(), String> {
        if let Some(client) = self.client.take() {
            client.close().await.map_err(smb_err)?;
        }
        self.share = None;
        Ok(())
    }
}

/// Download raw bytes (e.g. metadata.db) with retry. Thin async helper so
/// CLI and UI crates never touch backend types.
pub async fn download_db_bytes(
    conn: SmbConn,
    share: &str,
    remote_path: &str,
) -> Result<Vec<u8>, SmbError> {
    download_db_bytes_with_diag(conn, share, remote_path)
        .await
        .0
}

/// Metadata fetch with per-stage timing. Clocks are cheap `Instant`s;
/// the caller decides whether to surface the diag (gated by `diag`).
pub async fn download_db_bytes_with_diag(
    conn: SmbConn,
    share: &str,
    remote_path: &str,
) -> (Result<Vec<u8>, SmbError>, SmbFetchDiag) {
    let progress = std::sync::Mutex::new(SmbFetchDiag::default());
    download_db_bytes_with_progress(conn, share, remote_path, &progress).await
}

/// Same fetch, but stage progress is mirrored into `progress` as it
/// happens — so a caller that times the future out still learns whether
/// the stall was in session acquisition or the read.
pub async fn download_db_bytes_with_progress(
    conn: SmbConn,
    share: &str,
    remote_path: &str,
    progress: &std::sync::Mutex<SmbFetchDiag>,
) -> (Result<Vec<u8>, SmbError>, SmbFetchDiag) {
    pooled_download_diag(&conn, share, remote_path, Some(progress)).await
}

/// Publish a stage update to the caller's progress handle (if any).
fn note(
    progress: Option<&std::sync::Mutex<SmbFetchDiag>>,
    f: impl FnOnce(&mut SmbFetchDiag),
) {
    if let Some(m) = progress {
        if let Ok(mut g) = m.lock() {
            f(&mut g);
        }
    }
}

/// Generic remote-file download with retry (covers, epubs, …).
/// Sessions are pooled: repeat reads reuse the authenticated share
/// session instead of paying TCP + NTLM per file (the gallery-cover lag
/// vs OS-pooled transports). Same retry/auth semantics as `download_data`.
pub async fn download_file_bytes(
    conn: SmbConn,
    share: &str,
    remote_path: &str,
) -> Result<Vec<u8>, SmbError> {
    pooled_download(&conn, share, remote_path).await
}

/// List a share directory with retry (pooled session, same as reads).
pub async fn list_dir(
    conn: SmbConn,
    share: &str,
    path: &str,
) -> Result<Vec<SmbEntry>, SmbError> {
    pooled_list(&conn, share, path).await
}

/// Login + share connect with retry (powers Test SMB for real).
/// Deliberately unpooled: a connectivity test must not reuse a session.
pub async fn test_share(conn: SmbConn, share: &str) -> Result<(), SmbError> {
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .test_connection(&conn.user, &conn.password, &conn.domain, share)
        .await
}

// ---------------------------------------------------------------------------
// Session pool. Authenticated share sessions are reused across calls so
// repeat reads (gallery covers, epubs, db refreshes) skip TCP + NTLM
// setup. Keyed by creds+share (passwords in memory only, same precedent
// as the FFI db cache). Idle entries evict after POOL_IDLE_SECS; at most
// POOL_MAX_IDLE idle sessions per key. A failed session is dropped, never
// retried in place — the retry always reconnects fresh (withFreshClient).
// ---------------------------------------------------------------------------

const POOL_IDLE_SECS: u64 = 90;
const POOL_MAX_IDLE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PoolKey {
    host: String,
    share: String,
    user: String,
    domain: String,
    password: String,
}

impl PoolKey {
    fn of(conn: &SmbConn, share: &str) -> Self {
        Self {
            host: conn.host.clone(),
            share: share.to_string(),
            user: conn.user.clone(),
            domain: conn.domain.clone(),
            password: conn.password.clone(),
        }
    }
}

struct PooledSession {
    backend: SmbCrateBackend,
    last_used: std::time::Instant,
}

static POOL: OnceLock<tokio::sync::Mutex<HashMap<PoolKey, Vec<PooledSession>>>> =
    OnceLock::new();

fn pool() -> &'static tokio::sync::Mutex<HashMap<PoolKey, Vec<PooledSession>>> {
    POOL.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Pure eviction rule (unit-tested): keep fresh entries up to the cap.
fn keep_entry(last_used: std::time::Instant, now: std::time::Instant, kept: usize) -> bool {
    now.duration_since(last_used).as_secs() < POOL_IDLE_SECS && kept < POOL_MAX_IDLE
}

/// Per-fetch SMB timing (no secrets: booleans + millis + byte count only).
/// Built on every metadata fetch; only surfaced to the shell when the
/// `diag` config flag is set (Settings → Diagnostic logging).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SmbFetchDiag {
    pub pooled_hit: Option<bool>,
    pub connect_ms: u64,
    pub read_ms: u64,
    pub bytes: u64,
    /// Stale sessions dropped at checkout (never awaited — see below).
    pub evict_n: u32,
    /// Chunk progress of the read (1 MiB chunks): completed / total.
    pub chunks_done: u64,
    pub chunks_total: u64,
}

/// Checkout outcome: backend + pool-hit flag + stale sessions dropped.
pub struct Checkout {
    pub backend: SmbCrateBackend,
    pub pooled_hit: bool,
    pub evicted: u32,
}

async fn checkout(key: &PoolKey, conn: &SmbConn, share: &str) -> Result<Checkout, SmbError> {
    let now = std::time::Instant::now();
    let mut evicted: Vec<PooledSession> = Vec::new();
    let hit = {
        let mut guard = pool().lock().await;
        let mut hit: Option<PooledSession> = None;
        if let Some(entries) = guard.get_mut(key) {
            let mut kept: Vec<PooledSession> = Vec::new();
            // Newest first: the hottest TCP session wins.
            for e in entries.drain(..).rev() {
                if hit.is_none() && keep_entry(e.last_used, now, kept.len()) {
                    hit = Some(e);
                } else if keep_entry(e.last_used, now, kept.len()) {
                    kept.push(e);
                } else {
                    evicted.push(e);
                }
            }
            *entries = kept;
        }
        hit
    };
    // Stale sessions are DROPPED, never gracefully logged off: each
    // logoff leg (tree disconnect, session logoff, connection close)
    // waits out the crate's ~10s read timeout on a dead TCP, so saying
    // goodbye to N stale sessions could burn tens of seconds of the
    // fetch budget before the real work starts (the 30s-stall shape).
    // Socket close still sends FIN, so the server reaps promptly; this
    // matches the over-cap checkin path, which already drops. Count the
    // evictions for the fetch diag.
    let evicted_n = evicted.len() as u32;
    drop(evicted);
    if let Some(e) = hit {
        return Ok(Checkout { backend: e.backend, pooled_hit: true, evicted: evicted_n });
    }
    // Miss: fresh login + share connect (SmbClient retry rules intact).
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .login_params(&conn.user, &conn.password, &conn.domain)
        .await?;
    client.connect_share_retry("pooled", share).await?;
    Ok(Checkout { backend: client.into_backend(), pooled_hit: false, evicted: evicted_n })
}

async fn checkin(key: PoolKey, backend: SmbCrateBackend) {
    let mut guard = pool().lock().await;
    let entries = guard.entry(key).or_default();
    if entries.len() < POOL_MAX_IDLE {
        entries.push(PooledSession { backend, last_used: std::time::Instant::now() });
    }
    // Over cap: drop (TCP closes); the pool stays bounded.
}

async fn pooled_download(
    conn: &SmbConn,
    share: &str,
    remote_path: &str,
) -> Result<Vec<u8>, SmbError> {
    pooled_download_diag(conn, share, remote_path, None).await.0
}

async fn pooled_download_diag(
    conn: &SmbConn,
    share: &str,
    remote_path: &str,
    progress: Option<&std::sync::Mutex<SmbFetchDiag>>,
) -> (Result<Vec<u8>, SmbError>, SmbFetchDiag) {
    let key = PoolKey::of(conn, share);
    // Session acquisition (pooled reuse or fresh TCP + NTLM): its own
    // clock so a dead pooled session shows as connect_ms≈timeout rather
    // than an opaque stall.
    let t_connect = std::time::Instant::now();
    let co = match checkout(&key, conn, share).await {
        Ok(co) => co,
        Err(e) => return (Err(e), SmbFetchDiag::default()),
    };
    let mut backend = co.backend;
    let mut diag = SmbFetchDiag {
        pooled_hit: Some(co.pooled_hit),
        connect_ms: t_connect.elapsed().as_millis() as u64,
        evict_n: co.evicted,
        ..Default::default()
    };
    // Mirror acquisition progress so an outer timeout still attributes
    // the stall (a read-stall snapshot keeps connect_ms + pooled_hit).
    note(progress, |d| {
        d.pooled_hit = diag.pooled_hit;
        d.connect_ms = diag.connect_ms;
        d.evict_n = diag.evict_n;
    });
    // First attempt: pooled or fresh session, sharing-violation loop intact.
    let t_read = std::time::Instant::now();
    let mut last = String::new();
    for _ in 1..=3 {
        let (res, _chunks) = backend.read_file_shared_progress(remote_path, progress).await;
        match res {
            Ok(d) if !d.is_empty() => {
                diag.read_ms = t_read.elapsed().as_millis() as u64;
                diag.bytes = d.len() as u64;
                checkin(key, backend).await;
                let snapshot = diag.clone();
                note(progress, |d| {
                    d.read_ms = snapshot.read_ms;
                    d.bytes = snapshot.bytes;
                });
                return (Ok(d), diag);
            }
            Ok(_) => {
                last = "empty file".to_string();
                break;
            }
            Err(m) if m.to_ascii_lowercase().contains("sharing") => {
                last = m;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(m) => {
                last = m;
                break;
            }
        }
    }
    diag.read_ms = t_read.elapsed().as_millis() as u64;
    note(progress, |d| {
        d.read_ms = diag.read_ms;
        d.bytes = diag.bytes;
    });
    // Transient (non-auth) failure: retry once on a FRESH session, then
    // give up. Auth failures never retry (no poison, nothing cached).
    // Empty file is a content answer, not a transport failure.
    if last == "empty file" {
        return (Err(SmbError::NotFound(last)), diag);
    }
    if SmbClient::<SmbCrateBackend>::should_retry("download", &last) {
        let t2 = std::time::Instant::now();
        match checkout_fresh(conn, share).await {
            Ok(mut fresh) => {
                diag.pooled_hit = Some(false);
                diag.connect_ms = t2.elapsed().as_millis() as u64;
                note(progress, |d| {
                    d.pooled_hit = diag.pooled_hit;
                    d.connect_ms = diag.connect_ms;
                });
                let t3 = std::time::Instant::now();
                let (res, _chunks) =
                    fresh.read_file_shared_progress(remote_path, progress).await;
                match res {
                    Ok(d) if !d.is_empty() => {
                        diag.read_ms = t3.elapsed().as_millis() as u64;
                        diag.bytes = d.len() as u64;
                        checkin(key, fresh).await;
                        let snapshot = diag.clone();
                        note(progress, |d| {
                            d.read_ms = snapshot.read_ms;
                            d.bytes = snapshot.bytes;
                        });
                        return (Ok(d), diag);
                    }
                    Ok(_) => last = "empty file".to_string(),
                    Err(m) => last = m,
                }
                diag.read_ms = t3.elapsed().as_millis() as u64;
                note(progress, |d| d.read_ms = diag.read_ms);
            }
            Err(e) => {
                // Fresh connect failed: keep the original read error as
                // the outcome (matches pre-diag behavior) — the fresh
                // error is transport noise, not the root cause.
                let _ = e;
            }
        }
    }
    (Err(classify_error(&last)), diag)
}

/// Connect bypassing the pool (fresh-session retry path).
async fn checkout_fresh(conn: &SmbConn, share: &str) -> Result<SmbCrateBackend, SmbError> {
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .login_params(&conn.user, &conn.password, &conn.domain)
        .await?;
    client.connect_share_retry("pooled-retry", share).await?;
    Ok(client.into_backend())
}

async fn pooled_list(
    conn: &SmbConn,
    share: &str,
    path: &str,
) -> Result<Vec<SmbEntry>, SmbError> {
    let key = PoolKey::of(conn, share);
    let co = checkout(&key, conn, share).await?;
    let mut backend = co.backend;
    match backend.list_directory(path).await {
        Ok(files) => {
            checkin(key, backend).await;
            Ok(files.into_iter().filter(|e| e.name != "." && e.name != "..").collect())
        }
        Err(m) => {
            if SmbClient::<SmbCrateBackend>::should_retry("list", &m) {
                if let Ok(mut fresh) = checkout_fresh(conn, share).await {
                    match fresh.list_directory(path).await {
                        Ok(files) => {
                            checkin(key, fresh).await;
                            return Ok(files
                                .into_iter()
                                .filter(|e| e.name != "." && e.name != "..")
                                .collect());
                        }
                        Err(m2) => return Err(classify_error(&m2)),
                    }
                }
            }
            Err(classify_error(&m))
        }
    }
}

/// Parse `//host/share/rest`, `\\host\share\rest`, or
/// `smb://host/share/rest` into (host, share, rest). Rest uses `/`.
pub fn parse_unc(s: &str) -> Option<(String, String, String)> {
    let rest = s
        .strip_prefix("smb://")
        .or_else(|| s.strip_prefix("//"))
        .or_else(|| s.strip_prefix("\\\\"))?;
    let mut parts = rest.split(['/', '\\']);
    let host = parts.next()?.trim().to_string();
    let share = parts.next()?.trim().to_string();
    if host.is_empty() || share.is_empty() {
        return None;
    }
    let rest = parts.collect::<Vec<_>>().join("/");
    Some((host, share, rest))
}

/// Credentials for a host: explicit CLI/UI flags win, else the stored
/// Settings primary-server user/domain + saved password for that host.
pub fn conn_for(
    host: &str,
    user: Option<&str>,
    password: Option<&str>,
    domain: Option<&str>,
    settings: &crate::settings::Settings,
) -> SmbConn {
    let stored = settings
        .primary_server()
        .filter(|s| s.host == host);
    SmbConn {
        host: host.to_string(),
        user: user
            .map(str::to_string)
            .or_else(|| stored.map(|s| s.user.clone()))
            .unwrap_or_default(),
        password: password
            .map(str::to_string)
            .or_else(|| {
                settings
                    .read_password(host)
                    .map(str::to_string)
            })
            .unwrap_or_default(),
        domain: domain
            .map(str::to_string)
            .or_else(|| stored.map(|s| s.domain.clone()))
            .unwrap_or_default(),
    }
}

/// Recursive book lister (mirrors `listDirectoryRecursive`, epub/pdf filter).
/// Kept iterative here to avoid re-login per directory.
pub fn is_supported_book(name: &str) -> bool {
    match name.rsplit('.').next().map(str::to_ascii_lowercase) {
        Some(ext) => crate::models::BookFormat::from_ext(&ext).is_some(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_and_auth_classify() {
        assert!(is_transient_message("Network is down (50)"));
        assert!(is_transient_message("connection reset by peer"));
        assert!(!is_transient_message("logon failure"));
        assert!(is_auth_failure_message("LOGON_FAILURE"));
        assert!(matches!(classify_error("sharing violation"), SmbError::SharingViolation(_)));
        assert!(matches!(classify_error("object_name_not_found"), SmbError::NotFound(_)));
    }

    #[test]
    fn retryable_errnos_mirror_swift() {
        for code in [50, 51, 60, 64, 65] {
            assert!(is_retryable_errno(code));
        }
        assert!(!is_retryable_errno(2));
    }

    #[test]
    fn pool_eviction_rule() {
        let now = std::time::Instant::now();
        let fresh = now - std::time::Duration::from_secs(10);
        let stale = now - std::time::Duration::from_secs(900);
        assert!(keep_entry(fresh, now, 0));
        assert!(keep_entry(fresh, now, POOL_MAX_IDLE - 1));
        assert!(!keep_entry(fresh, now, POOL_MAX_IDLE));
        assert!(!keep_entry(stale, now, 0));
    }

    #[test]
    fn pool_key_splits_creds() {
        let a = PoolKey::of(
            &SmbConn { host: "h".into(), user: "u".into(), password: "p1".into(), domain: "".into() },
            "s",
        );
        let b = PoolKey::of(
            &SmbConn { host: "h".into(), user: "u".into(), password: "p2".into(), domain: "".into() },
            "s",
        );
        assert_ne!(a, b);
        assert_eq!(a, PoolKey::of(
            &SmbConn { host: "h".into(), user: "u".into(), password: "p1".into(), domain: "".into() },
            "s",
        ));
    }
}
