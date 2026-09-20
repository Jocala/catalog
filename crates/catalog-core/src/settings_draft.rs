//! Full Settings dialog port of Swift `SettingsView`
//! (`ReaderCatalogApp.swift` lines ~1090-1531).
//!
//! Sections (same order as Swift):
//! 1. Library Source — SMB/Local toggle + Local Calibre Path + Browse + Test
//! 2. SMB — server + Test SMB, share, calibre path + Test Calibre,
//!    user, password (show/hide), domain
//! 3. Kobo — multi-IP list with star-default, per-IP Test (ping),
//!    remove, Add
//! 4. Theme — Light/Dark/System
//! 5. Status + Cancel/Save (Save validates like Swift `save()`)

use crate::models::{CalibreLibraryConfig, ConnectionType, SmbServer, SmbShare};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassFail {
    Pass,
    Fail,
}

impl PassFail {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Fail => "Fail",
        }
    }
}

/// Badge text for Slint (`g-*-badge` properties). Empty when untested.
pub fn badge_label(v: Option<PassFail>) -> &'static str {
    match v {
        Some(pf) => pf.label(),
        None => "",
    }
}

#[derive(Debug, Clone, Default)]
pub struct SettingsDraft {
    pub library_source: String,
    pub local_dir: String,
    pub smb_server: String,
    pub smb_share: String,
    pub smb_user: String,
    pub smb_pass: String,
    pub show_pass: bool,
    pub smb_domain: String,
    pub calibre_path: String,
    pub status: String,
    pub smb_conn: Option<PassFail>,
    pub smb_db: Option<PassFail>,
    pub local_test: Option<PassFail>,
    pub kobo_ip: String,
    pub kobo_ips: Vec<String>,
    pub new_kobo_ip: String,
    pub kobo_results: HashMap<String, PassFail>,
    pub theme_preference: i32,
}

impl SettingsDraft {
    /// Mirrors Swift `SettingsView.load()`.
    pub fn load(s: &crate::settings::Settings) -> Self {
        let mut d = Self {
            library_source: if s.library_source.is_empty() {
                "smb".to_string()
            } else {
                s.library_source.clone()
            },
            local_dir: s.local_library_dir.clone(),
            theme_preference: s.theme_preference,
            kobo_ip: s.kobo_ip.clone(),
            kobo_ips: s.kobo_ips.clone(),
            ..Default::default()
        };
        if let Some(server) = s.primary_server() {
            d.smb_server = server.host.clone();
            d.smb_user = server.user.clone();
            d.smb_domain = server.domain.clone();
            if let Some(pw) = s.read_password(&server.host) {
                d.smb_pass = pw.to_string();
            }
            if let Some(share) = server.shares.first() {
                d.smb_share = share.name.clone();
                // Share-relative dir of metadata.db, e.g.
                // "calibre/metadata.db" -> "calibre/".
                let remote = &share.calibre_metadata_path;
                if let Some(i) = remote.rfind('/') {
                    d.calibre_path = format!("{}/", &remote[..i]);
                }
            }
        }
        // Kobo fallback: legacy single `kobo_ip` seeds the list.
        if d.kobo_ips.is_empty() && !d.kobo_ip.is_empty() {
            d.kobo_ips = vec![d.kobo_ip.clone()];
        }
        if !d.kobo_ips.contains(&d.kobo_ip) && !d.kobo_ips.is_empty() {
            d.kobo_ip = d.kobo_ips[0].clone();
        }
        d
    }

    pub fn clean_local(&self) -> String {
        let mut dir = self.local_dir.trim().to_string();
        if let Some(s) = dir.strip_prefix("file://") {
            dir = s.to_string();
        }
        while dir.ends_with('/') && dir.len() > 1 {
            dir.pop();
        }
        dir
    }

    fn clean_lib_dir(&self) -> String {
        let mut lib = self.calibre_path.trim().to_string();
        while lib.starts_with('/') {
            lib.remove(0);
        }
        if !lib.is_empty() && !lib.ends_with('/') {
            lib.push('/');
        }
        lib
    }

    /// Host + share only (no credentials) — safe to snapshot for a worker thread.
    pub fn smb_host_share(&self) -> Option<(String, String)> {
        let host = self.smb_server.trim().to_string();
        let share = self.smb_share.trim().to_string();
        if host.is_empty() || share.is_empty() {
            return None;
        }
        Some((host, share))
    }

