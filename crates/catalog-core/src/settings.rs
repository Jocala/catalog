//! Port of `JReaderPaths.swift` + `KeychainHelper.swift` +
//! `ReaderLog.swift` + `NotificationNames.swift` + the UserDefaults-backed
//! halves of `SmbCredentials.swift` / `CalibreManager.swift` / SettingsView.
//!
//! Swift stores everything in `UserDefaults` (+ passwords in a
//! `smb_pass_<host>` UserDefaults mirror — deliberately NOT the macOS
//! Keychain, to avoid a login-keychain prompt). catalog-core mirrors that
//! with a single JSON file in the platform data dir
//! (`directories::ProjectDirs`), so behaviour stays identical for every
//! native UI: `settings.json` next to covers/, thumbnails/, logs.

use crate::models::{CalibreLibraryConfig, SmbServer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Paths (JReaderPaths.swift) — platform data dir, never Desktop/Documents.
// ---------------------------------------------------------------------------

/// Platform data dir (`~/Library/Application Support/com.jocala.Catalog`
/// on macOS, `%APPDATA%\jocala\Catalog` on Windows,
/// `~/.local/share/catalog` on Linux). Created on first use.
/// Product identity: com.jocala.catalog / Jocala Catalog.
pub fn base_dir() -> PathBuf {
    let dir = directories::ProjectDirs::from("com", "jocala", "Catalog")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn books_dir() -> PathBuf {
    let d = base_dir().join("Books");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn covers_dir() -> PathBuf {
    let d = base_dir().join("covers");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn thumbnails_dir() -> PathBuf {
    let d = base_dir().join("thumbnails");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn log_file() -> PathBuf {
    base_dir().join("app_log.txt")
}

pub fn old_log_file() -> PathBuf {
    base_dir().join("app_log_old.txt")
}

pub fn kobo_index_file() -> PathBuf {
    base_dir().join("kobo_index.txt")
}

pub fn settings_file() -> PathBuf {
    base_dir().join("settings.json")
}

// ---------------------------------------------------------------------------
// Settings blob (UserDefaults keys used across the Swift app)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub smb_servers: Vec<SmbServer>,
    #[serde(default)]
    pub smb_passwords: HashMap<String, String>,
    #[serde(default)]
    pub calibre_libraries: Vec<CalibreLibraryConfig>,
    #[serde(default = "default_library_source")]
    pub library_source: String,
    #[serde(default)]
    pub local_library_dir: String,
    #[serde(default)]
    pub kobo_ip: String,
    #[serde(default)]
    pub kobo_ips: Vec<String>,
    #[serde(default)]
    pub theme_preference: i32,
    #[serde(default)]
    pub diagnostic_logging: bool,
    /// Global one-time prompt for KOReader stacking fix. `true` = already answered (Yes or No), never ask again.
    #[serde(default)]
    pub kobo_handoff_prompt_done: bool,
}

fn default_library_source() -> String {
    "smb".to_string()
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_file();
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).expect("settings serializable");
        fs::write(path, json)
    }

    // -- SmbServer.saved / .primary / .save (SmbCredentials.swift) --

    pub fn saved_servers(&self) -> &[SmbServer] {
        &self.smb_servers
    }

    pub fn primary_server(&self) -> Option<&SmbServer> {
        self.smb_servers.first()
    }

    pub fn save_servers(&mut self, servers: Vec<SmbServer>) {
        self.smb_servers = servers;
    }

    // -- KeychainHelper.save/read(password:for:) — UserDefaults mirror --

    pub fn save_password(&mut self, account: &str, password: &str) {
        self.smb_passwords
            .insert(account.to_string(), password.to_string());
    }

    pub fn read_password(&self, account: &str) -> Option<&str> {
        self.smb_passwords
            .get(account)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }

    // -- CalibreManager registry (add/remove/setPrimary/update) --

    pub fn add_library(&mut self, mut lib: CalibreLibraryConfig) {
        if self.calibre_libraries.iter().any(|l| l.name == lib.name) {
            return;
        }
        if self.calibre_libraries.is_empty() && !lib.is_primary {
            lib.is_primary = true;
        }
        self.calibre_libraries.push(lib);
        if self.calibre_libraries.len() == 1 && !self.calibre_libraries[0].is_primary {
            self.calibre_libraries[0].is_primary = true;
        }
    }

    pub fn remove_library(&mut self, name: &str) {
        self.calibre_libraries.retain(|l| l.name != name);
        if self.calibre_libraries.len() == 1 && !self.calibre_libraries[0].is_primary {
            self.calibre_libraries[0].is_primary = true;
        }
    }

    pub fn set_primary_library(&mut self, name: &str) {
        for l in &mut self.calibre_libraries {
            l.is_primary = l.name == name;
        }
    }

    pub fn update_library(&mut self, lib: CalibreLibraryConfig) {
        if let Some(slot) = self.calibre_libraries.iter_mut().find(|l| l.name == lib.name) {
            *slot = lib;
        }
    }

    pub fn primary_library(&self) -> Option<&CalibreLibraryConfig> {
        self.calibre_libraries.iter().find(|l| l.is_primary).or(self.calibre_libraries.first())
    }
}

// ---------------------------------------------------------------------------
// ReaderLog (ReaderLog.swift) — file + stderr, 512 KiB halve-trim,
// current/previous rotation at startup.
// ---------------------------------------------------------------------------

const MAX_LOG_SIZE: u64 = 512 * 1024;

static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn log_lock() -> &'static Mutex<()> {
    LOG_LOCK.get_or_init(|| Mutex::new(()))
}

fn timestamp() -> String {
    // HH:mm:ss.mmm without chrono (keeps deps minimal).
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % 86_400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, now.subsec_millis())
}

