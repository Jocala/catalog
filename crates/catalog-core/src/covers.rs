//! Port of `CatalogCore/Services/ThumbnailService.swift` (cover half).
//!
//! Pipeline: `covers/<sha256>.jpg` content cache → smb path handling
//! (derive `cover.jpg` sibling; the actual SMB bytes come from
//! `jreader_smb`) → http(s) cover.jpg → local folder/file cover.jpg →
//! EPUB-embedded cover. Scale-to-fit 120×180, never crop (Swift
//! `centerCropToThumbnail` is scale-to-fit despite the name).

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const THUMB_W: u32 = 120;
pub const THUMB_H: u32 = 180;

pub fn sha256_hex(data: &[u8]) -> String {
    hex_of(Sha256::digest(data))
}

fn hex_of(digest: sha2::digest::Output<Sha256>) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn cached_cover_path(covers_dir: &Path, hash: &str) -> PathBuf {
    covers_dir.join(format!("{hash}.jpg"))
}

pub fn cached_cover_for_hash(hash: &str) -> Option<PathBuf> {
    let p = cached_cover_path(&crate::settings::covers_dir(), hash);
    p.exists().then_some(p)
}

pub fn cache_bytes(data: &[u8]) -> Option<PathBuf> {
    if data.is_empty() {
        return None;
    }
    let hash = sha256_hex(data);
    let path = cached_cover_path(&crate::settings::covers_dir(), &hash);
    if !path.exists() {
        let _ = std::fs::write(&path, data);
    }
    Some(path)
}

/// Derive the `cover.jpg` sibling for a Calibre book path.
/// Mirrors the Swift rule: only full file paths (epub/pdf/kepub suffix)
/// take the sibling; folders (which may contain dots like
/// "01-03.winter.black (9329)") get `<folder>/cover.jpg`.
pub fn cover_sibling_for_book_path(book_path: &str) -> String {
    let lower = book_path.to_ascii_lowercase();
    if lower.ends_with(".epub") || lower.ends_with(".pdf") || lower.ends_with(".kepub") {
        match book_path.rfind('/') {
            Some(i) => format!("{}/cover.jpg", &book_path[..i]),
            None => format!("{book_path}/cover.jpg"),
        }
    } else {
        format!("{book_path}/cover.jpg")
    }
}

/// Parse an `smb://host/share/remote` URL into (host, share, remote_path).
pub fn parse_smb_url(path: &str) -> Option<(String, String, String)> {
    let rest = path.strip_prefix("smb://")?;
    let mut parts = rest.splitn(3, '/');
    Some((
        parts.next()?.to_string(),
        parts.next()?.to_string(),
        parts.next().unwrap_or("").to_string(),
    ))
}

/// Local cover lookup: folder → `<folder>/cover.jpg`, file →
/// `<parent>/cover.jpg`, else EPUB-embedded via `jreader_epub`.
pub fn local_cover_bytes(path: &Path) -> Option<Vec<u8>> {
    let cover = if path.is_dir() {
        path.join("cover.jpg")
    } else {
        path.parent().unwrap_or(path).join("cover.jpg")
    };
    if let Ok(data) = std::fs::read(&cover) {
        if !data.is_empty() {
            return Some(data);
        }
    }
    if path.is_dir() {
        // First EPUB inside the folder (mirrors Swift folder branch).
        let entries = std::fs::read_dir(path).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e.eq_ignore_ascii_case("epub")).unwrap_or(false) {
                if let Some(bytes) = crate::epub::extract_cover_bytes(&p) {
                    return Some(bytes);
                }
            }
        }
        return None;
    }
    if path.extension().map(|e| e.eq_ignore_ascii_case("epub")).unwrap_or(false) {
        return crate::epub::extract_cover_bytes(path);
    }
    None
}

/// Scale-to-fit inside 120×180 (never upscale, never crop).
pub fn scale_to_fit(data: &[u8]) -> Option<Vec<u8>> {
    scale_cover(data, THUMB_W, THUMB_H)
}

