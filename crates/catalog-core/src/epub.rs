//! Port of `CatalogCore/Services/EpubService.swift`.
//!
//! Regex-based OPF/NCX/nav parser over the `zip` crate (replaces
//! vendored ZIPFoundation). Covers the same fallbacks: meta cover →
//! properties=cover-image → id/href containing "cover"; NCX → nav →
//! spine fallback for chapters.

use crate::models::Chapter;
use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EpubMetadata {
    pub title: String,
    pub author: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f32>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub tags: Vec<String>,
    pub comment: Option<String>,
}

fn first_capture(re: &Regex, xml: &str) -> Option<String> {
    re.captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    // Non-greedy dotall so nested HTML (<p>Hi</p>) is captured, then
    // stripped by the caller — Swift's NSAttributedString path does the
    // same where its `([^<]+)` regex would otherwise return nil.
    let re = Regex::new(&format!(r"(?is)<{tag}[^>]*>(.*?)</{tag}>")).ok()?;
    first_capture(&re, xml)
}

fn strip_html_simple(html: &str) -> String {
    let re = Regex::new("<[^>]+>").unwrap();
    let mut s = re.replace_all(html, " ").to_string();
    for (e, r) in [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&nbsp;", " ")] {
        s = s.replace(e, r);
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

pub fn parse_opf_metadata(xml: &str) -> EpubMetadata {
    let title = extract_tag(xml, "dc:title")
        .or_else(|| extract_tag(xml, "title"))
        .unwrap_or_else(|| "Unknown".into());
    let author = extract_tag(xml, "dc:creator").or_else(|| extract_tag(xml, "creator"));
    let publisher = extract_tag(xml, "dc:publisher").or_else(|| extract_tag(xml, "publisher"));
    let series = Regex::new(r#"<meta[^>]*name="calibre:series"[^>]*content="([^"]+)""#)
        .ok()
        .and_then(|re| first_capture(&re, xml));
    let series_index = Regex::new(r#"<meta[^>]*name="calibre:series_index"[^>]*content="([^"]+)""#)
        .ok()
        .and_then(|re| first_capture(&re, xml))
        .and_then(|s| s.parse::<f32>().ok());
    let mut tags = vec![];
    if let Ok(re) = Regex::new(r"(?i)<dc:subject[^>]*>([^<]+)</dc:subject>") {
        for cap in re.captures_iter(xml) {
            let subject = cap[1].trim();
            for part in subject.split([';', ',', '|']) {
                let t = part.trim();
                if !t.is_empty() {
                    tags.push(t.to_string());
                }
            }
        }
    }
    let comment = extract_tag(xml, "dc:description")
        .or_else(|| extract_tag(xml, "description"))
        .map(|c| strip_html_simple(&c));
    let mut isbn = None;
    for pat in [
        r"(?i)<dc:identifier[^>]*>[^<]*(?:urn:)?isbn:([^<]+)</dc:identifier>",
        r#"(?i)<dc:identifier[^>]*id="[^"]*isbn[^"]*"[^>]*>([^<]+)</dc:identifier>"#,
    ] {
        if let Ok(re) = Regex::new(pat) {
            if let Some(v) = first_capture(&re, xml) {
                isbn = Some(v);
                break;
            }
        }
    }
    EpubMetadata { title, author, series, series_index, isbn, publisher, tags, comment }
}

pub fn parse_manifest(xml: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    // Pattern 0: id then href. Pattern 1: href then id (swapped).
    for (idx, pat) in [
        r#"<item[^>]*id="([^"]+)"[^>]*href="([^"]+)""#,
        r#"<item[^>]*href="([^"]+)"[^>]*id="([^"]+)""#,
    ]
    .iter()
    .enumerate()
    {
        let Ok(re) = Regex::new(pat) else { continue };
        let swap = idx == 1;
        for cap in re.captures_iter(xml) {
            let (id, href) = if swap {
                (cap[2].to_string(), cap[1].to_string())
            } else {
                (cap[1].to_string(), cap[2].to_string())
            };
            map.entry(id).or_insert(href);
        }
    }
    map
}

pub fn parse_spine(xml: &str) -> Vec<String> {
    Regex::new(r#"<itemref[^>]*idref="([^"]+)""#)
        .map(|re| re.captures_iter(xml).map(|c| c[1].to_string()).collect())
        .unwrap_or_default()
}

fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn find_opf_path(path: &Path) -> Option<String> {
    let data = read_zip_entry(path, "META-INF/container.xml")?;
    let xml = String::from_utf8_lossy(&data);
    Regex::new(r#"full-path="([^"]+)""#)
        .ok()?
        .captures(&xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn opf_base(opf: &str) -> String {
    match opf.rfind('/') {
        Some(i) => opf[..i].to_string(),
        None => String::new(),
    }
}

fn join_base(base: &str, href: &str) -> String {
    if base.is_empty() {
        href.to_string()
    } else {
        format!("{base}/{href}")
    }
}

pub fn parse_metadata_file(path: &Path) -> Option<EpubMetadata> {
    let opf = find_opf_path(path)?;
    let data = read_zip_entry(path, &opf)?;
    Some(parse_opf_metadata(&String::from_utf8_lossy(&data)))
}

pub fn parse_chapters_file(path: &Path) -> Vec<Chapter> {
    let Some(opf) = find_opf_path(path) else { return vec![] };
    let base = opf_base(&opf);
    let Some(opf_data) = read_zip_entry(path, &opf) else { return vec![] };
    let opf_xml = String::from_utf8_lossy(&opf_data).to_string();
    let spine = parse_spine(&opf_xml);
    let manifest = parse_manifest(&opf_xml);
    // NCX via spine toc id
    if let Some(toc_id) = Regex::new(r#"<spine[^>]*toc="([^"]+)""#)
        .ok()
        .and_then(|re| first_capture(&re, &opf_xml))
    {
        if let Some(href) = manifest.get(&toc_id) {
            let full = join_base(&base, href);
            if let Some(data) = read_zip_entry(path, &full) {
                let chapters = parse_ncx(&String::from_utf8_lossy(&data));
                if !chapters.is_empty() {
                    return chapters;
                }
            }
        }
    }
    // nav fallback
    for pat in [
        r#"<item[^>]*id="toc"[^>]*href="([^"]+)""#,
        r#"<item[^>]*id="nav"[^>]*href="([^"]+)""#,
        r#"<item[^>]*properties="nav"[^>]*href="([^"]+)""#,
    ] {
        if let Ok(re) = Regex::new(pat) {
            if let Some(nav_id) = first_capture(&re, &opf_xml) {
                if let Some(href) = manifest.get(&nav_id) {
                    let full = join_base(&base, href);
                    if let Some(data) = read_zip_entry(path, &full) {
                        let chapters = parse_nav_xhtml(&String::from_utf8_lossy(&data));
                        if !chapters.is_empty() {
                            return chapters;
                        }
                    }
                }
            }
        }
    }
    spine
        .iter()
        .enumerate()
        .map(|(i, idref)| {
            let href = manifest.get(idref).cloned().unwrap_or_else(|| format!("{idref}.xhtml"));
            let title = href
                .rsplit('/')
                .next()
                .unwrap_or(&href)
                .split('.')
                .next()
                .unwrap_or(&href)
                .replace('_', " ");
            Chapter { index: i, title, anchor: idref.clone() }
        })
        .collect()
}

fn parse_ncx(xml: &str) -> Vec<Chapter> {
    let Ok(re) = Regex::new(r#"(?s)<navPoint[^>]*>.*?<navLabel>.*?<text>([^<]+)</text>.*?</navLabel>.*?<content[^>]*src="([^"]+)""#) else {
        return vec![];
    };
    re.captures_iter(xml)
        .enumerate()
        .filter_map(|(i, c)| {
            Some(Chapter {
                index: i,
                title: c.get(1)?.as_str().to_string(),
                anchor: c.get(2)?.as_str().to_string(),
            })
        })
        .collect()
}

fn parse_nav_xhtml(xml: &str) -> Vec<Chapter> {
    let Ok(re) = Regex::new(r#"(?s)<a[^>]*href="([^"]+)"[^>]*>([^<]+)</a>"#) else {
        return vec![];
    };
    re.captures_iter(xml)
        .enumerate()
        .map(|(i, c)| Chapter {
            index: i,
            title: c[2].trim().to_string(),
            anchor: c[1].to_string(),
        })
        .collect()
}

/// Mirror `coverImageURL`: meta cover → cover-image → id/href contains cover.
pub fn cover_href(opf_xml: &str, manifest: &std::collections::HashMap<String, String>) -> Option<String> {
    let mut cover_id = Regex::new(r#"<meta[^>]*name="cover"[^>]*content="([^"]+)""#)
        .ok()
        .and_then(|re| first_capture(&re, opf_xml));
    if cover_id.is_none() {
        cover_id = Regex::new(r#"<item[^>]*id="([^"]+)"[^>]*properties="cover-image""#)
            .ok()
            .and_then(|re| first_capture(&re, opf_xml));
    }
    if cover_id.is_none() {
        for (id, href) in manifest {
            if id.to_ascii_lowercase().contains("cover") || href.to_ascii_lowercase().contains("cover") {
                cover_id = Some(id.clone());
                break;
            }
        }
    }
    cover_id.and_then(|id| manifest.get(&id).cloned())
}

pub fn extract_cover_bytes(path: &Path) -> Option<Vec<u8>> {
    let opf = find_opf_path(path)?;
    let base = opf_base(&opf);
    let opf_data = read_zip_entry(path, &opf)?;
    let opf_xml = String::from_utf8_lossy(&opf_data).to_string();
    let manifest = parse_manifest(&opf_xml);
    let href = cover_href(&opf_xml, &manifest)?;
    read_zip_entry(path, &join_base(&base, &href))
}

pub fn cache_dir_for(path: &Path) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("epub_{}", path.to_string_lossy().len()));
    dir
}

/// Import a zip archive into a library folder: extracts entries (zip-slip
/// safe via `enclosed_name`) under `<to_dir>/<stem>/`, returns placed
/// file paths. Used by `catalog-cli import-zip` and, later, the native
/// file-dialog import flows. Never touches metadata.db (read-only rule).
pub fn import_zip(zip_path: &Path, to_dir: &Path) -> Result<Vec<PathBuf>, crate::CoreError> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let stem = zip_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::CoreError::Other("zip file has no name".to_string()))?;
    let dest_root = to_dir.join(&stem);
    std::fs::create_dir_all(&dest_root)?;
    let mut placed = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(safe) = entry.enclosed_name() else {
            continue;
        };
        let dest = dest_root.join(safe);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
            placed.push(dest);
        }
    }
    placed.sort();
    Ok(placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opf_metadata_regex_parity() {
        let xml = r#"<package><metadata><dc:title>Emma</dc:title><dc:creator>Jane Austen</dc:creator><meta name="calibre:series" content="Classics"/><meta name="calibre:series_index" content="1.0"/><dc:subject>Fiction; Romance</dc:subject><dc:description><p>Hi</p></dc:description></metadata></package>"#;
        let m = parse_opf_metadata(xml);
        assert_eq!(m.title, "Emma");
        assert_eq!(m.author.as_deref(), Some("Jane Austen"));
        assert_eq!(m.series.as_deref(), Some("Classics"));
        assert_eq!(m.tags, vec!["Fiction", "Romance"]);
        assert_eq!(m.comment.as_deref(), Some("Hi"));
    }

    #[test]
    fn manifest_both_orders() {
        let xml = r#"<manifest><item id="c" href="cover.jpg"/><item href="ch1.xhtml" id="ch1"/></manifest>"#;
        let m = parse_manifest(xml);
        assert_eq!(m.get("c").map(String::as_str), Some("cover.jpg"));
        assert_eq!(m.get("ch1").map(String::as_str), Some("ch1.xhtml"));
    }
}
