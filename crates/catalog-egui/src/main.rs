//! Jocala Catalog — egui frontend over catalog-core.
//!
//! Functional + pretty, MIT/Apache only. All core I/O runs on worker
//! threads; the UI thread never blocks. Covers decode on workers and
//! upload as `TextureHandle`s with a 256-handle LRU.

use catalog_core::catalog::{CatalogState, SearchForm, ViewMode};
use catalog_core::db::{
    all_authors, all_series, all_tags, book_detail, books_by_author, books_by_series, fetch_books,
    fetch_count, open_memory_db, search_books, FileSource, SearchParams,
};
use catalog_core::models::{
    AuthorBook, BookDetail, BrowseMode, CatalogBook, CatalogSortOrder, SearchResult,
};
use catalog_core::settings::{ReaderLog, Settings};
use catalog_core::settings_draft::{badge_label, SettingsDraft};
use eframe::egui;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Cover fetch instrumentation (stderr): proves spawns stay bounded to
/// distinct ids instead of frames × visible cells.
static FETCH_SPAWNED: AtomicUsize = AtomicUsize::new(0);
static FETCH_DONE: AtomicUsize = AtomicUsize::new(0);

fn note_spawned(id: i64) {
    let n = FETCH_SPAWNED.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 10 || n.is_multiple_of(100) {
        eprintln!("[covers] fetch spawned n={n} id={id}");
    }
}

fn note_done() {
    let n = FETCH_DONE.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_multiple_of(100) {
        eprintln!(
            "[covers] fetch done n={n} spawned={}",
            FETCH_SPAWNED.load(Ordering::Relaxed)
        );
    }
}
use std::sync::{Arc, Mutex};

const MAX_TEX: usize = 256;
const CELL_W: f32 = 210.0;

// ---------------------------------------------------------------------------
// Theme.
// ---------------------------------------------------------------------------

fn styled(base: egui::Visuals, tune: impl FnOnce(&mut egui::Visuals)) -> egui::Style {
    let mut visuals = base;
    tune(&mut visuals);
    egui::Style { visuals, ..Default::default() }
}

fn apply_theme(ctx: &egui::Context, pref: i32) {
    use egui::{Theme, ThemePreference};
    let light = styled(egui::Visuals::light(), tune_light);
    let dark = styled(egui::Visuals::dark(), tune_dark);
    match pref {
        1 => {
            ctx.set_theme(ThemePreference::Light);
            ctx.set_style_of(Theme::Light, light);
        }
        2 => {
            ctx.set_theme(ThemePreference::Dark);
            ctx.set_style_of(Theme::Dark, dark);
        }
        _ => {
            ctx.set_theme(ThemePreference::System);
            ctx.set_style_of(Theme::Light, light);
            ctx.set_style_of(Theme::Dark, dark);
        }
    }
}

fn tune_light(v: &mut egui::Visuals) {
    let t = light_visuals();
    v.override_text_color = t.override_text_color;
    v.window_fill = t.window_fill;
    v.panel_fill = t.panel_fill;
    v.extreme_bg_color = t.extreme_bg_color;
    v.widgets.noninteractive.bg_fill = t.widgets.noninteractive.bg_fill;
    v.widgets.hovered.bg_fill = t.widgets.hovered.bg_fill;
    v.widgets.active.bg_fill = t.widgets.active.bg_fill;
    v.selection = t.selection;
    v.hyperlink_color = t.hyperlink_color;
    v.warn_fg_color = t.warn_fg_color;
}

fn tune_dark(v: &mut egui::Visuals) {
    let t = dark_visuals();
    v.window_fill = t.window_fill;
    v.panel_fill = t.panel_fill;
    v.extreme_bg_color = t.extreme_bg_color;
    v.widgets.active.bg_fill = t.widgets.active.bg_fill;
    v.selection = t.selection;
    v.hyperlink_color = t.hyperlink_color;
}

fn light_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    // Warm paper catalog: soft cream panels, deep-teal accent.
    let paper = egui::Color32::from_rgb(0xFA, 0xF7, 0xF1);
    let ink = egui::Color32::from_rgb(0x2B, 0x26, 0x20);
    let teal = egui::Color32::from_rgb(0x0E, 0x7C, 0x74);
    let amber = egui::Color32::from_rgb(0xB0, 0x6A, 0x1B);
    v.override_text_color = Some(ink);
    v.window_fill = paper;
    v.panel_fill = paper;
    v.extreme_bg_color = egui::Color32::from_rgb(0xF1, 0xEB, 0xDF);
    v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0xF3, 0xEE, 0xE3);
    v.selection.bg_fill = teal;
    v.selection.stroke.color = egui::Color32::WHITE;
    v.hyperlink_color = teal;
    v.warn_fg_color = amber;
    v.widgets.active.bg_fill = teal;
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(0xE4, 0xD9, 0xC6);
    v
}

fn dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    // Shelved-books-at-night: warm charcoal, amber accent.
    let coal = egui::Color32::from_rgb(0x1E, 0x1B, 0x17);
    let amber = egui::Color32::from_rgb(0xE0, 0xA0, 0x4E);
    let teal = egui::Color32::from_rgb(0x4E, 0xC2, 0xB5);
    v.window_fill = coal;
    v.panel_fill = coal;
    v.extreme_bg_color = egui::Color32::from_rgb(0x16, 0x14, 0x12);
    v.selection.bg_fill = amber;
    v.selection.stroke.color = coal;
    v.hyperlink_color = teal;
    v.widgets.active.bg_fill = amber;
    v
}

// ---------------------------------------------------------------------------
// State.
// ---------------------------------------------------------------------------

struct DetailView {
    book: CatalogBook,
    detail: Option<BookDetail>,
    loading: bool,
}

struct Core {
    state: CatalogState,
    settings: Settings,
    form: SearchForm,
    draft: Option<SettingsDraft>,
    /// Covers keyed by book PATH (stable across reloads; summaries carry
    /// paths but no ids, so tiles resolve art through the same cache).
    cover_bytes: HashMap<String, Vec<u8>>,
    cover_tex: HashMap<String, egui::TextureHandle>,
    cover_lru: VecDeque<String>,
    /// Paths with a fetch already running (the in-flight guard: one fetch
    /// per book, ever, until it resolves — no per-frame respawns).
    fetching: HashSet<String>,
    uploads_this_frame: u32,
    /// Drilled rows pre-sorted (sort runs once per drill / sort change,
    /// not per frame like `drilled_books_sorted` would).
    drilled_sorted: Vec<AuthorBook>,
    show_search: bool,
    show_settings: bool,
    detail: Option<DetailView>,
    tag_names: Vec<String>,
    series_names: Vec<String>,
    status: String,
}

impl Core {
    fn new() -> Self {
        ReaderLog::init();
        let settings = Settings::load();
        Self {
            state: CatalogState::new(),
            settings,
            form: SearchForm {
                expand_series_on_tag: true,
                ..Default::default()
            },
            draft: None,
            cover_bytes: HashMap::new(),
            cover_tex: HashMap::new(),
            cover_lru: VecDeque::new(),
            fetching: HashSet::new(),
            uploads_this_frame: 0,
            drilled_sorted: vec![],
            show_search: false,
            show_settings: false,
            detail: None,
            tag_names: vec![],
            series_names: vec![],
            status: String::new(),
        }
    }

