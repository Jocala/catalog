//! Port of `CatalogCore/Models/*` + `CalibreManager.CalibreLibraryConfig`
//! + `ReaderCatalogGUI` row types (`CatalogBook`, `SearchResult`).
//!
//! Swift sources:
//! - `CatalogModels.swift` (AuthorBook, AuthorSummary, SeriesSummary,
//!   TagSummary, BookDetail, SearchedBook)
//! - `SmbCredentials.swift` (SmbServer, SmbShare)
//! - `BookFormat.swift`, `Chapter.swift`
//! - `Calibre/CalibreManager.swift` (CalibreLibraryConfig registry entry)
//! - `ReaderCatalogApp.swift` (CatalogBook, BrowseMode, CatalogSortOrder)
//! - `SearchFormSheet.swift` (SearchResult)

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Catalog row DTOs (CatalogModels.swift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorBook {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub path: String,
    pub cover_hash: String,
    pub timestamp: String,
    pub root_folder: String,
    pub author_sort: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorSummary {
    pub id: i64,
    pub name: String,
    pub sort: String,
    pub book_count: usize,
    pub first_book_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesSummary {
    pub id: i64,
    pub name: String,
    pub book_count: usize,
    pub first_book_path: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagSummary {
    pub id: i64,
    pub name: String,
    pub book_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookDetail {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub series: Option<String>,
    pub series_index: f32,
    pub comments: Option<String>,
    pub tags: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub pubdate: Option<String>,
    pub timestamp: String,
    pub author_sort: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchedBook {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub path: String,
    pub series: Option<String>,
    pub tags: Vec<String>,
    pub cover_hash: String,
    pub author_sort: String,
}

// CatalogBook lives in ReaderCatalogApp.swift (GUI row, has_cover flag).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogBook {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub path: String,
    pub has_cover: bool,
    pub cover_hash: Option<String>,
}

// SearchResult lives in SearchFormSheet.swift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub file_path: String,
    pub file_name: String,
    pub book_id: Option<i64>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub series: Option<String>,
    pub tags: Vec<String>,
    pub author_sort: String,
}

impl SearchResult {
    pub fn id(&self) -> &str {
        &self.file_path
    }
}

impl From<SearchedBook> for SearchResult {
    fn from(sb: SearchedBook) -> Self {
        let file_name = sb
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&sb.path)
            .to_string();
        Self {
            file_path: sb.path.clone(),
            file_name,
            book_id: Some(sb.id),
            title: Some(sb.title.clone()),
            author: Some(sb.author.clone()),
            series: sb.series.clone(),
            tags: sb.tags.clone(),
            author_sort: sb.author_sort.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// SMB credentials (SmbCredentials.swift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SmbShare {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub calibre_metadata_path: String,
}

impl SmbShare {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            calibre_metadata_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SmbServer {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub shares: Vec<SmbShare>,
}

fn default_port() -> u16 {
    445
}

impl SmbServer {
    pub fn display_name(&self) -> &str {
        if self.label.is_empty() {
            &self.host
        } else {
            &self.label
        }
    }

    pub fn primary_share_name(&self) -> &str {
        self.shares.first().map(|s| s.name.as_str()).unwrap_or("")
    }

    pub fn set_primary_share_name(&mut self, name: &str) {
        if self.shares.is_empty() {
            self.shares.push(SmbShare::new(name));
        } else if !name.is_empty() {
            self.shares[0].name = name.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Book formats (BookFormat.swift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Pdf,
    Epub,
}

impl BookFormat {
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "epub" => Some(Self::Epub),
            _ => None,
        }
    }

    /// Mirrors `BookFormat.ebookFormats = [.epub]` — the recursive SMB
    /// lister only surfaces epubs.
    pub fn is_ebook_ext(ext: &str) -> bool {
        matches!(Self::from_ext(ext), Some(Self::Epub))
    }
}

pub fn supported_book_exts() -> &'static [&'static str] {
    &["pdf", "epub"]
}

// ---------------------------------------------------------------------------
// Chapters (Chapter.swift + EpubService.SpineItem)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    pub anchor: String,
}

// ---------------------------------------------------------------------------
// Calibre registry (CalibreManager.swift — registry half only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Local,
    Smb,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibreLibraryConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub conn_type: ConnectionType,
    pub path: String,
    #[serde(default)]
    pub is_primary: bool,
}

impl CalibreLibraryConfig {
    pub fn display_path(&self) -> String {
        match self.conn_type {
            ConnectionType::Local => self.path.clone(),
            ConnectionType::Smb => format!("smb://{}", self.path),
            ConnectionType::Remote => self.path.clone(),
        }
    }

    fn parts(&self) -> Vec<&str> {
        self.path.split('/').collect()
    }

    pub fn smb_host(&self) -> Option<&str> {
        if self.conn_type != ConnectionType::Smb {
            return None;
        }
        self.parts().first().copied()
    }

    pub fn smb_share(&self) -> Option<&str> {
        if self.conn_type != ConnectionType::Smb {
            return None;
        }
        let parts = self.parts();
        if parts.len() >= 2 {
            Some(parts[1])
        } else {
            None
        }
    }

    pub fn smb_metadata_path(&self) -> String {
        let parts = self.parts();
        if parts.len() >= 3 {
            parts[2..].join("/")
        } else {
            String::new()
        }
    }

    pub fn smb_lib_root(&self) -> String {
        let meta = self.smb_metadata_path();
        match meta.rfind('/') {
            Some(i) => meta[..i].to_string(),
            None => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Browse / sort enums (ReaderCatalogApp.swift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrowseMode {
    Books,
    Author,
    Series,
    Tags,
}

impl BrowseMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Books => "Books",
            Self::Author => "Author",
            Self::Series => "Series",
            Self::Tags => "Tags",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CatalogSortOrder {
    Author,
    Az,
    Za,
    Date,
    Oldest,
}

impl CatalogSortOrder {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Author => "Author",
            Self::Az => "A–Z",
            Self::Za => "Z–A",
            Self::Date => "Newest",
            Self::Oldest => "Oldest",
        }
    }

    pub fn is_descending(&self) -> bool {
        matches!(self, Self::Za | Self::Oldest)
    }
    pub fn is_by_author(&self) -> bool {
        matches!(self, Self::Author)
    }
    pub fn is_by_date(&self) -> bool {
        matches!(self, Self::Date | Self::Oldest)
    }
}

// ---------------------------------------------------------------------------
// Errors (CatalogDBError in SmbCatalogDB.swift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum CatalogDbError {
    #[error("{0}")]
    NotConfigured(String),
    #[error("Calibre database not found at {smb_path}.\nCheck Settings → SMB server and Calibre path.\n{detail}")]
    NotFound { smb_path: String, detail: String },
    #[error("SMB login failed for {smb_path}.\nCheck Settings → SMB user and password.\n{detail}")]
    AuthFailed { smb_path: String, detail: String },
    #[error("Could not reach the Calibre database at {smb_path}.\n{detail}")]
    Network { smb_path: String, detail: String },
    #[error("Calibre database unreadable: {0}")]
    Corrupt(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibre_smb_path_splitters() {
        let c = CalibreLibraryConfig {
            name: "main".into(),
            conn_type: ConnectionType::Smb,
            path: "smb-host.example/ebooks/calibre/metadata.db".into(),
            is_primary: true,
        };
        assert_eq!(c.smb_host(), Some("smb-host.example"));
        assert_eq!(c.smb_share(), Some("ebooks"));
        assert_eq!(c.smb_metadata_path(), "calibre/metadata.db");
        assert_eq!(c.smb_lib_root(), "calibre");
        assert_eq!(c.display_path(), "smb://smb-host.example/ebooks/calibre/metadata.db");
    }

    #[test]
    fn book_format_exts() {
        assert!(BookFormat::is_ebook_ext("epub"));
        assert!(BookFormat::is_ebook_ext("EPUB"));
        assert!(!BookFormat::is_ebook_ext("pdf"));
        assert_eq!(BookFormat::from_ext("pdf"), Some(BookFormat::Pdf));
    }

    #[test]
    fn smb_server_primary_share() {
        let mut s = SmbServer {
            label: String::new(),
            host: "smb-host.example".into(),
            port: 445,
            user: "test-user".into(),
            domain: String::new(),
            shares: vec![],
        };
        assert_eq!(s.primary_share_name(), "");
        s.set_primary_share_name("ebooks");
        assert_eq!(s.primary_share_name(), "ebooks");
    }
}
