//! Port of `ReaderCatalogGUI/SmbCatalogDB.swift`.
//!
//! Single-database rule: the ONLY database is the live Calibre
//! `metadata.db`, read fully into memory then opened as an in-memory
//! SQLite DB (Swift uses `sqlite3_deserialize` + FREEONCLOSE; here we
//! write bytes to a temp file and use rusqlite's backup into `:memory:`
//! so no snapshot file survives). All SQL below mirrors the Swift
//! queries verbatim against the canonical Calibre schema.

use crate::models::{
    AuthorBook, AuthorSummary, BookDetail, CatalogBook, CatalogDbError, SearchedBook, SeriesSummary,
    TagSummary,
};
use rusqlite::{Connection, Row};
use std::path::PathBuf;

fn col_string(row: &Row, idx: usize) -> Option<String> {
    row.get::<_, Option<String>>(idx).unwrap_or(None)
}

fn col_string_or(row: &Row, idx: usize, fallback: &str) -> String {
    col_string(row, idx).unwrap_or_else(|| fallback.to_string())
}

pub fn strip_calibre_html(html: &str) -> String {
    let re = regex::Regex::new("<[^>]+>").unwrap();
    let mut s = re.replace_all(html, "").to_string();
    for (e, r) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ] {
        s = s.replace(e, r);
    }
    s.trim().to_string()
}

/// Directory entry for listings (local `read_dir` or SMB QUERY_DIRECTORY).
/// Plain data — UI crates never see protocol types.
pub type DirEntry = crate::smb::SmbEntry;

#[derive(Debug, Clone)]
pub struct SmbLocation {
    pub host: String,
    pub share: String,
    /// Path of metadata.db inside the share, e.g. `calibre/metadata.db`.
    pub remote_path: String,
    /// Dir of metadata.db inside the share, e.g. `calibre`.
    pub lib_root: String,
}

/// Thin file-source abstraction (Phase 4): local folder or SMB share.
/// UI crates match on this only — never on protocol details.
#[derive(Debug, Clone)]
pub enum FileSource {
    Local { dir: PathBuf },
    Smb { loc: SmbLocation, conn: crate::smb::SmbConn },
}

impl From<crate::smb::SmbError> for CatalogDbError {
    fn from(e: crate::smb::SmbError) -> Self {
        // Same crate, so the orphan rule allows this bridge.
        match e {
            crate::smb::SmbError::Auth(m) => CatalogDbError::AuthFailed {
                smb_path: String::new(),
                detail: m,
            },
            crate::smb::SmbError::NotFound(m) => CatalogDbError::NotFound {
                smb_path: String::new(),
                detail: m,
            },
            crate::smb::SmbError::Network(m) => CatalogDbError::Network {
                smb_path: String::new(),
                detail: m,
            },
            crate::smb::SmbError::SharingViolation(m) => CatalogDbError::Network {
                smb_path: String::new(),
                detail: format!("sharing violation: {m}"),
            },
            crate::smb::SmbError::NotConfigured(m) => CatalogDbError::NotConfigured(m),
            crate::smb::SmbError::Io(m) => CatalogDbError::Corrupt(format!("io: {m}")),
        }
    }
}

impl FileSource {
    pub fn local(dir: impl Into<PathBuf>) -> Self {
        Self::Local { dir: dir.into() }
    }

    /// SMB source for an explicit library dir inside a share.
    /// `remote_dir` is the folder holding metadata.db ("" = share root).
    pub fn smb_with_creds(
        host: &str,
        share: &str,
        remote_dir: &str,
        user: &str,
        password: &str,
        domain: &str,
    ) -> Self {
        let remote_dir = remote_dir.trim_matches('/').to_string();
        let remote_path = if remote_dir.is_empty() {
            "metadata.db".to_string()
        } else {
            format!("{remote_dir}/metadata.db")
        };
        Self::Smb {
            loc: SmbLocation {
                host: host.to_string(),
                share: share.to_string(),
                remote_path,
                lib_root: remote_dir,
            },
            conn: crate::smb::SmbConn {
                host: host.to_string(),
                user: user.to_string(),
                password: password.to_string(),
                domain: domain.to_string(),
            },
        }
    }