fn rotate_at_startup_once() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let cur = log_file();
        let old = old_log_file();
        if cur.exists() {
            let tmp = base_dir().join("app_log.tmp");
            let _ = fs::rename(&cur, &tmp);
            let _ = fs::remove_file(&old);
            let _ = fs::rename(&tmp, &old);
        }
        write_line("I", "ReaderLog", "=== App Start ===");
    });
}

fn trim_if_needed(path: &Path) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.len() <= MAX_LOG_SIZE {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else { return };
    let lines: Vec<&str> = content.lines().collect();
    let kept = lines[lines.len() / 2..].join("\n");
    let _ = fs::write(path, kept);
}

fn write_line(level: &str, tag: &str, msg: &str) {
    let _guard = log_lock().lock().ok();
    let path = log_file();
    trim_if_needed(&path);
    let line = format!("[{}] {level}/{tag}: {msg}\n", timestamp());
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
    eprintln!("[{level}/{tag}] {msg}");
}

pub struct ReaderLog;

impl ReaderLog {
    pub fn init() {
        rotate_at_startup_once();
    }

    pub fn i(tag: &str, msg: &str) {
        rotate_at_startup_once();
        write_line("I", tag, msg);
    }

    pub fn e(tag: &str, msg: &str) {
        rotate_at_startup_once();
        write_line("E", tag, msg);
    }

    pub fn d(tag: &str, msg: &str) {
        if !Settings::load().diagnostic_logging {
            return;
        }
        rotate_at_startup_once();
        write_line("D", tag, msg);
    }

    pub fn clear() {
        let _guard = log_lock().lock().ok();
        let _ = fs::remove_file(log_file());
        let _ = fs::remove_file(old_log_file());
    }

    pub fn current() -> (String, String) {
        let content = fs::read_to_string(log_file())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "(empty)".to_string());
        ("Current Session".to_string(), content)
    }

    pub fn previous() -> Option<(String, String)> {
        let old = old_log_file();
        if !old.exists() {
            return None;
        }
        let content = fs::read_to_string(old).unwrap_or_default();
        Some(("Previous Session".to_string(), content))
    }
}

// ---------------------------------------------------------------------------
// Notification names (NotificationNames.swift) — the UI posts AppEvents.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    ImportedFoldersChanged,
    AuthorDisplayChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_add_promotes_sole_primary() {
        let mut s = Settings::default();
        s.add_library(CalibreLibraryConfig {
            name: "main".into(),
            conn_type: crate::models::ConnectionType::Smb,
            path: "h/share/calibre/metadata.db".into(),
            is_primary: false,
        });
        assert!(s.calibre_libraries[0].is_primary);
        assert!(s.primary_library().is_some());
    }

    #[test]
    fn password_mirror_roundtrip() {
        let mut s = Settings::default();
        assert_eq!(s.read_password("smb-host.example"), None);
        s.save_password("smb-host.example", "test-password");
        assert_eq!(
            s.read_password("smb-host.example"),
            Some("test-password")
        );
    }
}