    /// Mirrors Swift `save()`. Returns false when the sheet should stay
    /// open (SMB selected but host/share missing). Persists Kobo default
    /// even when SMB validation blocks dismissal.
    pub fn apply_to(&mut self, s: &mut crate::settings::Settings) -> bool {
        if self.library_source != "smb" && self.library_source != "local" {
            self.library_source = "smb".to_string();
        }
        s.library_source = self.library_source.clone();
        let clean_local = self.clean_local();
        self.local_dir = clean_local.clone();
        s.local_library_dir = clean_local.clone();
        let lib_dir = self.clean_lib_dir();
        self.calibre_path = lib_dir.clone();

        let host = self.smb_server.trim().to_string();
        let share_name = self.smb_share.trim().to_string();
        let user = self.smb_user.trim().to_string();
        let domain = self.smb_domain.trim().to_string();
        let pass = self.smb_pass.clone();

        self.save_kobo(s);
        if self.library_source == "smb" && (host.is_empty() || share_name.is_empty()) {
            self.status = if self.kobo_ip.is_empty() {
                "Enter SMB host and share first".to_string()
            } else {
                format!("Enter SMB host and share first (Kobo default saved: {})", self.kobo_ip)
            };
            return false;
        }

        let meta_path = format!("{lib_dir}metadata.db");
        let server = SmbServer {
            label: String::new(),
            host: host.clone(),
            port: 445,
            user,
            domain,
            shares: vec![SmbShare {
                name: share_name.clone(),
                calibre_metadata_path: meta_path.clone(),
            }],
        };
        s.save_servers(vec![server]);
        if !pass.is_empty() && !host.is_empty() {
            s.save_password(&host, &pass);
        }
        // Single "Main" primary library (mirrors Swift save()).
        let full_path = format!("{host}/{share_name}/{meta_path}");
        if let Some(idx) = s
            .calibre_libraries
            .iter()
            .position(|l| l.conn_type == ConnectionType::Smb && l.smb_host() == Some(host.as_str()))
        {
            s.calibre_libraries[idx].path = full_path;
            s.calibre_libraries[idx].name = "Main".to_string();
            for (i, l) in s.calibre_libraries.iter_mut().enumerate() {
                l.is_primary = i == idx;
            }
        } else {
            s.calibre_libraries.retain(|l| l.conn_type != ConnectionType::Smb);
            let mut cfg = CalibreLibraryConfig {
                name: "Main".to_string(),
                conn_type: ConnectionType::Smb,
                path: full_path,
                is_primary: true,
            };
            if s.calibre_libraries.is_empty() {
                cfg.is_primary = true;
            }
            s.calibre_libraries.push(cfg);
            if s.calibre_libraries.len() == 1 {
                s.calibre_libraries[0].is_primary = true;
            }
        }
        s.theme_preference = self.theme_preference;
        if self.library_source == "local" {
            self.status = format!(
                "Saved Local {}",
                if clean_local.is_empty() { "(no folder)".to_string() } else { clean_local }
            );
        } else {
            self.status = format!("Saved SMB {host}/{share_name}/{meta_path}");
        }
        true
    }

    fn save_kobo(&self, s: &mut crate::settings::Settings) {
        self.save_kobo_into(s);
    }

    /// Persist just the Kobo list + default (Swift saves on every
    /// star/add/remove tap, independent of the Save button).
    pub fn save_kobo_into(&self, s: &mut crate::settings::Settings) {
        s.kobo_ips = self.kobo_ips.clone();
        s.kobo_ip = self.kobo_ip.clone();
    }

    // -- Kobo list (add/remove/star) — mirrors Swift --

    pub fn add_kobo(&mut self) {
        let ip = self.new_kobo_ip.trim().to_string();
        if ip.is_empty() {
            return;
        }
        if self.kobo_ips.contains(&ip) {
            self.status = "Already exists".to_string();
            return;
        }
        self.kobo_ips.push(ip.clone());
        if self.kobo_ip.is_empty() {
            self.kobo_ip = ip.clone();
        }
        self.new_kobo_ip.clear();
        self.status = format!("Added {ip}");
    }

    pub fn remove_kobo(&mut self, ip: &str) {
        self.kobo_ips.retain(|x| x != ip);
        self.kobo_results.remove(ip);
        if self.kobo_ip == ip {
            self.kobo_ip = self.kobo_ips.first().cloned().unwrap_or_default();
        }
        self.status = if self.kobo_ips.is_empty() {
            "No Kobo IPs".to_string()
        } else {
            format!("Removed {ip}")
        };
        if self.kobo_ip.is_empty() && !self.kobo_ips.is_empty() {
            self.kobo_ip = self.kobo_ips[0].clone();
        }
    }

    // -- Tests (mirrors Swift testLocal/testSMBConnection/testSMBDatabase/testKobo) --
    //
    // The `check_*` fns below are the blocking bodies: they take plain
    // inputs and touch no shared state, so main.rs runs them on worker
    // threads and applies the results via invoke_from_event_loop. Never
    // call them on the Slint UI thread — DNS, TCP timeouts, ping, and a
    // 40 MB+ metadata.db read all stall the event loop (beachball).
    // The `test_*` wrappers preserve the old synchronous API.