    pub fn display_path(&self) -> String {
        match self {
            Self::Smb { loc, .. } => {
                format!("smb://{}/{}/{}", loc.host, loc.share, loc.remote_path)
            }
            Self::Local { dir } => format!("file://{}/metadata.db", dir.display()),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    fn local_verified(mut dir: String) -> Result<Self, CatalogDbError> {
        if dir.is_empty() {
            return Err(CatalogDbError::NotConfigured(
                "No local library folder.".into(),
            ));
        }
        if let Some(s) = dir.strip_prefix("file://") {
            dir = s.to_string();
        }
        while dir.ends_with('/') && dir.len() > 1 {
            dir.pop();
        }
        let db = PathBuf::from(&dir).join("metadata.db");
        if !db.exists() {
            return Err(CatalogDbError::NotFound {
                smb_path: format!("file://{}", db.display()),
                detail: "metadata.db not found in that folder".into(),
            });
        }
        Ok(Self::Local { dir: PathBuf::from(dir) })
    }

    /// CLI/UI entry point: a local dir or `smb://host/share[/dir]`.
    /// SMB creds: explicit flags win, else stored Settings for that host.
    pub fn from_arg(
        path: &str,
        user: Option<&str>,
        password: Option<&str>,
        domain: Option<&str>,
        settings: &crate::settings::Settings,
    ) -> Result<Self, CatalogDbError> {
        if let Some(rest) = path.strip_prefix("smb://") {
            let mut parts = rest.split('/');
            let host = parts.next().unwrap_or("").trim().to_string();
            let share = parts.next().unwrap_or("").trim().to_string();
            if host.is_empty() || share.is_empty() {
                return Err(CatalogDbError::NotConfigured(
                    "smb:// URL must be smb://host/share[/dir]".into(),
                ));
            }
            let remote_dir = parts.collect::<Vec<_>>().join("/");
            let conn = crate::smb::conn_for(&host, user, password, domain, settings);
            return Ok(Self::smb_with_creds(
                &host,
                &share,
                &remote_dir,
                &conn.user,
                &conn.password,
                &conn.domain,
            ));
        }
        Self::local_verified(path.trim().to_string())
    }

    /// Mirrors `resolveTarget()` using Settings (no hardcoded hosts).
    /// SMB credentials come from the primary server + password store.
    pub fn from_settings(settings: &crate::settings::Settings) -> Result<Self, CatalogDbError> {
        if settings.library_source == "local" {
            return Self::local_verified(settings.local_library_dir.trim().to_string());
        }
        let server = settings.primary_server().ok_or_else(|| {
            CatalogDbError::NotConfigured("No SMB server configured — open Settings → SMB first.".into())
        })?;
        if server.host.is_empty() {
            return Err(CatalogDbError::NotConfigured("No SMB server configured.".into()));
        }
        let share = server.shares.first().ok_or_else(|| {
            CatalogDbError::NotConfigured("No SMB share configured.".into())
        })?;
        if share.name.is_empty() {
            return Err(CatalogDbError::NotConfigured("No SMB share configured.".into()));
        }
        if share.calibre_metadata_path.is_empty() {
            return Err(CatalogDbError::NotConfigured("No Calibre path configured.".into()));
        }
        let remote = share.calibre_metadata_path.clone();
        let root = match remote.rfind('/') {
            Some(i) => remote[..i].to_string(),
            None => String::new(),
        };
        let password = settings.read_password(&server.host).unwrap_or("").to_string();
        Ok(Self::Smb {
            loc: SmbLocation {
                host: server.host.clone(),
                share: share.name.clone(),
                remote_path: remote,
                lib_root: root,
            },
            conn: crate::smb::SmbConn {
                host: server.host.clone(),
                user: server.user.clone(),
                password,
                domain: server.domain.clone(),
            },
        })
    }

    /// Read the live metadata.db bytes from either source.
    pub async fn read_db_bytes(&self) -> Result<Vec<u8>, CatalogDbError> {
        match self {
            Self::Local { dir } => {
                let dir = dir.to_string_lossy().to_string();
                read_local_db(&dir)
            }
            Self::Smb { loc, conn } => {
                crate::smb::download_db_bytes(conn.clone(), &loc.share, &loc.remote_path)
                    .await
                    .map_err(|e| {
                        let mut db_err = CatalogDbError::from(e);
                        // Attach the location for actionable errors.
                        let path = format!("smb://{}/{}/{}", loc.host, loc.share, loc.remote_path);
                        match &mut db_err {
                            CatalogDbError::NotFound { smb_path, .. }
                            | CatalogDbError::AuthFailed { smb_path, .. }
                            | CatalogDbError::Network { smb_path, .. } => {
                                *smb_path = path;
                            }
                            _ => {}
                        }
                        db_err
                    })
            }
        }
    }

    /// List a directory: local folder or share-relative path ("" = root).
    pub async fn list_dir(&self, rel: &str) -> Result<Vec<DirEntry>, CatalogDbError> {
        match self {
            Self::Local { dir } => {
                let base = dir.join(rel.trim_matches('/'));
                let rd = std::fs::read_dir(&base).map_err(|e| CatalogDbError::NotFound {
                    smb_path: format!("file://{}", base.display()),
                    detail: e.to_string(),
                })?;
                let mut out = vec![];
                for entry in rd.flatten() {
                    let ft = entry.file_type().ok();
                    let md = entry.metadata().ok();
                    out.push(DirEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        is_directory: ft.map(|t| t.is_dir()).unwrap_or(false),
                        size: md.map(|m| m.len()).unwrap_or(0),
                    });
                }
                out.sort_by_key(|a| a.name.to_lowercase());
                Ok(out)
            }
            Self::Smb { loc, conn } => {
                let sub = if loc.lib_root.is_empty() {
                    rel.trim_matches('/').to_string()
                } else if rel.trim().is_empty() {
                    loc.lib_root.clone()
                } else {
                    format!("{}/{}", loc.lib_root, rel.trim_matches('/'))
                };
                crate::smb::list_dir(conn.clone(), &loc.share, &sub)
                    .await
                    .map_err(CatalogDbError::from)
            }
        }
    }

    pub fn book_path(&self, rel: &str) -> String {
        if rel.is_empty() {
            return String::new();
        }
        if rel.starts_with("smb://") || rel.starts_with("http://") || rel.starts_with("https://") {
            return rel.to_string();
        }
        if rel.starts_with('/') {
            return rel.to_string();
        }
        match self {
            Self::Local { dir } => {
                let base = dir.to_string_lossy();
                let base = base.trim_end_matches('/');
                if base.is_empty() {
                    rel.to_string()
                } else {
                    format!("{base}/{rel}")
                }
            }
            Self::Smb { loc, .. } => {
                let prefix = if loc.lib_root.is_empty() {
                    String::new()
                } else {
                    format!("{}/", loc.lib_root)
                };
                format!("smb://{}/{}/{prefix}{rel}", loc.host, loc.share)
            }
        }
    }
}

/// Open raw `metadata.db` bytes as an in-memory rusqlite connection.
/// (Swift: `sqlite3_deserialize` FREEONCLOSE. Rust: temp file + backup.)
pub fn open_memory_db(bytes: &[u8]) -> Result<Connection, CatalogDbError> {
    if bytes.is_empty() {
        return Err(CatalogDbError::NotFound {
            smb_path: String::new(),
            detail: "empty file".into(),
        });
    }
    if bytes.len() >= 512 * 1024 * 1024 {
        return Err(CatalogDbError::Corrupt(format!("unusable size {}", bytes.len())));
    }
    // Unique per call: parallel tests share a pid, so pid alone races.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catalog-meta-{}-{n}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, bytes)
        .map_err(|e| CatalogDbError::Corrupt(format!("temp write: {e}")))?;
    let src = Connection::open(&tmp).map_err(|e| CatalogDbError::Corrupt(format!("open: {e}")))?;
    let mut mem = Connection::open_in_memory()
        .map_err(|e| CatalogDbError::Corrupt(format!("memory open: {e}")))?;
    {
        let backup = rusqlite::backup::Backup::new(&src, &mut mem)
            .map_err(|e| CatalogDbError::Corrupt(format!("backup init: {e}")))?;
        backup
            .run_to_completion(100, std::time::Duration::from_millis(10), None)
            .map_err(|e| CatalogDbError::Corrupt(format!("backup: {e}")))?;
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(mem)
}

/// Read a local `metadata.db` file into memory (local-source branch).
pub fn read_local_db(dir: &str) -> Result<Vec<u8>, CatalogDbError> {
    let db = PathBuf::from(dir).join("metadata.db");
    let data = std::fs::read(&db).map_err(|e| CatalogDbError::NotFound {
        smb_path: format!("file://{}", db.display()),
        detail: e.to_string().chars().take(300).collect(),
    })?;
    if data.is_empty() {
        return Err(CatalogDbError::NotFound {
            smb_path: format!("file://{}", db.display()),
            detail: "empty file".into(),
        });
    }
    Ok(data)
}

// ---------------------------------------------------------------------------
// Queries (mirror SmbCatalogDB methods)
// ---------------------------------------------------------------------------

pub fn fetch_books(
    db: &Connection,
    source: &FileSource,
    q: &str,
    sort_desc: bool,
    by_author: bool,
    by_date: bool,
) -> Result<Vec<CatalogBook>, CatalogDbError> {
    let mut sql = "SELECT b.id, b.title, b.author_sort, b.path, b.has_cover, b.series_index,
                (SELECT s.name FROM books_series_link bsl JOIN series s ON s.id = bsl.series WHERE bsl.book = b.id LIMIT 1) AS series,
                (SELECT group_concat(tg.name, ', ') FROM books_tags_link btl JOIN tags tg ON tg.id = btl.tag WHERE btl.book = b.id) AS tags
                FROM books b"
        .to_string();
    let mut args: Vec<String> = vec![];
    if !q.is_empty() {
        sql += " WHERE b.title LIKE ?1 OR b.author_sort LIKE ?2";
        args.push(format!("%{q}%"));
        args.push(format!("%{q}%"));
    }
    if by_date {
        sql += &format!(" ORDER BY b.timestamp {}", if sort_desc { "ASC" } else { "DESC" });
    } else if by_author {
        sql += &format!(" ORDER BY b.author_sort {}, b.title", if sort_desc { "DESC" } else { "ASC" });
    } else {
        sql += &format!(" ORDER BY b.sort {}", if sort_desc { "DESC" } else { "ASC" });
    }
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            let id: i64 = row.get(0)?;
            let title: String = col_string_or(row, 1, "?");
            let author: String = col_string_or(row, 2, "?");
            let rel: String = col_string_or(row, 3, "");
            let has_cover: i64 = row.get(4).unwrap_or(0);
            let series_index: f64 = row.get::<_, Option<f64>>(5).unwrap_or(None).unwrap_or(0.0);
            let series: Option<String> = col_string(row, 6);
            let tags_str: String = col_string_or(row, 7, "");
            let tags: Vec<String> = if tags_str.is_empty() {
                vec![]
            } else {
                tags_str.split(", ").map(str::to_string).collect()
            };
            Ok((id, title, author, rel, has_cover != 0, series, series_index as f32, tags))
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let mut out = vec![];
    for r in rows {
        let (id, title, author, rel, has_cover, series, series_index, tags) =
            r.map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
        out.push(CatalogBook {
            id,
            title,
            author,
            path: source.book_path(&rel),
            has_cover,
            cover_hash: None,
            series,
            series_index,
            tags,
        });
    }
    Ok(out)
}

pub fn fetch_count(db: &Connection, q: &str) -> Result<usize, CatalogDbError> {
    let mut sql = "SELECT count(*) FROM books".to_string();
    if !q.is_empty() {
        sql += " WHERE title LIKE ?1 OR author_sort LIKE ?2";
    }
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let n: i64 = if q.is_empty() {
        stmt.query_row([], |r| r.get(0))
            .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?
    } else {
        let like = format!("%{q}%");
        stmt.query_row([&like, &like], |r| r.get(0))
            .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?
    };
    Ok(n as usize)
}

pub fn all_tags(db: &Connection, sort_desc: bool) -> Result<Vec<TagSummary>, CatalogDbError> {
    let sql = format!(
        "SELECT t.id, t.name, COUNT(*) FROM tags t JOIN books_tags_link btl ON t.id=btl.tag GROUP BY t.id ORDER BY t.name {}",
        if sort_desc { "DESC" } else { "ASC" }
    );
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TagSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                book_count: row.get::<_, i64>(2)? as usize,
            })
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))
}

