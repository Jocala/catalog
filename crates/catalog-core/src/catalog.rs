//! Port of `ReaderCatalogApp.swift` (ContentView/CatalogStore/
//! SettingsView/BookInfoView) + `SearchFormSheet.swift` to Slint
//! (`ui/app-window.slint` owns layout, `main.rs` owns state sync).
//!
//! Catalog-only: grid/list of books, author/series/tag drill-in,
//! search sheet (query + title/author + series/tag dropdowns +
//! tag→series expand), settings (SMB/local source, Kobo IPs, theme),
//! book detail + Read-on-Kobo. Thumbnails resolve from the local
//! `covers/` cache; live SMB fetch is injected via
//! `jreader_smb` by the caller.

use crate::models::{AuthorBook, AuthorSummary, BrowseMode, CatalogBook, CatalogSortOrder, SearchResult, SeriesSummary, TagSummary};

#[derive(Debug, Clone, Default)]
pub struct SearchForm {
    pub query: String,
    pub title: String,
    pub author: String,
    pub series: String,
    pub tag: String,
    pub expand_series_on_tag: bool,
}

impl SearchForm {
    /// Mirrors `SearchFormSheet.hasAnyField`.
    pub fn has_any_field(&self) -> bool {
        !(self.query.is_empty()
            && self.title.is_empty()
            && self.author.is_empty()
            && self.series.is_empty()
            && self.tag.is_empty())
    }

    /// Mirrors the tag-expand fan-out condition.
    pub fn is_tag_expand(&self) -> bool {
        self.expand_series_on_tag
            && !self.tag.is_empty()
            && self.series.is_empty()
            && self.title.is_empty()
            && self.author.is_empty()
            && self.query.is_empty()
    }

    pub fn to_search_params(&self) -> crate::db::SearchParams {
        crate::db::SearchParams {
            query: self.query.trim().to_string(),
            title: self.title.clone(),
            author: self.author.clone(),
            series: self.series.clone(),
            tag: self.tag.clone(),
            publisher: String::new(),
            date_from: String::new(),
            date_to: String::new(),
            sort_descending: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

#[derive(Debug, Default)]
pub struct CatalogState {
    pub books: Vec<CatalogBook>,
    pub authors: Vec<AuthorSummary>,
    pub series: Vec<SeriesSummary>,
    pub tags: Vec<TagSummary>,
    pub total_count: usize,
    pub is_loading: bool,
    pub db_error: Option<String>,
    pub kobo_status: String,
    pub kobo_error: Option<String>,
    pub browse_mode: Option<BrowseMode>,
    pub sort_order: Option<CatalogSortOrder>,
    pub drilled_kind: Option<BrowseMode>,
    pub drilled_title: Option<String>,
    pub drilled_books: Vec<AuthorBook>,
    pub is_drilling: bool,
    pub search_results: Vec<SearchResult>,
    pub series_results: Vec<SeriesSummary>,
    pub series_results_tag: String,
    pub is_search_mode: bool,
    pub view_mode: ViewMode,
}

impl CatalogState {
    pub fn new() -> Self {
        Self {
            browse_mode: Some(BrowseMode::Books),
            sort_order: Some(CatalogSortOrder::Author),
            ..Default::default()
        }
    }

    pub fn mode_count_text(&self) -> String {
        if self.drilled_kind.is_some() {
            return format!("{} books", self.drilled_books.len());
        }
        match self.browse_mode {
            Some(BrowseMode::Author) => format!("{} authors", self.authors.len()),
            Some(BrowseMode::Series) => format!("{} series", self.series.len()),
            Some(BrowseMode::Tags) => format!("{} tags", self.tags.len()),
            _ => format!("{} books", self.total_count),
        }
    }

    /// Mirrors `availableSortOrders()` (no Current — no reading_progress).
    pub fn available_sort_orders(&self) -> Vec<CatalogSortOrder> {
        use BrowseMode as M;
        use CatalogSortOrder as O;
        if let Some(kind) = self.drilled_kind {
            return match kind {
                M::Author | M::Series => vec![O::Az, O::Za, O::Date, O::Oldest],
                M::Tags | M::Books => vec![O::Author, O::Az, O::Za, O::Date, O::Oldest],
            };
        }
        match self.browse_mode {
            Some(M::Author) => vec![O::Az, O::Za],
            Some(M::Series) => vec![O::Author, O::Az, O::Za],
            Some(M::Tags) => vec![O::Az, O::Za],
            _ => vec![O::Author, O::Az, O::Za, O::Date, O::Oldest],
        }
    }

    /// Mirrors `drilledBooksSorted()`.
    pub fn drilled_books_sorted(&self) -> Vec<AuthorBook> {
        let mut v = self.drilled_books.clone();
        match self.sort_order {
            Some(CatalogSortOrder::Az) => v.sort_by_key(|a| a.title.to_lowercase()),
            Some(CatalogSortOrder::Za) => v.sort_by_key(|b| std::cmp::Reverse(b.title.to_lowercase())),
            Some(CatalogSortOrder::Date) => v.sort_by(|a, b| {
                if a.timestamp != b.timestamp {
                    if a.timestamp.is_empty() {
                        return std::cmp::Ordering::Greater;
                    }
                    if b.timestamp.is_empty() {
                        return std::cmp::Ordering::Less;
                    }
                    return b.timestamp.cmp(&a.timestamp);
                }
                a.title.to_lowercase().cmp(&b.title.to_lowercase())
            }),
            Some(CatalogSortOrder::Oldest) => v.sort_by(|a, b| {
                if a.timestamp != b.timestamp {
                    if a.timestamp.is_empty() {
                        return std::cmp::Ordering::Greater;
                    }
                    if b.timestamp.is_empty() {
                        return std::cmp::Ordering::Less;
                    }
                    return a.timestamp.cmp(&b.timestamp);
                }
                a.title.to_lowercase().cmp(&b.title.to_lowercase())
            }),
            _ => v.sort_by(|a, b| {
                let ka = if a.author_sort.is_empty() { &a.author } else { &a.author_sort };
                let kb = if b.author_sort.is_empty() { &b.author } else { &b.author_sort };
                ka.to_lowercase().cmp(&kb.to_lowercase())
            }),
        }
        v
    }

    pub fn enter_search_mode(&mut self, results: Vec<SearchResult>) {
        self.is_search_mode = true;
        self.search_results = results;
        self.series_results.clear();
    }

    pub fn enter_series_mode(&mut self, series: Vec<SeriesSummary>, tag: String) {
        self.is_search_mode = true;
        self.series_results = series;
        self.series_results_tag = tag;
        self.search_results.clear();
    }

    pub fn exit_search_mode(&mut self) {
        self.is_search_mode = false;
        self.search_results.clear();
        self.series_results.clear();
    }

    pub fn exit_drill(&mut self) {
        self.drilled_kind = None;
        self.drilled_title = None;
        self.drilled_books.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_form_tag_expand_parity() {
        let f = SearchForm { tag: "espionage".into(), expand_series_on_tag: true, ..Default::default() };
        assert!(f.has_any_field());
        assert!(f.is_tag_expand());
        let g = SearchForm { query: "bond".into(), tag: "espionage".into(), expand_series_on_tag: true, ..Default::default() };
        assert!(!g.is_tag_expand());
    }

    #[test]
    fn sort_orders_mirror_swift() {
        let mut s = CatalogState::new();
        s.browse_mode = Some(BrowseMode::Author);
        assert_eq!(s.available_sort_orders(), vec![CatalogSortOrder::Az, CatalogSortOrder::Za]);
    }
}