/// Parameterized scale-to-fit (table thumbnails, detail art, …).
/// Never upscales, never crops; returns JPEG bytes.
pub fn scale_cover(data: &[u8], max_w: u32, max_h: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    if w <= 0.0 || h <= 0.0 || max_w == 0 || max_h == 0 {
        return None;
    }
    let scale = (max_w as f32 / w).min(max_h as f32 / h);
    if scale >= 1.0 {
        return Some(data.to_vec());
    }
    let (nw, nh) = ((w * scale) as u32, (h * scale) as u32);
    let thumb = img.resize(nw.max(1), nh.max(1), image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(out.into_inner())
}

/// Disk cache keyed by book path (stable across launches, unlike the
/// content-hash cache — the DB carries no cover hash to look up by).
pub fn path_cache_key(book_path: &str) -> String {
    sha256_hex(book_path.as_bytes())
}

pub fn cached_book_cover(book_path: &str) -> Option<Vec<u8>> {
    let p = cached_cover_path(&crate::settings::covers_dir(), &path_cache_key(book_path));
    let data = std::fs::read(p).ok()?;
    (!data.is_empty()).then_some(data)
}

pub fn store_book_cover(book_path: &str, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let path = cached_cover_path(&crate::settings::covers_dir(), &path_cache_key(book_path));
    if !path.exists() {
        let _ = std::fs::write(path, data);
    }
}

/// Single entry point for UI cover loads: disk cache first, then the
/// live source (local folder/EPUB or SMB `cover.jpg` sibling), caching
/// hits for next time. Returns full-size JPEG bytes; callers scale.
pub async fn fetch_cover_cached(
    source: &crate::db::FileSource,
    book_path: &str,
) -> Option<Vec<u8>> {
    if book_path.is_empty() {
        return None;
    }
    if let Some(hit) = cached_book_cover(book_path) {
        return Some(hit);
    }
    let data = match source {
        crate::db::FileSource::Local { .. } => local_cover_bytes(Path::new(book_path)),
        crate::db::FileSource::Smb { loc, conn } => {
            let sib = cover_sibling_for_book_path(book_path);
            let (_, _, remote) = parse_smb_url(&sib)?;
            // Sanity: the sibling must live under the same share.
            let (h, s, _) = parse_smb_url(book_path).unwrap_or_default();
            if h != loc.host || s != loc.share {
                return None;
            }
            crate::smb::download_file_bytes(conn.clone(), &loc.share, &remote)
                .await
                .ok()
        }
    };
    let data = data.filter(|d| !d.is_empty())?;
    store_book_cover(book_path, &data);
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_rule_mirrors_swift() {
        assert_eq!(
            cover_sibling_for_book_path("Jane Austen/Emma (1)/Emma.epub"),
            "Jane Austen/Emma (1)/cover.jpg"
        );
        // Dotted folder is NOT treated as a file.
        assert_eq!(
            cover_sibling_for_book_path("Austen/01-03.winter.black (9329)"),
            "Austen/01-03.winter.black (9329)/cover.jpg"
        );
        assert_eq!(
            cover_sibling_for_book_path("smb://h/s/Author/Title (1)"),
            "smb://h/s/Author/Title (1)/cover.jpg"
        );
    }

    #[test]
    fn smb_url_split() {
        let (h, s, r) = parse_smb_url("smb://h/share/calibre/cover.jpg").unwrap();
        assert_eq!((h.as_str(), s.as_str(), r.as_str()), ("h", "share", "calibre/cover.jpg"));
    }

    #[test]
    fn local_cover_end_to_end() {
        use image::{ImageBuffer, Rgb};
        // Build a 100x150 red JPEG as a fake book cover.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(100, 150, |_, _| Rgb([200, 30, 30]));
        let mut jpg = std::io::Cursor::new(Vec::new());
        img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        let bytes = jpg.into_inner();
        assert!(!bytes.is_empty());

        let dir = std::env::temp_dir().join(format!(
            "catalog-cover-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let book = dir.join("Author").join("Title (1)");
        std::fs::create_dir_all(&book).unwrap();
        std::fs::write(book.join("cover.jpg"), &bytes).unwrap();

        // Local lookup finds the sibling cover.
        let found = local_cover_bytes(&book).expect("cover.jpg found");
        assert_eq!(found, bytes);

        // Scale fits inside the requested box.
        let thumb = scale_cover(&found, 44, 64).expect("scaled");
        let back = image::load_from_memory(&thumb).unwrap();
        assert!(back.width() <= 44 && back.height() <= 64);

        // Path-keyed cache round-trips (cleaned up afterwards so the
        // real covers dir is left untouched).
        let key = book.to_string_lossy().to_string();
        assert!(cached_book_cover(&key).is_none());
        store_book_cover(&key, &bytes);
        assert_eq!(cached_book_cover(&key).unwrap(), bytes);
        let _ = std::fs::remove_file(cached_cover_path(
            &crate::settings::covers_dir(),
            &path_cache_key(&key),
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