pub fn all_authors(
    db: &Connection,
    source: &FileSource,
    sort_desc: bool,
) -> Result<Vec<AuthorSummary>, CatalogDbError> {
    let sql = format!(
        "SELECT a.id, a.name, a.sort, COUNT(*) AS book_count,
                (SELECT b.path FROM books b JOIN books_authors_link bal2 ON b.id = bal2.book
                 WHERE bal2.author = a.id ORDER BY b.sort LIMIT 1) AS first_book_path
         FROM authors a JOIN books_authors_link bal ON a.id = bal.author
         GROUP BY a.id ORDER BY a.sort {}",
        if sort_desc { "DESC" } else { "ASC" }
    );
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let rel: Option<String> = row.get(4).unwrap_or(None);
            Ok(AuthorSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                sort: col_string_or(row, 2, ""),
                book_count: row.get::<_, i64>(3)? as usize,
                first_book_path: rel.map(|r| source.book_path(&r)),
            })
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))
}

pub fn books_by_author(
    db: &Connection,
    source: &FileSource,
    author_id: i64,
) -> Result<Vec<AuthorBook>, CatalogDbError> {
    let sql = "SELECT b.id, b.title,
               (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
               (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
               b.path, b.timestamp, b.series_index,
               (SELECT s.name FROM books_series_link bsl JOIN series s ON s.id = bsl.series WHERE bsl.book = b.id LIMIT 1) AS series,
               (SELECT group_concat(tg.name, ', ') FROM books_tags_link btl JOIN tags tg ON tg.id = btl.tag WHERE btl.book = b.id) AS tags
               FROM books b JOIN books_authors_link bal ON b.id = bal.book
               WHERE bal.author = ?1 ORDER BY b.sort";
    author_books_query(db, source, sql, author_id)
}

pub fn books_by_series(
    db: &Connection,
    source: &FileSource,
    series_id: i64,
) -> Result<Vec<AuthorBook>, CatalogDbError> {
    let sql = "SELECT b.id, b.title,
               (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
               (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
               b.path, b.timestamp, b.series_index,
               (SELECT s.name FROM books_series_link bsl JOIN series s ON s.id = bsl.series WHERE bsl.book = b.id LIMIT 1) AS series,
               (SELECT group_concat(tg.name, ', ') FROM books_tags_link btl JOIN tags tg ON tg.id = btl.tag WHERE btl.book = b.id) AS tags
               FROM books b JOIN books_series_link bsl ON b.id = bsl.book
               WHERE bsl.series = ?1 ORDER BY b.series_index";
    author_books_query(db, source, sql, series_id)
}

fn author_books_query(
    db: &Connection,
    source: &FileSource,
    sql: &str,
    id: i64,
) -> Result<Vec<AuthorBook>, CatalogDbError> {
    let mut stmt = db.prepare(sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let rows = stmt
        .query_map([id], |row| {
            let rel: String = col_string_or(row, 4, "");
            let series_index: f64 = row.get::<_, Option<f64>>(6).unwrap_or(None).unwrap_or(0.0);
            let series: Option<String> = col_string(row, 7);
            let tags_str: String = col_string_or(row, 8, "");
            let tags: Vec<String> = if tags_str.is_empty() {
                vec![]
            } else {
                tags_str.split(", ").map(str::to_string).collect()
            };
            Ok(AuthorBook {
                id: row.get(0)?,
                title: row.get(1)?,
                author: col_string_or(row, 2, "Unknown"),
                path: source.book_path(&rel),
                cover_hash: String::new(),
                timestamp: col_string_or(row, 5, ""),
                root_folder: String::new(),
                author_sort: col_string_or(row, 3, ""),
                series,
                series_index: series_index as f32,
                tags,
            })
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))
}

pub fn all_series(
    db: &Connection,
    source: &FileSource,
    sort_desc: bool,
    by_author: bool,
) -> Result<Vec<SeriesSummary>, CatalogDbError> {
    let order = if by_author {
        format!(
            "author_sort COLLATE NOCASE ASC, s.sort COLLATE NOCASE {}",
            if sort_desc { "DESC" } else { "ASC" }
        )
    } else {
        format!("s.sort {}", if sort_desc { "DESC" } else { "ASC" })
    };
    let sql = format!(
        "SELECT s.id, s.name, COUNT(*) AS book_count,
                (SELECT b.path FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                 WHERE bsl2.series = s.id ORDER BY b.sort LIMIT 1) AS first_path,
                (SELECT a.sort FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                 JOIN books_authors_link bal ON bal.book = b.id JOIN authors a ON bal.author = a.id
                 WHERE bsl2.series = s.id ORDER BY b.series_index LIMIT 1) AS author_sort
         FROM series s JOIN books_series_link bsl ON s.id = bsl.series
         GROUP BY s.id ORDER BY {order}"
    );
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let rel: Option<String> = row.get(3).unwrap_or(None);
            Ok(SeriesSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                book_count: row.get::<_, i64>(2)? as usize,
                first_book_path: rel.map(|r| source.book_path(&r)),
                author: col_string(row, 4),
            })
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))
}

#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub query: String,
    pub title: String,
    pub author: String,
    pub series: String,
    pub tag: String,
    pub publisher: String,
    pub date_from: String,
    pub date_to: String,
    pub sort_descending: bool,
}

pub fn search_books(
    db: &Connection,
    source: &FileSource,
    p: &SearchParams,
) -> Result<Vec<SearchedBook>, CatalogDbError> {
    let mut conditions: Vec<String> = vec![];
    let mut bindings: Vec<String> = vec![];
    fn add_like(conditions: &mut Vec<String>, bindings: &mut Vec<String>, field: &str, value: &str) {
        if !value.is_empty() {
            conditions.push(format!("{field} LIKE ?"));
            bindings.push(format!("%{value}%"));
        }
    }
    if !p.query.is_empty() {
        let q = p.query.clone();
        if let Some(ci) = q.find(':') {
            let field = &q[..ci];
            let value = q[ci + 1..].trim();
            match field {
                "author" => {
                    conditions.push("(a.name LIKE ? OR a.sort LIKE ?)".into());
                    bindings.push(format!("%{value}%"));
                    bindings.push(format!("%{value}%"));
                }
                "title" => add_like(&mut conditions, &mut bindings, "b.title", value),
                "series" => add_like(&mut conditions, &mut bindings, "s.name", value),
                "tag" => add_like(&mut conditions, &mut bindings, "tg.name", value),
                "publisher" => add_like(&mut conditions, &mut bindings, "pub.name", value),
                "comments" => add_like(&mut conditions, &mut bindings, "c.text", value),
                _ => {
                    let like = format!("%{q}%");
                    conditions.push("(b.title LIKE ? OR a.name LIKE ? OR a.sort LIKE ? OR s.name LIKE ? OR tg.name LIKE ? OR pub.name LIKE ? OR c.text LIKE ?)".into());
                    for _ in 0..7 {
                        bindings.push(like.clone());
                    }
                }
            }
        } else {
            let like = format!("%{q}%");
            conditions.push("(b.title LIKE ? OR a.name LIKE ? OR a.sort LIKE ? OR s.name LIKE ? OR tg.name LIKE ? OR pub.name LIKE ? OR c.text LIKE ?)".into());
            for _ in 0..7 {
                bindings.push(like.clone());
            }
        }
    }
    add_like(&mut conditions, &mut bindings, "b.title", &p.title);
    if !p.author.is_empty() {
        conditions.push("(a.name LIKE ? OR a.sort LIKE ?)".into());
        bindings.push(format!("%{}%", p.author));
        bindings.push(format!("%{}%", p.author));
    }
    add_like(&mut conditions, &mut bindings, "s.name", &p.series);
    add_like(&mut conditions, &mut bindings, "tg.name", &p.tag);
    add_like(&mut conditions, &mut bindings, "pub.name", &p.publisher);
    if !p.date_from.is_empty() {
        conditions.push("strftime('%Y', b.pubdate) >= ?".into());
        bindings.push(p.date_from.clone());
    }
    if !p.date_to.is_empty() {
        conditions.push("strftime('%Y', b.pubdate) <= ?".into());
        bindings.push(p.date_to.clone());
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    // Series-scoped search fills the grid with one series: order by
    // index (like the series-tile drill), not title sort.
    let order = if p.series.is_empty() {
        format!("b.sort {}", if p.sort_descending { "DESC" } else { "ASC" })
    } else {
        format!(
            "s.name {}, b.series_index {}",
            if p.sort_descending { "DESC" } else { "ASC" },
            if p.sort_descending { "DESC" } else { "ASC" }
        )
    };
    let sql = format!(
        "SELECT DISTINCT b.id, b.title, a.name, b.path, s.name, b.series_index,
                (SELECT GROUP_CONCAT(t2.name, ', ') FROM books_tags_link btl2 JOIN tags t2 ON t2.id = btl2.tag WHERE btl2.book = b.id) AS tags,
                a.sort AS author_sort
         FROM books b
         JOIN books_authors_link bal ON bal.book = b.id
         JOIN authors a ON a.id = bal.author
         LEFT JOIN books_series_link bsl ON bsl.book = b.id
         LEFT JOIN series s ON s.id = bsl.series
         LEFT JOIN books_tags_link btl ON btl.book = b.id
         LEFT JOIN tags tg ON tg.id = btl.tag
         LEFT JOIN books_publishers_link bpl ON bpl.book = b.id
         LEFT JOIN publishers pub ON pub.id = bpl.publisher
         LEFT JOIN comments c ON b.id = c.book
         {where_clause}
         ORDER BY {order}"
    );
    let mut stmt = db.prepare(&sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let params: Vec<&dyn rusqlite::ToSql> =
        bindings.iter().map(|b| b as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            let rel: String = col_string_or(row, 3, "");
            let series_index: f64 = row.get::<_, Option<f64>>(5).unwrap_or(None).unwrap_or(0.0);
            let tags_str: String = col_string_or(row, 6, "");
            Ok(SearchedBook {
                id: row.get(0)?,
                title: row.get(1)?,
                author: col_string_or(row, 2, "Unknown"),
                path: source.book_path(&rel),
                series: col_string(row, 4),
                series_index: series_index as f32,
                tags: if tags_str.is_empty() {
                    vec![]
                } else {
                    tags_str.split(", ").map(str::to_string).collect()
                },
                cover_hash: String::new(),
                author_sort: col_string_or(row, 7, ""),
            })
        })
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))
}