    /// Blocking local check — worker thread only.
    pub fn check_local(dir: &str) -> (Option<PassFail>, String) {
        if dir.is_empty() {
            return (Some(PassFail::Fail), "Fail Local: enter a folder first".to_string());
        }
        let db_path = std::path::PathBuf::from(dir).join("metadata.db");
        if !db_path.exists() {
            return (Some(PassFail::Fail), format!("Fail Local: no metadata.db in {dir}"));
        }
        match crate::db::read_local_db(dir)
            .ok()
            .and_then(|b| crate::db::open_memory_db(&b).ok())
            .and_then(|db| crate::db::fetch_count(&db, "").ok())
        {
            Some(n) => (Some(PassFail::Pass), format!("OK Local {n} books")),
            None => (Some(PassFail::Fail), "Fail Local: unreadable".to_string()),
        }
    }

    /// Credentials snapshot for worker-thread SMB ops (no UI types).
    pub fn smb_conn(&self) -> crate::smb::SmbConn {
        crate::smb::SmbConn {
            host: self.smb_server.trim().to_string(),
            user: self.smb_user.trim().to_string(),
            password: self.smb_pass.clone(),
            domain: self.smb_domain.trim().to_string(),
        }
    }

    /// Real SMB login + share connect (Swift testConnection parity).
    /// Async — call from a worker thread or under a runtime, never the
    /// UI thread (DNS + handshake stall the event loop).
    pub async fn check_smb_connection(
        conn: &crate::smb::SmbConn,
        share: &str,
    ) -> (Option<PassFail>, String) {
        if conn.host.trim().is_empty() || share.trim().is_empty() {
            return (
                Some(PassFail::Fail),
                "Fail SMB-connection: enter host and share first".to_string(),
            );
        }
        match crate::smb::test_share(conn.clone(), share).await {
            Ok(()) => (
                Some(PassFail::Pass),
                format!("OK SMB-connection {}/{}", conn.host, share),
            ),
            Err(e) => (Some(PassFail::Fail), format!("Fail SMB-connection {e}")),
        }
    }

    /// Real database probe (Swift testSMBDatabase parity): list the
    /// library dir and expect metadata.db. Async — worker thread only.
    pub async fn check_smb_database(
        conn: &crate::smb::SmbConn,
        share: &str,
        lib_dir: &str,
    ) -> (Option<PassFail>, String) {
        if conn.host.trim().is_empty() || share.trim().is_empty() {
            return (
                Some(PassFail::Fail),
                "Fail SMB-database: enter host and share first".to_string(),
            );
        }
        let dir = lib_dir.trim_matches('/');
        match crate::smb::list_dir(conn.clone(), share, dir).await {
            Ok(entries) => {
                let found = entries.iter().any(|e| e.name.eq_ignore_ascii_case("metadata.db"));
                if found {
                    (
                        Some(PassFail::Pass),
                        format!(
                            "OK SMB-database {}/{}/{}metadata.db found",
                            conn.host, share, lib_dir
                        ),
                    )
                } else {
                    (
                        Some(PassFail::Fail),
                        format!(
                            "Fail SMB-database {}/{}/{lib_dir} — no metadata.db here",
                            conn.host, share
                        ),
                    )
                }
            }
            Err(e) => (Some(PassFail::Fail), format!("Fail SMB-database {e}")),
        }
    }

    /// Blocking single ICMP ping (mirrors Swift `testKobo`) — worker
    /// thread only. Returns the trimmed target (empty when input was
    /// empty, in which case the caller reports without touching results).
    pub fn check_kobo(ip: &str) -> (String, PassFail, String) {
        let target = ip.trim().to_string();
        if target.is_empty() {
            return (target, PassFail::Fail, "Enter Kobo IP".to_string());
        }
        let exe = ["/sbin/ping", "/bin/ping", "/usr/bin/ping"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .unwrap_or("/sbin/ping");
        let out = std::process::Command::new(exe)
            .args(["-c", "1", "-W", "1000", "-o", &target])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                let summary = text.lines().last().unwrap_or("").trim();
                let status = format!("Kobo awake {target} ({}…)", summary.chars().take(120).collect::<String>());
                (target, PassFail::Pass, status)
            }
            _ => {
                let status =
                    format!("Kobo is sleeping or unreachable at {target} — press power button to wake.");
                (target, PassFail::Fail, status)
            }
        }
    }

}

// NOTE: folder picking lives in the UI crates (GtkFileDialog / Win32
// GetOpenFileName / NSOpenPanel) — catalog-core stays GUI-free.