    fn target(&self) -> Result<FileSource, String> {
        FileSource::from_settings(&self.settings).map_err(|e| e.to_string())
    }
}

struct App {
    core: Arc<Mutex<Core>>,
    started: bool,
}

/// Which row set a virtualized grid shows.
#[derive(Clone, Copy)]
enum RowSet {
    Books,
    Drilled,
    Search,
}

impl App {
    /// Name-row count for Author/Series/Tags (no vec clone).
    fn name_count(&self, mode: BrowseMode) -> usize {
        let c = self.core.lock().unwrap();
        match mode {
            BrowseMode::Author => c.state.authors.len(),
            BrowseMode::Series => c.state.series.len(),
            BrowseMode::Tags => c.state.tags.len(),
            BrowseMode::Books => 0,
        }
    }

    /// One name row: (id, name, book count, first-book path for art).
    fn name_row(&self, mode: BrowseMode, idx: usize) -> Option<(i64, String, usize, Option<String>)> {
        let c = self.core.lock().unwrap();
        match mode {
            BrowseMode::Author => c.state.authors.get(idx).map(|a| {
                (a.id, a.name.clone(), a.book_count, a.first_book_path.clone())
            }),
            BrowseMode::Series => c.state.series.get(idx).map(|s| {
                (s.id, s.name.clone(), s.book_count, s.first_book_path.clone())
            }),
            BrowseMode::Tags => c.state.tags.get(idx).map(|t| {
                (t.id, t.name.clone(), t.book_count, None)
            }),
            BrowseMode::Books => None,
        }
    }
    /// Scoped draft access: lock, apply, release. Never hold the guard
    /// across worker spawns or nested core locks.
    fn d_get<R: Default>(&self, f: impl FnOnce(&SettingsDraft) -> R) -> R {
        self.core.lock().unwrap().draft.as_ref().map(f).unwrap_or_default()
    }

    fn d_set(&self, f: impl FnOnce(&mut SettingsDraft)) {
        if let Some(d) = self.core.lock().unwrap().draft.as_mut() {
            f(d);
        }
    }

    fn spawn(&self, ctx: &egui::Context, job: impl FnOnce(Arc<Mutex<Core>>) + Send + 'static) {
        let ctx = ctx.clone();
        let core = self.core.clone();
        std::thread::spawn(move || {
            job(core);
            ctx.request_repaint();
        });
    }

