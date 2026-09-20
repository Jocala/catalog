//! catalog-core: GUI-free business logic for Jocala Catalog
//! (com.jocala.catalog).
//!
//! Ported from the first-party Swift in `macos/Sources` (CatalogCore +
//! ReaderCatalogGUI helpers). Vendored SMBClient / ZIPFoundation are NOT
//! ported — [`smb`] uses the pure-Rust `smb` crate, [`epub`] uses the
//! `zip` crate. UI crates (GTK4 / Win32 / AppKit-Swift) call only these
//! APIs; no GUI types exist in this crate.

pub mod models;
pub mod settings;
pub mod settings_draft;
pub mod db;
pub mod smb;
pub mod epub;
pub mod covers;
pub mod kobo;
pub mod catalog;

// Frequently used roots for UI crates and the CLI.
pub use db::FileSource;
pub use settings::Settings;

/// Unified core error. Module errors map into this via `#[from]` or the
/// `core_err` helpers so UI crates match on one type for alerts.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("auth failed: {0}")]
    AuthFailed(String),
    #[error("network: {0}")]
    Network(String),
    #[error("database unreadable: {0}")]
    Corrupt(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("smb: {0}")]
    Smb(String),
    #[error("{0}")]
    Other(String),
}

impl From<models::CatalogDbError> for CoreError {
    fn from(e: models::CatalogDbError) -> Self {
        match e {
            models::CatalogDbError::NotConfigured(m) => Self::NotConfigured(m),
            models::CatalogDbError::NotFound { smb_path, detail } => {
                Self::NotFound(format!("{smb_path}: {detail}"))
            }
            models::CatalogDbError::AuthFailed { smb_path, detail } => {
                Self::AuthFailed(format!("{smb_path}: {detail}"))
            }
            models::CatalogDbError::Network { smb_path, detail } => {
                Self::Network(format!("{smb_path}: {detail}"))
            }
            models::CatalogDbError::Corrupt(m) => Self::Corrupt(m),
        }
    }
}

impl From<smb::SmbError> for CoreError {
    fn from(e: smb::SmbError) -> Self {
        match e {
            smb::SmbError::Auth(m) => Self::AuthFailed(m),
            smb::SmbError::NotFound(m) => Self::NotFound(m),
            smb::SmbError::Network(m) => Self::Network(m),
            smb::SmbError::SharingViolation(m) => Self::Network(format!("sharing violation: {m}")),
            smb::SmbError::NotConfigured(m) => Self::NotConfigured(m),
            smb::SmbError::Io(m) => Self::Other(format!("io: {m}")),
        }
    }
}