pub fn book_detail(db: &Connection, book_id: i64) -> Result<Option<BookDetail>, CatalogDbError> {
    let sql = "SELECT b.title, a.name, s.name, b.series_index, c.text,
               (SELECT group_concat(tg.name, ', ') FROM books_tags_link btl
                JOIN tags tg ON btl.tag = tg.id WHERE btl.book = b.id) AS tags,
               (SELECT pub.name FROM books_publishers_link bpl
                JOIN publishers pub ON pub.id = bpl.publisher WHERE bpl.book = b.id LIMIT 1) AS publisher,
               (SELECT val FROM identifiers WHERE book = b.id AND type = 'isbn' LIMIT 1) AS isbn,
               b.pubdate, b.timestamp, a.sort
               FROM books b
               JOIN books_authors_link bal ON b.id = bal.book
               JOIN authors a ON bal.author = a.id
               LEFT JOIN books_series_link bsl ON b.id = bsl.book
               LEFT JOIN series s ON bsl.series = s.id
               LEFT JOIN comments c ON b.id = c.book
               WHERE b.id = ?1 LIMIT 1";
    let mut stmt = db.prepare(sql).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let mut rows = stmt
        .query([book_id])
        .map_err(|e| CatalogDbError::Corrupt(e.to_string()))?;
    let Some(row) = rows.next().map_err(|e| CatalogDbError::Corrupt(e.to_string()))? else {
        return Ok(None);
    };
    Ok(Some(BookDetail {
        id: book_id,
        title: row.get(0).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?,
        author: row.get(1).map_err(|e| CatalogDbError::Corrupt(e.to_string()))?,
        series: col_string(row, 2),
        series_index: row.get::<_, Option<f64>>(3).unwrap_or(None).unwrap_or(0.0) as f32,
        comments: col_string(row, 4).map(|c| strip_calibre_html(&c)),
        tags: col_string(row, 5),
        publisher: col_string(row, 6),
        isbn: col_string(row, 7),
        pubdate: col_string(row, 8),
        timestamp: col_string_or(row, 9, ""),
        author_sort: col_string_or(row, 10, ""),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT, author_sort TEXT, path TEXT, has_cover INTEGER, sort TEXT, timestamp TEXT, pubdate TEXT, series_index REAL, last_modified TEXT);
             CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT, sort TEXT);
             CREATE TABLE books_authors_link(book INTEGER, author INTEGER);
             CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE books_tags_link(book INTEGER, tag INTEGER);
             CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT, sort TEXT);
             CREATE TABLE books_series_link(book INTEGER, series INTEGER);
             CREATE TABLE books_publishers_link(book INTEGER, publisher INTEGER);
             CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE comments(book INTEGER, text TEXT);
             CREATE TABLE identifiers(book INTEGER, type TEXT, val TEXT);
             INSERT INTO books VALUES(1,'Emma','Austen, Jane','Jane Austen/Emma (1)',1,'Emma','2024-01-01','2024-01-01',1.0,'2024-01-01');
             INSERT INTO authors VALUES(1,'Jane Austen','Austen, Jane');
             INSERT INTO books_authors_link VALUES(1,1);",
        )
        .unwrap();
        db
    }

    #[test]
    fn fetch_and_count_mirror_swift() {
        let db = fixture_db();
        let t = FileSource::Local { dir: "/books".into() };
        assert_eq!(fetch_count(&db, "").unwrap(), 1);
        assert_eq!(fetch_count(&db, "Emma").unwrap(), 1);
        assert_eq!(fetch_count(&db, "Bond").unwrap(), 0);
        let books = fetch_books(&db, &t, "", false, true, false).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].path, "/books/Jane Austen/Emma (1)");
    }

    #[test]
    fn html_strip_mirrors_swift() {
        assert_eq!(strip_calibre_html("<p>Hi &amp; bye</p>"), "Hi & bye");
    }
}
