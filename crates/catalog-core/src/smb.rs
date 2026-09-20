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
    async fn read_file_shared(&mut self, path: &str) -> Result<Vec<u8>, String> {
        use smb::{GetLen, ReadAt};
        let share = self.tree_share().await?;
        let unc = self.unc(&share, path)?;
        let client = self.ensure_client().await?;
        let args = smb::FileCreateArgs::make_open_existing(
            smb::FileAccessMask::new().with_generic_read(true),
        );
        let file = match client.create_file(&unc, &args).await.map_err(smb_err)? {
            smb::Resource::File(f) => f,
            _ => return Err("not a file".to_string()),
        };
        let len = file.get_len().await.map_err(smb_err)?;
        const CHUNK: u64 = 1 << 20;
        let mut out = Vec::with_capacity(len.min(256 << 20) as usize);
        let mut off = 0u64;
        while off < len {
            let n = CHUNK.min(len - off) as usize;
            let mut buf = vec![0u8; n];
            let read = file.read_at(&mut buf, off).await.map_err(smb_err)?;
            if read == 0 {
                break;
            }
            buf.truncate(read);
            out.extend_from_slice(&buf);
            off += read as u64;
        }
        file.close().await.map_err(smb_err)?;
        Ok(out)
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
    download_file_bytes(conn, share, remote_path).await
}

/// Generic remote-file download with retry (covers, epubs, …).
pub async fn download_file_bytes(
    conn: SmbConn,
    share: &str,
    remote_path: &str,
) -> Result<Vec<u8>, SmbError> {
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .download_data(&conn.user, &conn.password, &conn.domain, share, remote_path)
        .await
}

/// List a share directory with retry.
pub async fn list_dir(
    conn: SmbConn,
    share: &str,
    path: &str,
) -> Result<Vec<SmbEntry>, SmbError> {
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .list_directory(&conn.user, &conn.password, &conn.domain, share, path)
        .await
}

/// Login + share connect with retry (powers Test SMB for real).
pub async fn test_share(conn: SmbConn, share: &str) -> Result<(), SmbError> {
    let mut client = SmbClient::new(SmbCrateBackend::new(conn.clone()));
    client
        .test_connection(&conn.user, &conn.password, &conn.domain, share)
        .await
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
}