    fn spawn_async(
        &self,
        ctx: &egui::Context,
        job: impl FnOnce(Arc<Mutex<Core>>, tokio::runtime::Runtime) + Send + 'static,
    ) {
        let ctx = ctx.clone();
        let core = self.core.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            job(core, rt);
            ctx.request_repaint();
        });
    }

    // -- cover texture cache (UI thread only) --

    /// Max new texture uploads per frame: keeps fast scrolling smooth
    /// while art streams in (placeholders hold the rest until later
    /// frames). Reset at the top of every `ui()`.
    const MAX_UPLOADS_PER_FRAME: u32 = 4;

    fn cover_texture(
        &self,
        ctx: &egui::Context,
        core: &mut Core,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some(h) = core.cover_tex.get(path) {
            return Some(h.clone());
        }
        if core.uploads_this_frame >= Self::MAX_UPLOADS_PER_FRAME {
            return None;
        }
        let bytes = core.cover_bytes.get(path)?.clone();
        if bytes.is_empty() {
            return None;
        }
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let px = img.into_raw();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
        let uri = format!("cover://{path}");
        let hdl = ctx.load_texture(uri, ci, egui::TextureOptions::LINEAR);
        core.uploads_this_frame += 1;
        if core.cover_tex.len() >= MAX_TEX {
            if let Some(old) = core.cover_lru.pop_front() {
                core.cover_tex.remove(&old);
                ctx.forget_image(&format!("cover://{old}"));
            }
        }
        core.cover_lru.push_back(path.to_string());
        core.cover_tex.insert(path.to_string(), hdl.clone());
        Some(hdl)
    }

    fn ensure_cover_bytes(&self, ctx: &egui::Context, id: i64, path: String) {
        // Check AND mark under one lock: without the in-flight guard this
        // respawns a thread per frame per uncached cell (thread storm).
        let should = {
            let mut c = self.core.lock().unwrap();
            !(c.cover_bytes.contains_key(&path) || !c.fetching.insert(path.clone()))
        };
        if !should {
            return;
        }
        note_spawned(id);
        self.spawn(ctx, move |core| {
            let source = {
                let c = core.lock().unwrap();
                match c.target() {
                    Ok(s) => s,
                    Err(_) => {
                        core.lock().unwrap().fetching.remove(&path);
                        return;
                    }
                }
            };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let fetched =
                rt.block_on(catalog_core::covers::fetch_cover_cached(&source, &path));
            let mut c = core.lock().unwrap();
            c.fetching.remove(&path);
            // Decode + downscale on the WORKER (detail size; the grid
            // downsamples further at upload). The UI thread only ever
            // decodes these small JPEGs. Empty vec = unusable cover,
            // cached as such so corrupt files don't retry forever.
            if let Some(raw) = fetched {
                let small = catalog_core::covers::scale_cover(&raw, 200, 260)
                    .unwrap_or_default();
                c.cover_bytes.insert(path, small);
            }
            drop(c);
            note_done();
        });
    }

    // -- catalog loads (always workers) --

    fn reload(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, rt| {
            let (settings, mode, order) = {
                let c = core.lock().unwrap();
                (
                    c.settings.clone(),
                    c.state.browse_mode.unwrap_or(BrowseMode::Books),
                    c.state.sort_order.unwrap_or(CatalogSortOrder::Author),
                )
            };
            eprintln!("[reload] start source={:?}", settings.library_source);
            let outcome: Result<(), String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let mut c = core.lock().unwrap();
                match mode {
                    BrowseMode::Books => {
                        c.state.books = fetch_books(
                            &db,
                            &source,
                            "",
                            order.is_descending(),
                            order.is_by_author(),
                            order.is_by_date(),
                        )
                        .map_err(|e| e.to_string())?;
                        c.state.total_count =
                            fetch_count(&db, "").map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Author => {
                        c.state.authors =
                            all_authors(&db, &source, order.is_descending())
                                .map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Series => {
                        c.state.series = all_series(
                            &db,
                            &source,
                            order.is_descending(),
                            order.is_by_author(),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Tags => {
                        c.state.tags = all_tags(&db, order.is_descending())
                            .map_err(|e| e.to_string())?;
                    }
                }
                c.state.db_error = None;
                c.status.clear();
                eprintln!(
                    "[reload] ok books={} authors={} series={} tags={} total={}",
                    c.state.books.len(),
                    c.state.authors.len(),
                    c.state.series.len(),
                    c.state.tags.len(),
                    c.state.total_count,
                );
                Ok(())
            });
            if let Err(e) = &outcome {
                eprintln!("[reload] err {e}");
                let mut c = core.lock().unwrap();
                c.state.db_error = Some(e.clone());
            }
        });
    }

    fn run_search(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, rt| {
            let (settings, form) = {
                let c = core.lock().unwrap();
                (c.settings.clone(), c.form.clone())
            };
            if !form.has_any_field() {
                return;
            }
            let outcome: Result<(), String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                // Tag→series fan-out (mirrors SearchFormSheet).
                if form.is_tag_expand() {
                    let tags = all_tags(&db, false).map_err(|e| e.to_string())?;
                    if let Some(tag) =
                        tags.iter().find(|t| t.name.to_lowercase() == form.tag.to_lowercase())
                    {
                        let p = SearchParams { tag: tag.name.clone(), ..Default::default() };
                        let found =
                            search_books(&db, &source, &p).map_err(|e| e.to_string())?;
                        let names: std::collections::HashSet<String> = found
                            .iter()
                            .filter_map(|b| b.series.clone())
                            .collect();
                        let all = all_series(&db, &source, false, false)
                            .map_err(|e| e.to_string())?;
                        let series: Vec<_> = all
                            .into_iter()
                            .filter(|s| names.contains(&s.name))
                            .collect();
                        let mut combined = vec![];
                        for s in &series {
                            for b in books_by_series(&db, &source, s.id)
                                .map_err(|e| e.to_string())?
                            {
                                combined.push(SearchResult {
                                    file_path: b.path.clone(),
                                    file_name: b
                                        .path
                                        .rsplit('/')
                                        .next()
                                        .unwrap_or(&b.path)
                                        .to_string(),
                                    book_id: Some(b.id),
                                    title: Some(b.title.clone()),
                                    author: Some(b.author.clone()),
                                    series: Some(s.name.clone()),
                                    tags: vec![],
                                    author_sort: b.author_sort.clone(),
                                });
                            }
                        }
                        let mut c = core.lock().unwrap();
                        c.state.is_search_mode = true;
                        c.state.search_results = combined;
                        c.state.series_results = series;
                        c.state.series_results_tag = tag.name.clone();
                        c.state.db_error = None;
                        return Ok(());
                    }
                }
                let found = search_books(&db, &source, &form.to_search_params())
                    .map_err(|e| e.to_string())?;
                let mut c = core.lock().unwrap();
                c.state.is_search_mode = true;
                c.state.search_results = found.into_iter().map(Into::into).collect();
                c.state.series_results = vec![];
                c.state.db_error = None;
                Ok(())
            });
            if let Err(e) = outcome {
                core.lock().unwrap().state.db_error = Some(e);
            }
        });
    }

    fn drill(&self, ctx: &egui::Context, mode: BrowseMode, title: String, id: i64) {
        self.spawn_async(ctx, move |core, rt| {
            let settings = core.lock().unwrap().settings.clone();
            let tag_name = if mode == BrowseMode::Tags {
                core.lock()
                    .unwrap()
                    .state
                    .tags
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.clone())
            } else {
                None
            };
            let outcome: Result<Vec<AuthorBook>, String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let books = match mode {
                    BrowseMode::Author => {
                        books_by_author(&db, &source, id).map_err(|e| e.to_string())?
                    }
                    BrowseMode::Series => {
                        books_by_series(&db, &source, id).map_err(|e| e.to_string())?
                    }
                    BrowseMode::Tags => {
                        let p = SearchParams { tag: tag_name.unwrap_or(title.clone()), ..Default::default() };
                        search_books(&db, &source, &p)
                            .map_err(|e| e.to_string())?
                            .into_iter()
                            .map(|s| AuthorBook {
                                id: s.id,
                                title: s.title,
                                author: s.author,
                                path: s.path,
                                cover_hash: String::new(),
                                timestamp: String::new(),
                                root_folder: String::new(),
                                author_sort: s.author_sort,
                            })
                            .collect()
                    }
                    BrowseMode::Books => vec![],
                };
                Ok(books)
            });
            let mut c = core.lock().unwrap();
            match outcome {
                Ok(books) => {
                    c.state.drilled_kind = Some(mode);
                    c.state.drilled_title = Some(title);
                    c.state.drilled_books = books;
                    c.drilled_sorted = c.state.drilled_books_sorted();
                    c.state.db_error = None;
                }
                Err(e) => c.state.db_error = Some(e),
            }
        });
    }

    fn open_detail(&self, ctx: &egui::Context, book: CatalogBook) {
        // Detail cover travels through the same byte cache (detail view
        // ensures it, texture upload happens on the UI thread).
        let for_worker = book.clone();
        self.spawn_async(ctx, move |core, rt| {
            let settings = core.lock().unwrap().settings.clone();
            let outcome = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let detail = book_detail(&db, for_worker.id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("no book id {}", for_worker.id))?;
                let cover = catalog_core::covers::fetch_cover_cached(&source, &for_worker.path)
                    .await;
                Ok::<_, String>((detail, cover))
            });
            let mut c = core.lock().unwrap();
            match outcome {
                Ok((detail, cover)) => {
                    // Same worker-side downscale as the grid path, so the
                    // UI thread never decodes full-size art.
                    if let Some(raw) = cover {
                        let small = catalog_core::covers::scale_cover(&raw, 200, 260)
                            .unwrap_or_default();
                        c.cover_bytes.insert(for_worker.path.clone(), small);
                        c.fetching.remove(&for_worker.path);
                    }
                    c.state.db_error = None;
                    c.detail = Some(DetailView {
                        book: for_worker,
                        detail: Some(detail),
                        loading: false,
                    });
                }
                Err(e) => c.state.db_error = Some(e),
            }
        });
        // Mark loading synchronously so the window opens at once.
        {
            let mut c = self.core.lock().unwrap();
            c.detail = Some(DetailView { book, detail: None, loading: true });
        }
    }

    fn read_on_kobo(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, _rt| {
            let (book, ip) = {
                let c = core.lock().unwrap();
                match &c.detail {
                    Some(d) => (
                        (d.book.title.clone(), d.book.author.clone()),
                        c.settings.kobo_ip.trim().to_string(),
                    ),
                    None => return,
                }
            };
            if ip.is_empty() {
                core.lock().unwrap().state.kobo_error =
                    Some("Kobo IP not set — enter it in Settings → Kobo".to_string());
                return;
            }
            core.lock().unwrap().state.kobo_status = format!("Opening “{}” on Kobo…", book.0);
            let out = Self::kobo_open_blocking(&book.0, &book.1, &ip);
            let mut c = core.lock().unwrap();
            if out.starts_with("Opened on Kobo:") {
                c.state.kobo_status = out;
            } else {
                c.state.kobo_error = Some(out.clone());
                c.state.kobo_status = "Kobo failed".to_string();
            }
        });
    }

    fn kobo_open_blocking(title: &str, author_sort: &str, ip: &str) -> String {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(Self::kobo_open(title, author_sort, ip))
    }

    async fn kobo_open(title: &str, author_sort: &str, ip: &str) -> String {
        use catalog_core::kobo as kobo;
        let probe = kobo::ssh_sync(ip, "echo ok", 5).await;
        if probe.code != 0 || !probe.output.to_lowercase().contains("ok") {
            let hint = if probe.output.trim().is_empty() {
                format!("Kobo is sleeping or unreachable at {ip} — press power button to wake, then try again.")
            } else {
                probe.output.clone()
            };
            return if hint.contains("sleeping") {
                hint
            } else {
                format!("Kobo is sleeping or unreachable at {ip} — press power button to wake.\n{hint}")
            };
        }
        let predicted =
            kobo::predicted_path(title, author_sort, Some(&kobo::natural_name(author_sort)));
        let esc = predicted.replace('\'', "'\\''");
        let check = kobo::ssh_sync(
            ip,
            &format!("if [ -f '{esc}' ]; then echo \"exists:{esc}\"; else echo \"missing\"; fi"),
            6,
        )
        .await;
        let chosen = if check.output.contains("exists:") {
            Some(predicted.clone())
        } else {
            let candidates = match kobo::cached_paths() {
                Some(c) => c,
                None => {
                    let list = kobo::ssh_sync(ip, "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort", 30).await;
                    if list.code != 0
                        && !(list.output.contains("/mnt/onboard/")
                            && list.output.contains(".epub"))
                    {
                        return format!("Failed to list books on Kobo: {}", list.output);
                    }
                    let paths: Vec<String> = list
                        .output
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(str::to_string)
                        .collect();
                    if paths.is_empty() {
                        return "No books found on Kobo (find returned empty) — is /mnt/onboard mounted?"
                            .to_string();
                    }
                    kobo::save_paths(&paths);
                    paths
                }
            };
            let (strict, title_only) = kobo::strict_match(&candidates, title, author_sort);
            if strict.len() == 1 {
                strict.into_iter().next().cloned()
            } else if strict.is_empty() && title_only.len() == 1 {
                title_only.into_iter().next().cloned()
            } else if strict.is_empty() && title_only.is_empty() {
                return format!("No match for: \"{title}\" (predicted path not found, strict search over {} books matched 0). Check Calibre Kobo template or re-sync Kobo.", candidates.len());
            } else {
                let n = if strict.is_empty() { title_only.len() } else { strict.len() };
                return format!(
                    "{n} matches for \"{title}\" — ambiguous, not opening. Refine title."
                );
            }
        };
        let Some(path) = chosen else {
            return "Internal error: ambiguous match".to_string();
        };
        let open = kobo::ssh_sync(ip, &kobo::koreader_open_cmd(&path), 10).await;
        if open.code != 0 {
            return format!("Failed to open on Kobo: {}", open.output);
        }
        format!("Opened on Kobo: {title}")
    }
}

// ---------------------------------------------------------------------------
// Views.
// ---------------------------------------------------------------------------

fn sort_label(o: &CatalogSortOrder) -> &'static str {
    o.as_str()
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        if !self.started {
            self.started = true;
            self.reload(ctx);
        }
        // Fresh upload budget every frame (see cover_texture cap).
        self.core.lock().unwrap().uploads_this_frame = 0;
        {
            let pref = self.core.lock().unwrap().settings.theme_preference;
            apply_theme(ctx, pref);
        }

        // -- errors banner --
        let (db_err, kobo_err) = {
            let c = self.core.lock().unwrap();
            (c.state.db_error.clone(), c.state.kobo_error.clone())
        };
        if let Some(e) = db_err {
            egui::Panel::top("errbar").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, "⚠ Calibre:");
                    ui.label(&e);
                    if ui.button("Retry").clicked() {
                        self.core.lock().unwrap().state.db_error = None;
                        self.reload(ctx);
                    }
                    if ui.button("Dismiss").clicked() {
                        self.core.lock().unwrap().state.db_error = None;
                    }
                });
            });
        }
        if let Some(e) = kobo_err {
            egui::Panel::top("kobobar").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::GOLD, "▤ Kobo:");
                    ui.label(&e);
                    if ui.button("Dismiss").clicked() {
                        self.core.lock().unwrap().state.kobo_error = None;
                    }
                });
            });
        }

        // -- toolbar --
        egui::Panel::top("toolbar").show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button("🔍 Search").clicked() {
                    // Preload tag/series options, then open.
                    self.preload_search_options(ctx);
                    self.core.lock().unwrap().show_search = true;
                }
                if ui.button("Reload").clicked() {
                    let mut c = self.core.lock().unwrap();
                    c.state.exit_search_mode();
                    c.state.exit_drill();
                    drop(c);
                    self.reload(ctx);
                }
                if ui.button("Settings").clicked() {
                    let mut c = self.core.lock().unwrap();
                    c.draft = Some(SettingsDraft::load(&c.settings));
                    c.show_settings = true;
                }
                ui.separator();
                let mut mode = self
                    .core
                    .lock()
                    .unwrap()
                    .state
                    .browse_mode
                    .unwrap_or(BrowseMode::Books);
                egui::ComboBox::from_label("Browse")
                    .selected_text(mode.as_str())
                    .show_ui(ui, |ui| {
                        for m in [
                            BrowseMode::Books,
                            BrowseMode::Author,
                            BrowseMode::Series,
                            BrowseMode::Tags,
                        ] {
                            ui.selectable_value(&mut mode, m, m.as_str());
                        }
                    });
                if Some(mode) != self.core.lock().unwrap().state.browse_mode {
                    let mut c = self.core.lock().unwrap();
                    c.state.browse_mode = Some(mode);
                    c.state.exit_drill();
                    c.state.exit_search_mode();
                    drop(c);
                    self.reload(ctx);
                }
                let avail = self.core.lock().unwrap().state.available_sort_orders();
                let mut order = self
                    .core
                    .lock()
                    .unwrap()
                    .state
                    .sort_order
                    .unwrap_or(CatalogSortOrder::Author);
                egui::ComboBox::from_label("Sort")
                    .selected_text(sort_label(&order))
                    .show_ui(ui, |ui| {
                        for o in &avail {
                            ui.selectable_value(&mut order, *o, sort_label(o));
                        }
                    });
                if Some(order) != self.core.lock().unwrap().state.sort_order {
                    // Drilled lists re-sort in memory (no re-query); root
                    // modes re-query with the new ORDER BY.
                    let drilled = {
                        let mut c = self.core.lock().unwrap();
                        c.state.sort_order = Some(order);
                        if c.state.drilled_kind.is_some() {
                            c.drilled_sorted = c.state.drilled_books_sorted();
                            true
                        } else {
                            false
                        }
                    };
                    if !drilled {
                        self.reload(ctx);
                    }
                }
                ui.separator();
                let mut vm = self.core.lock().unwrap().state.view_mode;
                ui.selectable_value(&mut vm, ViewMode::Grid, "Grid");
                ui.selectable_value(&mut vm, ViewMode::List, "List");
                self.core.lock().unwrap().state.view_mode = vm;
                let searching = self.core.lock().unwrap().state.is_search_mode;
                if searching {
                    ui.separator();
                    if ui.button("Library").clicked() {
                        self.core.lock().unwrap().state.exit_search_mode();
                    }
                    let n = self.core.lock().unwrap().state.search_results.len()
                        + self.core.lock().unwrap().state.series_results.len();
                    ui.label(format!("{n} results"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let t = self.core.lock().unwrap().state.mode_count_text();
                    ui.label(t);
                    let s = self.core.lock().unwrap().status.clone();
                    if !s.is_empty() {
                        ui.colored_label(egui::Color32::GOLD, s);
                    } else {
                        let k = self.core.lock().unwrap().state.kobo_status.clone();
                        if !k.is_empty() {
                            ui.label(k);
                        }
                    }
                });
            });
        });

        // -- central --
        egui::CentralPanel::default().show(root, |ui| {
            let searching = self.core.lock().unwrap().state.is_search_mode;
            let loading = self.core.lock().unwrap().state.books.is_empty()
                && self.core.lock().unwrap().state.authors.is_empty()
                && self.core.lock().unwrap().state.series.is_empty()
                && self.core.lock().unwrap().state.tags.is_empty()
                && self.core.lock().unwrap().state.db_error.is_none();
            if searching {
                self.search_results_ui(ctx, ui);
            } else if loading {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Loading…");
                });
            } else if self.core.lock().unwrap().state.drilled_kind.is_some() {
                self.drilled_ui(ctx, ui);
            } else {
                self.mode_ui(ctx, ui);
            }
        });

        self.search_window(ctx);
        self.settings_window(ctx);
        self.detail_window(ctx);
    }
}

impl App {
    fn preload_search_options(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, rt| {
            let settings = core.lock().unwrap().settings.clone();
            let outcome: Result<(Vec<String>, Vec<String>), String> =
                rt.block_on(async {
                    let source = FileSource::from_settings(&settings)
                        .map_err(|e| e.to_string())?;
                    let bytes =
                        source.read_db_bytes().await.map_err(|e| e.to_string())?;
                    let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                    let tags = all_tags(&db, false)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|t| t.name)
                        .collect();
                    let series = all_series(&db, &source, false, false)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|s| s.name)
                        .collect();
                    Ok((tags, series))
                });
            if let Ok((tags, series)) = outcome {
                let mut c = core.lock().unwrap();
                c.tag_names = tags;
                c.series_names = series;
            }
        });
    }

    /// Paint a cover texture aspect-fitted into `rect`, or a placeholder.
    /// Shared by grid cells, list rows, and the detail view.
    fn paint_cover(
        ui: &mut egui::Ui,
        rect: egui::Rect,
        tex: Option<egui::TextureHandle>,
    ) {
        match tex {
            Some(h) => {
                let [tw, th] = h.size();
                let s = (rect.width() / tw.max(1) as f32)
                    .min(rect.height() / th.max(1) as f32);
                let r = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(tw as f32 * s, th as f32 * s),
                );
                ui.painter().image(
                    h.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => {
                ui.painter().rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "▤",
                    egui::FontId::proportional(28.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }
    }

    fn book_cell(
        &self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        id: i64,
        path: &str,
        title: &str,
        author: &str,
    ) -> bool {
        // Returns true when tapped. The cover fetch fires ONLY for
        // visible cells: egui::Grid lays out every row each frame, so an
        // unconditional fetch would warm the whole library on first
        // paint (Swift's lazy loading is per-visible-cell).
        let mut tapped = false;
        ui.vertical(|ui| {
            ui.set_width(CELL_W);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(187.0, 240.0), egui::Sense::click());
            if ui.is_rect_visible(rect) {
                self.ensure_cover_bytes(ctx, id, path.to_string());
            }
            let tex = {
                let mut c = self.core.lock().unwrap();
                self.cover_texture(ctx, &mut c, path)
            };
            Self::paint_cover(ui, rect, tex);
            if resp.clicked() {
                tapped = true;
            }
            ui.label(egui::RichText::new(title).small().strong());
            ui.label(egui::RichText::new(author).small().weak());
        });
        tapped
    }

    /// One visible row's identity (id, path, title, author). Resolved per
    /// visible index under a short lock — ~200 bytes cloned, never the
    /// whole vec (full-vec clones per frame is what stalled scrolling).
    fn row_item(&self, set: RowSet, idx: usize) -> Option<(i64, String, String, String)> {
        let c = self.core.lock().unwrap();
        match set {
            RowSet::Books => c.state.books.get(idx).map(|b| {
                (b.id, b.path.clone(), b.title.clone(), b.author.clone())
            }),
            RowSet::Drilled => c.drilled_sorted.get(idx).map(|b| {
                (b.id, b.path.clone(), b.title.clone(), b.author.clone())
            }),
            RowSet::Search => c.state.search_results.get(idx).and_then(|r| {
                r.book_id.map(|id| {
                    (
                        id,
                        r.file_path.clone(),
                        r.title.clone().unwrap_or_else(|| r.file_name.clone()),
                        r.author.clone().unwrap_or_default(),
                    )
                })
            }),
        }
    }

    fn row_count(&self, set: RowSet) -> usize {
        let c = self.core.lock().unwrap();
        match set {
            RowSet::Books => c.state.books.len(),
            RowSet::Drilled => c.drilled_sorted.len(),
            RowSet::Search => c.state.search_results.len(),
        }
    }

    /// Fixed 4-across virtualized book grid. Only visible table rows
    /// lay out (Swift's lazy loading equivalent); an egui::Grid would
    /// lay out all N rows every frame and stall scrolling.
    fn book_table(&self, ctx: &egui::Context, ui: &mut egui::Ui, id: &str, set: RowSet) {
        use egui_extras::{Column, TableBuilder};
        let nrows = self.row_count(set).div_ceil(4);
        TableBuilder::new(ui)
            .id_salt(id)
            .column(Column::exact(212.0))
            .column(Column::exact(212.0))
            .column(Column::exact(212.0))
            .column(Column::exact(212.0))
            .body(|body| {
                body.rows(300.0, nrows, |mut row| {
                    let base = row.index() * 4;
                    for k in 0..4 {
                        row.col(|ui| {
                            if let Some((id, path, title, author)) =
                                self.row_item(set, base + k)
                            {
                                let book = CatalogBook {
                                    id,
                                    title: title.clone(),
                                    author: author.clone(),
                                    path: path.clone(),
                                    has_cover: true,
                                    cover_hash: None,
                                };
                                if self.book_cell(ctx, ui, id, &path, &title, &author) {
                                    self.open_book(ctx, book);
                                }
                            }
                        });
                    }
                });
            });
    }

    fn mode_ui(&self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mode = self.core.lock().unwrap().state.browse_mode.unwrap_or(BrowseMode::Books);
        match mode {
            BrowseMode::Books => {
                let n = self.core.lock().unwrap().state.books.len();
                if n == 0 {
                    self.empty_hint(ui, "No books");
                    return;
                }
                let grid = self.core.lock().unwrap().state.view_mode == ViewMode::Grid;
                if grid {
                    self.book_table(ctx, ui, "books", RowSet::Books);
                } else {
                    use egui_extras::{Column, TableBuilder};
                    TableBuilder::new(ui).column(Column::remainder()).body(|body| {
                        body.rows(70.0, n, |mut row| {
                            let idx = row.index();
                            // One short lock + one small row clone per
                            // VISIBLE row only.
                            let b = self
                                .core
                                .lock()
                                .unwrap()
                                .state
                                .books
                                .get(idx)
                                .cloned();
                            let Some(b) = b else { return };
                            row.col(|ui| {
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(40.0, 58.0),
                                        egui::Sense::hover(),
                                    );
                                    if ui.is_rect_visible(rect) {
                                        self.ensure_cover_bytes(ctx, b.id, b.path.clone());
                                    }
                                    let tex = {
                                        let mut c = self.core.lock().unwrap();
                                        self.cover_texture(ctx, &mut c, &b.path)
                                    };
                                    Self::paint_cover(ui, rect, tex);
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(&b.title).strong());
                                        ui.label(egui::RichText::new(&b.author).weak());
                                    });
                                    if ui.button("Details").clicked() {
                                        self.open_book(ctx, b.clone());
                                    }
                                });
                            });
                        });
                    });
                }
            }
            BrowseMode::Author => self.name_grid(ctx, ui, BrowseMode::Author),
            BrowseMode::Series => self.name_grid(ctx, ui, BrowseMode::Series),
            BrowseMode::Tags => self.name_grid(ctx, ui, BrowseMode::Tags),
        }
    }

    /// Author/series/tag browser: virtualized rows with first-book art
    /// (Swift tile parity — no more text-only tags). Rows resolve per
    /// visible index; drill on tap.
    /// Author/series/tag browser: virtualized rows with first-book art
    /// (Swift tile parity — no more text-only tags). Rows resolve per
    /// visible index; drill on tap.
    fn name_grid(&self, ctx: &egui::Context, ui: &mut egui::Ui, mode: BrowseMode) {
        use egui_extras::{Column, TableBuilder};
        let n = self.name_count(mode);
        if n == 0 {
            self.empty_hint(ui, "Nothing here yet");
            return;
        }
        TableBuilder::new(ui).column(Column::remainder()).body(|body| {
            body.rows(76.0, n, |mut row| {
                let idx = row.index();
                // (id, name, count, first-book path) — one small clone.
                let item = self.name_row(mode, idx);
                let Some((id, name, count, first_path)) = item else {
                    return;
                };
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(52.0, 68.0),
                            egui::Sense::click(),
                        );
                        let mut tapped = resp.clicked();
                        if ui.is_rect_visible(rect) {
                            if let Some(ref p) = first_path {
                                self.ensure_cover_bytes(ctx, id, p.clone());
                            }
                        }
                        let tex = first_path.as_ref().and_then(|p| {
                            let mut c = self.core.lock().unwrap();
                            self.cover_texture(ctx, &mut c, p)
                        });
                        Self::paint_cover(ui, rect, tex);
                        ui.vertical(|ui| {
                            if ui
                                .button(egui::RichText::new(&name).strong())
                                .clicked()
                            {
                                tapped = true;
                            }
                            ui.label(
                                egui::RichText::new(format!("{count} books")).weak(),
                            );
                        });
                        if tapped {
                            self.drill(ctx, mode, name.clone(), id);
                        }
                    });
                });
            });
        });
    }

    fn empty_hint(&self, ui: &mut egui::Ui, title: &str) {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.heading(title);
                ui.label("Set the SMB server and Calibre path in Settings → SMB");
                if ui.button("Open Settings").clicked() {
                    let mut c = self.core.lock().unwrap();
                    c.draft = Some(SettingsDraft::load(&c.settings));
                    c.show_settings = true;
                }
            });
        });
    }

    fn drilled_ui(&self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // Title + count only (the pre-sorted rows come from the cache).
        let (title, n) = {
            let c = self.core.lock().unwrap();
            (
                c.state.drilled_title.clone().unwrap_or_default(),
                c.drilled_sorted.len(),
            )
        };
        ui.horizontal(|ui| {
            if ui.button("‹ Back").clicked() {
                self.core.lock().unwrap().state.exit_drill();
            }
            ui.heading(&title);
            ui.label(format!("{n} books"));
        });
        ui.separator();
        self.book_table(ctx, ui, "drilled", RowSet::Drilled);
    }

    fn search_results_ui(&self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let (series, tag, results) = {
            let c = self.core.lock().unwrap();
            (
                c.state.series_results.clone(),
                c.state.series_results_tag.clone(),
                c.state.search_results.clone(),
            )
        };
        if !series.is_empty() {
            ui.label(format!("{} series with \"{tag}\"", series.len()));
            let cols = 4;
            egui::Grid::new("sseries").num_columns(cols).spacing([12.0, 12.0]).show(ui, |ui| {
                for (i, s) in series.iter().enumerate() {
                    if ui.button(format!("{}\n{} books", s.name, s.book_count)).clicked() {
                        let id = s.id;
                        let tag = tag.clone();
                        self.drill_series_to_books(ctx, id, &tag);
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
        }
        if results.is_empty() && series.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.heading("No Results");
                ui.label("No books found for your search");
            });
            return;
        }
        self.book_table(ctx, ui, "sresults", RowSet::Search);
    }

    fn drill_series_to_books(&self, ctx: &egui::Context, id: i64, tag: &str) {
        // Series tap inside search results flattens to books (Swift parity).
        let tag = tag.to_string();
        self.spawn_async(ctx, move |core, rt| {
            let settings = core.lock().unwrap().settings.clone();
            let outcome: Result<Vec<SearchResult>, String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let books =
                    books_by_series(&db, &source, id).map_err(|e| e.to_string())?;
                Ok(books
                    .into_iter()
                    .map(|b| SearchResult {
                        file_path: b.path.clone(),
                        file_name: b
                            .path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&b.path)
                            .to_string(),
                        book_id: Some(b.id),
                        title: Some(b.title.clone()),
                        author: Some(b.author.clone()),
                        series: Some(tag.to_string()),
                        tags: vec![],
                        author_sort: b.author_sort.clone(),
                    })
                    .collect())
            });
            let mut c = core.lock().unwrap();
            match outcome {
                Ok(results) => {
                    c.state.search_results = results;
                    c.state.series_results = vec![];
                }
                Err(e) => c.state.db_error = Some(e),
            }
        });
    }

    fn open_book(&self, ctx: &egui::Context, book: CatalogBook) {
        // Show the window immediately with a spinner; fill in async
        // (detail + cover travel together through the byte cache).
        {
            let mut c = self.core.lock().unwrap();
            c.detail = Some(DetailView { book: book.clone(), detail: None, loading: true });
        }
        self.ensure_cover_bytes(ctx, book.id, book.path.clone());
        self.open_detail(ctx, book);
    }

    fn search_window(&self, ctx: &egui::Context) {
        let mut open = self.core.lock().unwrap().show_search;
        egui::Window::new("Search Books").open(&mut open).show(ctx, |ui| {
            // Pull/push the form through the lock per widget (simple + safe).
            let mut q = self.core.lock().unwrap().form.query.clone();
            ui.horizontal(|ui| {
                ui.label("🔍");
                if ui.text_edit_singleline(&mut q).changed() {
                    self.core.lock().unwrap().form.query = q.clone();
                }
            });
            ui.separator();
            let mut t = self.core.lock().unwrap().form.title.clone();
            ui.horizontal(|ui| {
                ui.label("Title");
                if ui.text_edit_singleline(&mut t).changed() {
                    self.core.lock().unwrap().form.title = t.clone();
                }
            });
            let mut a = self.core.lock().unwrap().form.author.clone();
            ui.horizontal(|ui| {
                ui.label("Author");
                if ui.text_edit_singleline(&mut a).changed() {
                    self.core.lock().unwrap().form.author = a.clone();
                }
            });
            // Series / tag dropdowns from preloaded name lists.
            let series_opts = self.core.lock().unwrap().series_names.clone();
            let mut s = self.core.lock().unwrap().form.series.clone();
            ui.horizontal(|ui| {
                ui.label("Series");
                egui::ComboBox::from_id_salt("s_series")
                    .selected_text(if s.is_empty() { "Any" } else { s.as_str() })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s, String::new(), "Any");
                        for opt in &series_opts {
                            ui.selectable_value(&mut s, opt.clone(), opt);
                        }
                    });
                self.core.lock().unwrap().form.series = s.clone();
            });
            let tag_opts = self.core.lock().unwrap().tag_names.clone();
            let mut tg = self.core.lock().unwrap().form.tag.clone();
            ui.horizontal(|ui| {
                ui.label("Tag");
                egui::ComboBox::from_id_salt("s_tag")
                    .selected_text(if tg.is_empty() { "Any" } else { tg.as_str() })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut tg, String::new(), "Any");
                        for opt in &tag_opts {
                            ui.selectable_value(&mut tg, opt.clone(), opt);
                        }
                    });
                self.core.lock().unwrap().form.tag = tg.clone();
            });
            let mut exp = self.core.lock().unwrap().form.expand_series_on_tag;
            ui.checkbox(&mut exp, "Select series if any book matches tag");
            self.core.lock().unwrap().form.expand_series_on_tag = exp;
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.core.lock().unwrap().show_search = false;
                }
                let any = self.core.lock().unwrap().form.has_any_field();
                if ui.add_enabled(any, egui::Button::new("Search")).clicked() {
                    self.core.lock().unwrap().show_search = false;
                    self.run_search(ctx);
                }
            });
        });
        self.core.lock().unwrap().show_search = open;
    }

    fn settings_window(&self, ctx: &egui::Context) {
        let mut open = self.core.lock().unwrap().show_settings;
        egui::Window::new("Settings").open(&mut open).min_width(560.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // -- source --
                ui.heading("Library Source");
                let mut src = self.d_get(|d| d.library_source.clone());
                // Keep the local field authoritative when toggled.
                let src_was = src.clone();
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut src, "smb".to_string(), "SMB");
                    ui.selectable_value(&mut src, "local".to_string(), "Local");
                });
                if src != src_was {
                    let src2 = src.clone();
                    self.d_set(move |d| d.library_source = src2);
                }
                let local_on = src == "local";
                ui.add_enabled_ui(local_on, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Local Calibre Path");
                        let mut dir = self.d_get(|d| d.local_dir.clone());
                        if ui.text_edit_singleline(&mut dir).changed() {
                            self.d_set(|d| d.local_dir = dir.clone());
                        }
                        if ui.button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .set_title("Pick your Calibre library folder")
                                .pick_folder()
                            {
                                let p = p.display().to_string();
                                self.d_set(move |d| d.local_dir = p.clone());
                            }
                        }
                        if ui.button("Test").clicked() {
                            self.draft_test_local(ctx);
                        }
                        ui.label(badge_label(self.d_get(|d| d.local_test)));
                    });
                });
                ui.separator();
                // -- SMB --
                ui.heading("SMB");
                let smb_on = src != "local";
                ui.add_enabled_ui(smb_on, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("SMB Server");
                        let mut v = self.d_get(|d| d.smb_server.clone());
                        if ui.text_edit_singleline(&mut v).changed() {
                            self.d_set(|d| d.smb_server = v.clone());
                        }
                        if ui.button("Test SMB").clicked() {
                            self.draft_test_smb(ctx);
                        }
                        ui.label(badge_label(self.d_get(|d| d.smb_conn)));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Share");
                        let mut v = self.d_get(|d| d.smb_share.clone());
                        if ui.text_edit_singleline(&mut v).changed() {
                            self.d_set(|d| d.smb_share = v.clone());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("SMB Calibre Path");
                        let mut v = self.d_get(|d| d.calibre_path.clone());
                        if ui.text_edit_singleline(&mut v).changed() {
                            self.d_set(|d| d.calibre_path = v.clone());
                        }
                        if ui.button("Test Calibre").clicked() {
                            self.draft_test_db(ctx);
                        }
                        ui.label(badge_label(self.d_get(|d| d.smb_db)));
                    });
                    ui.horizontal(|ui| {
                        ui.label("User");
                        let mut v = self.d_get(|d| d.smb_user.clone());
                        if ui.text_edit_singleline(&mut v).changed() {
                            self.d_set(|d| d.smb_user = v.clone());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Password");
                        let mut v = self.d_get(|d| d.smb_pass.clone());
                        let mut show = self.d_get(|d| d.show_pass);
                        if show {
                            ui.text_edit_singleline(&mut v);
                        } else {
                            ui.add(egui::TextEdit::singleline(&mut v).password(true));
                        }
                        if ui.button(if show { "Hide" } else { "Show" }).clicked() {
                            show = !show;
                        }
                        self.d_set(|d| {
                            d.smb_pass = v.clone();
                            d.show_pass = show;
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Domain");
                        let mut v = self.d_get(|d| d.smb_domain.clone());
                        if ui.text_edit_singleline(&mut v).changed() {
                            self.d_set(|d| d.smb_domain = v.clone());
                        }
                    });
                });
                ui.separator();
                // -- Kobo --
                ui.heading("Kobo");
                let ips = self.d_get(|d| d.kobo_ips.clone());
                for ip in ips {
                    ui.horizontal(|ui| {
                        let is_def = self.d_get(|d| d.kobo_ip.clone()) == ip;
                        if ui.button(if is_def { "★" } else { "☆" }).clicked() {
                            self.draft_kobo_star(&ip);
                        }
                        ui.monospace(&ip);
                        if ui.button("Test").clicked() {
                            self.draft_kobo_test(ctx, ip.clone());
                        }
                        if ui.button("🗑").clicked() {
                            self.draft_kobo_remove(&ip);
                        }
                        let r = self.d_get(|d| {
                            d.kobo_results.get(&ip).copied().map(|p| p.label().to_string())
                        });
                        ui.label(r.unwrap_or_default());
                    });
                }
                ui.horizontal(|ui| {
                    let mut v = self.d_get(|d| d.new_kobo_ip.clone());
                    let enter = ui.text_edit_singleline(&mut v).lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    self.d_set(|d| d.new_kobo_ip = v.clone());
                    if enter || ui.button("Add").clicked() {
                        self.d_set(|d| {
                            d.new_kobo_ip = v.clone();
                            d.add_kobo();
                        });
                    }
                });
                ui.weak("Multiple Kobo IPs — star selects default for “Read on Kobo”. Ping tests awake.");
                ui.separator();
                // -- theme --
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    let mut t = self.d_get(|d| d.theme_preference);
                    ui.selectable_value(&mut t, 1, "Light");
                    ui.selectable_value(&mut t, 2, "Dark");
                    ui.selectable_value(&mut t, 0, "System");
                    self.d_set(move |d| d.theme_preference = t);
                });
                let st = self.d_get(|d| d.status.clone());
                if !st.is_empty() {
                    ui.label(&st);
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.core.lock().unwrap().show_settings = false;
                    }
                    if ui.button("Save").clicked() {
                        let ok = {
                            let mut c = self.core.lock().unwrap();
                            let mut draft = c.draft.take().unwrap_or_else(|| {
                                SettingsDraft::load(&c.settings)
                            });
                            let ok = draft.apply_to(&mut c.settings);
                            if ok {
                                let _ = c.settings.save();
                            }
                            c.draft = Some(draft);
                            ok
                        };
                        if ok {
                            self.core.lock().unwrap().show_settings = false;
                            self.reload(ctx);
                        }
                    }
                });
            });
        });
        self.core.lock().unwrap().show_settings = open;
    }

    // -- settings-draft bridges (keep dialog code readable) --

    fn draft_test_local(&self, ctx: &egui::Context) {
        self.spawn(ctx, |core| {
            let dir = core
                .lock()
                .unwrap()
                .draft
                .as_ref()
                .map(|d| d.clean_local())
                .unwrap_or_default();
            let (pf, status) = SettingsDraft::check_local(&dir);
            if let Some(d) = core.lock().unwrap().draft.as_mut() {
                d.local_test = pf;
                d.status = status;
            }
        });
    }

    fn draft_test_smb(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, rt| {
            let (conn, share) = {
                let c = core.lock().unwrap();
                let d = c.draft.as_ref().expect("draft open");
                (d.smb_conn(), d.smb_share.clone())
            };
            let (pf, status) = rt.block_on(SettingsDraft::check_smb_connection(&conn, &share));
            if let Some(d) = core.lock().unwrap().draft.as_mut() {
                d.smb_conn = pf;
                d.status = status;
            }
        });
    }

    fn draft_test_db(&self, ctx: &egui::Context) {
        self.spawn_async(ctx, |core, rt| {
            let (conn, share, lib) = {
                let c = core.lock().unwrap();
                let d = c.draft.as_ref().expect("draft open");
                (d.smb_conn(), d.smb_share.clone(), d.calibre_path.clone())
            };
            let (pf, status) =
                rt.block_on(SettingsDraft::check_smb_database(&conn, &share, &lib));
            if let Some(d) = core.lock().unwrap().draft.as_mut() {
                d.smb_db = pf;
                d.status = status;
            }
        });
    }

    fn draft_kobo_star(&self, ip: &str) {
        let mut c = self.core.lock().unwrap();
        let Some(mut dd) = c.draft.clone() else {
            return;
        };
        dd.kobo_ip = ip.to_string();
        dd.status = format!("Default Kobo: {ip}");
        dd.save_kobo_into(&mut c.settings);
        let _ = c.settings.save();
        c.draft = Some(dd);
    }

    fn draft_kobo_remove(&self, ip: &str) {
        let mut c = self.core.lock().unwrap();
        if let Some(d) = c.draft.as_mut() {
            d.remove_kobo(ip);
            let dd = d.clone();
            dd.save_kobo_into(&mut c.settings);
            let _ = c.settings.save();
        }
    }

    fn draft_kobo_test(&self, ctx: &egui::Context, ip: String) {
        self.spawn(ctx, move |core| {
            let (target, pf, status) = SettingsDraft::check_kobo(&ip);
            if !target.is_empty() {
                if let Some(d) = core.lock().unwrap().draft.as_mut() {
                    d.kobo_results.insert(target, pf);
                    d.status = status;
                }
            }
        });
    }

    fn detail_window(&self, ctx: &egui::Context) {
        let mut open = self.core.lock().unwrap().detail.is_some();
        if !open {
            return;
        }
        egui::Window::new("Book Details").open(&mut open).show(ctx, |ui| {
            let (title, author, detail, loading, id, path) = {
                let c = self.core.lock().unwrap();
                match &c.detail {
                    Some(d) => (
                        d.book.title.clone(),
                        d.book.author.clone(),
                        d.detail.clone(),
                        d.loading,
                        d.book.id,
                        d.book.path.clone(),
                    ),
                    None => return,
                }
            };
            ui.horizontal(|ui| {
                // Cover.
                let tex = {
                    let mut c = self.core.lock().unwrap();
                    self.cover_texture(ctx, &mut c, &path)
                };
                if tex.is_none() {
                    self.ensure_cover_bytes(ctx, id, path);
                }
                if let Some(h) = tex {
                    ui.add(egui::Image::new(&h).max_size(egui::vec2(160.0, 230.0)));
                } else {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(160.0, 230.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
                }
                ui.vertical(|ui| {
                    ui.heading(&title);
                    ui.label(&author);
                    if let Some(d) = &detail {
                        if let Some(s) = &d.series {
                            ui.label(format!("Series: {s} #{}", d.series_index));
                        }
                        if let Some(t) = &d.tags {
                            ui.label(t);
                        }
                        if let Some(p) = &d.publisher {
                            ui.label(format!("Publisher: {p}"));
                        }
                        if let Some(i) = &d.isbn {
                            ui.label(format!("ISBN: {i}"));
                        }
                    } else if loading {
                        ui.spinner();
                    }
                });
            });
            if let Some(d) = detail {
                if let Some(c) = d.comments {
                    if !c.is_empty() {
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| ui.label(c));
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("📖 Read on Kobo").clicked() {
                    self.read_on_kobo(ctx);
                }
                if ui.button("Close").clicked() {
                    self.core.lock().unwrap().detail = None;
                }
                let k = self.core.lock().unwrap().state.kobo_status.clone();
                if !k.is_empty() {
                    ui.label(k);
                }
            });
        });
        if !open {
            self.core.lock().unwrap().detail = None;
        }
    }
}

fn main() -> eframe::Result<()> {
    ReaderLog::init();
    // Fixed window, as approved: 900x858, non-resizable. The 4-across
    // grid is laid out to fit exactly this size.
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 858.0])
            .with_min_inner_size([900.0, 858.0])
            .with_max_inner_size([900.0, 858.0])
            .with_resizable(false),
        ..Default::default()
    };
        eframe::run_native(
        "Jocala Catalog",
        opts,
        Box::new(|_cc| {
            let app = App {
                core: Arc::new(Mutex::new(Core::new())),
                started: false,
            };
            Ok(Box::new(app))
        }),
    )
}
