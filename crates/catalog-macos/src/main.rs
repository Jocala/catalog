//! Jocala Catalog — native macOS UI (AppKit via objc2) over catalog-core.
//!
//! Catalog browser: toolbar (search, reload, settings, open, mode, sort,
//! back, status), book table (double-click opens detail or drills in),
//! detail window (Read on Kobo), settings window (source, SMB, Kobo,
//! theme), File menu (Open…, Reload).
//!
//! Rules: catalog-core stays GUI-free; every core call runs on a worker
//! thread and marshals back via `dispatch2::run_on_main` — the main
//! thread never blocks. Object creation uses `msg_send!` directly.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("catalog-macos runs on macOS only");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
mod app {
    use catalog_core::catalog::CatalogState;
    use catalog_core::db::{self, FileSource};
    use catalog_core::models::{BrowseMode, CatalogSortOrder};
    use catalog_core::settings::Settings;
    use catalog_core::settings_draft::SettingsDraft;
    use objc2::ffi::NSInteger;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{
        define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadMarker,
        MainThreadOnly, Message,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
        NSButton, NSImage, NSImageCell, NSImageScaling, NSImageView, NSMenu, NSMenuItem,
        NSOpenPanel, NSPopUpButton, NSScrollView, NSSearchField, NSSecureTextField, NSTableColumn,
        NSTableView, NSTableViewDataSource, NSTextField, NSTextView, NSView, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{ns_string, NSData, NSPoint, NSRect, NSSize, NSString};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    // ------------------------------------------------------------------
    // Shared state (plain Send data; workers + main thread).
    // ------------------------------------------------------------------

    struct UiData {
        state: CatalogState,
        settings: Settings,
        detail_book: Option<(i64, String, String)>,
        status: String,
        /// Cover JPEG bytes by book id (worker-filled, main-thread shown).
        covers: HashMap<i64, Vec<u8>>,
    }

    impl UiData {
        fn new() -> Self {
            Self {
                state: CatalogState::new(),
                settings: Settings::load(),
                detail_book: None,
                status: String::new(),
                covers: HashMap::new(),
            }
        }
    }

    static DATA: OnceLock<Mutex<UiData>> = OnceLock::new();

    fn data() -> &'static Mutex<UiData> {
        DATA.get_or_init(|| Mutex::new(UiData::new()))
    }

    // ------------------------------------------------------------------
    // Widgets: raw pointers, main thread only.
    // ------------------------------------------------------------------

    struct Widgets {
        window: Option<*mut NSWindow>,
        table: Option<*mut NSTableView>,
        cover_col: Option<*mut NSTableColumn>,
        title_col: Option<*mut NSTableColumn>,
        author_col: Option<*mut NSTableColumn>,
        search: Option<*mut NSSearchField>,
        status: Option<*mut NSTextField>,
        mode_popup: Option<*mut NSPopUpButton>,
        sort_popup: Option<*mut NSPopUpButton>,
        back_btn: Option<*mut NSButton>,
        settings_win: Option<*mut NSWindow>,
        f_source: Option<*mut NSPopUpButton>,
        f_local: Option<*mut NSTextField>,
        f_server: Option<*mut NSTextField>,
        f_share: Option<*mut NSTextField>,
        f_calibre: Option<*mut NSTextField>,
        f_user: Option<*mut NSTextField>,
        f_pass: Option<*mut NSSecureTextField>,
        f_domain: Option<*mut NSTextField>,
        f_kobos: Option<*mut NSTextField>,
        f_kobo_default: Option<*mut NSTextField>,
        f_theme: Option<*mut NSPopUpButton>,
        f_status: Option<*mut NSTextField>,
        f_local_badge: Option<*mut NSTextField>,
        f_smb_badge: Option<*mut NSTextField>,
        f_db_badge: Option<*mut NSTextField>,
        f_kobo_badge: Option<*mut NSTextField>,
        detail_win: Option<*mut NSWindow>,
        detail_text: Option<*mut NSTextView>,
        detail_image: Option<*mut NSImageView>,
    }

    // SAFETY: every access runs on the main thread (actions, data source,
    // run_on_main completions); windows/views outlive the app.
    unsafe impl Send for Widgets {}
    unsafe impl Sync for Widgets {}

    impl Widgets {
        fn empty() -> Self {
            Self {
                window: None,
                table: None,
                cover_col: None,
                title_col: None,
                author_col: None,
                search: None,
                status: None,
                mode_popup: None,
                sort_popup: None,
                back_btn: None,
                settings_win: None,
                f_source: None,
                f_local: None,
                f_server: None,
                f_share: None,
                f_calibre: None,
                f_user: None,
                f_pass: None,
                f_domain: None,
                f_kobos: None,
                f_kobo_default: None,
                f_theme: None,
                f_status: None,
                f_local_badge: None,
                f_smb_badge: None,
                f_db_badge: None,
                f_kobo_badge: None,
                detail_win: None,
                detail_text: None,
                detail_image: None,
            }
        }
    }

    /// Dereference a widget pointer (main thread only). The pointer is
    /// `Copy`d out of the ivars first, so no guard is held while the
    /// caller uses the view — sound because all widgets outlive the app
    /// and every use runs on the main thread.
    unsafe fn wref<T>(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut T>,
    ) -> Option<&T> {
        f(&iv.lock().unwrap()).map(|p| unsafe { &*p })
    }

    fn ptr_of<T: Message>(obj: &Retained<T>) -> *mut T {
        Retained::as_ptr(obj) as *mut T
    }

    fn raw_self(c: &CatalogController) -> *const CatalogController {
        c as *const _
    }

    // ------------------------------------------------------------------
    // Controller.
    // ------------------------------------------------------------------

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "CatalogController"]
        #[ivars = Mutex<Widgets>]
        struct CatalogController;

        unsafe impl NSObjectProtocol for CatalogController {}

        unsafe impl NSApplicationDelegate for CatalogController {
            #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
            fn quit_on_close(&self, _sender: &NSApplication) -> bool {
                true
            }
        }

        unsafe impl NSTableViewDataSource for CatalogController {
            #[unsafe(method(numberOfRowsInTableView:))]
            fn nrows(&self, _table: &NSTableView) -> NSInteger {
                let n = visible_count();
                eprintln!("DBG nrows={n}");
                n as NSInteger
            }

            #[unsafe(method_id(tableView:objectValueForTableColumn:row:))]
            fn cell(
                &self,
                _table: &NSTableView,
                col: &NSTableColumn,
                row: NSInteger,
            ) -> Retained<AnyObject> {
                let iv = self.ivars().lock().unwrap();
                let is_cover = iv
                    .cover_col
                    .map(|p| std::ptr::eq(col, unsafe { &*p }))
                    .unwrap_or(false);
                let is_title = iv
                    .title_col
                    .map(|p| std::ptr::eq(col, unsafe { &*p }))
                    .unwrap_or(true);
                drop(iv);
                // NOTE: no early `return` here — method_id methods convert
                // the tail expression via into_return; early returns
                // bypass that and fail to compile.
                if is_cover {
                    match cover_image_for_row(row) {
                        Some(img) => upcast_image(img),
                        None => upcast_string(NSString::from_str("")),
                    }
                } else {
                    upcast_string(NSString::from_str(&row_text(row, is_title)))
                }
            }
        }

        impl CatalogController {
            // Designated initializer: ivars start uninitialized, so set
            // them here (called via `new` at launch). `method_id` is
            // required for Retained returns.
            #[unsafe(method_id(init))]
            fn init(this: Allocated<Self>) -> Retained<Self> {
                let this = this.set_ivars(Mutex::new(Widgets::empty()));
                unsafe { msg_send![super(this), init] }
            }

            #[unsafe(method(doNothing:))]
            fn do_nothing(&self, _sender: &AnyObject) {
                // Popup change actions with no live behavior (source and
                // theme apply on Save).
            }

            #[unsafe(method(doSearch:))]
            fn do_search(&self, _sender: &AnyObject) {
                // NSSearchField derefs to NSTextField for stringValue.
                // (Fires from the Search button; Return key behavior on
                // NSSearchField is unreliable for action delivery.)
                let q = unsafe { wref(self.ivars(), |w| w.search) }
                    .map(|s| s.stringValue().to_string())
                    .unwrap_or_default();
                if q.trim().is_empty() {
                    return;
                }
                spawn_search(q);
            }

            #[unsafe(method(doReload:))]
            fn do_reload(&self, _sender: &AnyObject) {
                spawn_reload();
            }

            #[unsafe(method(openSettings:))]
            fn open_settings(&self, _sender: &AnyObject) {
                fill_settings_form(self);
                show_win(self.ivars(), |w| w.settings_win);
            }

            #[unsafe(method(openLibrary:))]
            fn open_library(&self, _sender: &AnyObject) {
                let mtm = MainThreadMarker::new().unwrap();
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(false);
                panel.setCanChooseDirectories(true);
                // NSModalResponse is an NSInteger alias (OK == 1).
                if panel.runModal() == 1 {
                    if let Some(path) = panel.URL().and_then(|u| u.path()) {
                        let dir = path.to_string();
                        let mut d = data().lock().unwrap();
                        d.settings.library_source = "local".to_string();
                        d.settings.local_library_dir = dir;
                        let _ = d.settings.save();
                    }
                    spawn_reload();
                }
            }

            #[unsafe(method(saveSettings:))]
            fn save_settings(&self, _sender: &AnyObject) {
                if save_settings_form(self) {
                    hide_win(self.ivars(), |w| w.settings_win);
                    spawn_reload();
                }
            }

            #[unsafe(method(cancelSettings:))]
            fn cancel_settings(&self, _sender: &AnyObject) {
                hide_win(self.ivars(), |w| w.settings_win);
            }

            #[unsafe(method(browseLocal:))]
            fn browse_local(&self, _sender: &AnyObject) {
                let mtm = MainThreadMarker::new().unwrap();
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(false);
                panel.setCanChooseDirectories(true);
                // NSModalResponse is an NSInteger alias (OK == 1).
                if panel.runModal() == 1 {
                    if let Some(path) = panel.URL().and_then(|u| u.path()) {
                        set_field(self.ivars(), |w| w.f_local, &path.to_string());
                    }
                }
            }

            #[unsafe(method(testLocal:))]
            fn test_local(&self, _sender: &AnyObject) {
                let dir = get_field(self.ivars(), |w| w.f_local).unwrap_or_default();
                set_badge(self.ivars(), |w| w.f_local_badge, "");
                set_form_status(self, "Testing local Calibre…");
                let me = ControllerPtr(raw_self(self));
                std::thread::spawn(move || {
                    let (pf, status) = SettingsDraft::check_local(&dir);
                    dispatch2::run_on_main(move |_mtm| {
                        let c = ctrl_of(me);
                        set_badge(c.ivars(), |w| w.f_local_badge, badge_of(pf));
                        set_form_status(c, &status);
                    });
                });
            }

            #[unsafe(method(testSmb:))]
            fn test_smb(&self, _sender: &AnyObject) {
                let draft = read_draft_fields(self);
                let conn = draft.smb_conn();
                let share = draft.smb_share.clone();
                if conn.host.trim().is_empty() || share.trim().is_empty() {
                    set_badge(self.ivars(), |w| w.f_smb_badge, "Fail");
                    set_form_status(self, "Fail SMB-connection: enter host and share first");
                    return;
                }
                set_badge(self.ivars(), |w| w.f_smb_badge, "");
                set_form_status(
                    self,
                    &format!("Testing SMB-connection {}/{}…", conn.host, share),
                );
                let me = ControllerPtr(raw_self(self));
                std::thread::spawn(move || {
                    let rt = worker_runtime();
                    let (pf, status) =
                        rt.block_on(SettingsDraft::check_smb_connection(&conn, &share));
                    dispatch2::run_on_main(move |_mtm| {
                        let c = ctrl_of(me);
                        set_badge(c.ivars(), |w| w.f_smb_badge, badge_of(pf));
                        set_form_status(c, &status);
                    });
                });
            }

            #[unsafe(method(testDb:))]
            fn test_db(&self, _sender: &AnyObject) {
                let draft = read_draft_fields(self);
                let conn = draft.smb_conn();
                let share = draft.smb_share.clone();
                let lib = draft.calibre_path.clone();
                if conn.host.trim().is_empty() || share.trim().is_empty() {
                    set_badge(self.ivars(), |w| w.f_db_badge, "Fail");
                    set_form_status(self, "Fail SMB-database: enter host and share first");
                    return;
                }
                set_badge(self.ivars(), |w| w.f_db_badge, "");
                set_form_status(self, "Testing SMB-database…");
                let me = ControllerPtr(raw_self(self));
                std::thread::spawn(move || {
                    let rt = worker_runtime();
                    let (pf, status) =
                        rt.block_on(SettingsDraft::check_smb_database(&conn, &share, &lib));
                    dispatch2::run_on_main(move |_mtm| {
                        let c = ctrl_of(me);
                        set_badge(c.ivars(), |w| w.f_db_badge, badge_of(pf));
                        set_form_status(c, &status);
                    });
                });
            }

            #[unsafe(method(testKobo:))]
            fn test_kobo(&self, _sender: &AnyObject) {
                let def = get_field(self.ivars(), |w| w.f_kobo_default).unwrap_or_default();
                if def.trim().is_empty() {
                    set_badge(self.ivars(), |w| w.f_kobo_badge, "Fail");
                    set_form_status(self, "Enter Kobo IP");
                    return;
                }
                set_badge(self.ivars(), |w| w.f_kobo_badge, "");
                set_form_status(self, &format!("Testing Kobo {def}… (ping)"));
                let me = ControllerPtr(raw_self(self));
                std::thread::spawn(move || {
                    let (_key, pf, status) = SettingsDraft::check_kobo(&def);
                    dispatch2::run_on_main(move |_mtm| {
                        let c = ctrl_of(me);
                        set_badge(c.ivars(), |w| w.f_kobo_badge, badge_of(Some(pf)));
                        set_form_status(c, &status);
                    });
                });
            }

            #[unsafe(method(modeChanged:))]
            fn mode_changed(&self, _sender: &AnyObject) {
                if let Some(idx) = popup_index(self.ivars(), |w| w.mode_popup) {
                    let mode = match idx {
                        1 => BrowseMode::Author,
                        2 => BrowseMode::Series,
                        3 => BrowseMode::Tags,
                        _ => BrowseMode::Books,
                    };
                    {
                        let mut d = data().lock().unwrap();
                        d.state.browse_mode = Some(mode);
                        d.state.exit_drill();
                        d.state.exit_search_mode();
                    }
                    refresh_sort_popup(self);
                    spawn_reload();
                }
            }

            #[unsafe(method(sortChanged:))]
            fn sort_changed(&self, _sender: &AnyObject) {
                if let Some(idx) = popup_index(self.ivars(), |w| w.sort_popup) {
                    let orders = {
                        let d = data().lock().unwrap();
                        d.state.available_sort_orders()
                    };
                    if let Some(order) = orders.get(idx).copied() {
                        data().lock().unwrap().state.sort_order = Some(order);
                        spawn_reload();
                    }
                }
            }

            #[unsafe(method(goBack:))]
            fn go_back(&self, _sender: &AnyObject) {
                {
                    let mut d = data().lock().unwrap();
                    d.state.exit_drill();
                    d.state.exit_search_mode();
                }
                spawn_reload();
            }

            #[unsafe(method(openSelected:))]
            fn open_selected(&self, _sender: &AnyObject) {
                let row = selected_row(self);
                if row < 0 {
                    return;
                }
                let idx = row as usize;
                let d = data().lock().unwrap();
                if d.state.is_search_mode {
                    let hit = d.state.search_results.get(idx).and_then(|r| {
                        r.book_id.map(|id| (id, r.file_path.clone()))
                    });
                    drop(d);
                    if let Some((id, path)) = hit {
                        open_detail(id, path);
                    }
                    return;
                }
                if d.state.drilled_kind.is_some() {
                    let sorted = d.state.drilled_books_sorted();
                    drop(d);
                    if let Some(b) = sorted.get(idx) {
                        open_detail(b.id, b.path.clone());
                    }
                    return;
                }
                match d.state.browse_mode {
                    Some(BrowseMode::Books) => {
                        let hit = d.state.books.get(idx).map(|b| (b.id, b.path.clone()));
                        drop(d);
                        if let Some((id, path)) = hit {
                            open_detail(id, path);
                        }
                    }
                    Some(m) => {
                        let (title, ident) = match m {
                            BrowseMode::Author => d
                                .state
                                .authors
                                .get(idx)
                                .map(|a| (a.name.clone(), a.id))
                                .unwrap_or_default(),
                            BrowseMode::Series => d
                                .state
                                .series
                                .get(idx)
                                .map(|s| (s.name.clone(), s.id))
                                .unwrap_or_default(),
                            BrowseMode::Tags => d
                                .state
                                .tags
                                .get(idx)
                                .map(|t| (t.name.clone(), t.id))
                                .unwrap_or_default(),
                            BrowseMode::Books => (String::new(), 0),
                        };
                        drop(d);
                        if ident != 0 || !title.is_empty() {
                            spawn_drill(m, title, ident);
                        }
                    }
                    None => {}
                }
            }

            #[unsafe(method(readOnKobo:))]
            fn read_on_kobo(&self, _sender: &AnyObject) {
                let (book, ip) = {
                    let d = data().lock().unwrap();
                    (d.detail_book.clone(), d.settings.kobo_ip.trim().to_string())
                };
                let Some((_, title, author)) = book else {
                    return;
                };
                if ip.is_empty() {
                    set_status(self, "Kobo IP not set — enter it in Settings → Kobo");
                    return;
                }
                set_status(self, &format!("Opening “{title}” on Kobo…"));
                let me = ControllerPtr(raw_self(self));
                std::thread::spawn(move || {
                    let rt = worker_runtime();
                    let out = rt.block_on(kobo_open(&title, &author, &ip));
                    let opened = out.starts_with("Opened on Kobo:");
                    dispatch2::run_on_main(move |_mtm| {
                        let c = ctrl_of(me);
                        if opened {
                            set_status(c, &out);
                        } else {
                            set_status(c, &format!("Kobo failed: {out}"));
                        }
                    });
                });
            }

            #[unsafe(method(closeDetail:))]
            fn close_detail(&self, _sender: &AnyObject) {
                hide_win(self.ivars(), |w| w.detail_win);
            }
        }
    );

    fn badge_of(pf: Option<catalog_core::settings_draft::PassFail>) -> &'static str {
        pf.map(|p| p.label()).unwrap_or("")
    }

    fn upcast_string(s: Retained<NSString>) -> Retained<AnyObject> {
        s.into_super().into_super()
    }

    fn upcast_image(i: Retained<NSImage>) -> Retained<AnyObject> {
        i.into_super().into_super()
    }

    /// (id, book path) for rows that represent books; None for
    /// author/series/tag summary rows (no single cover).
    fn row_book(row: NSInteger) -> Option<(i64, String)> {
        let d = data().lock().unwrap();
        let idx = row as usize;
        if d.state.is_search_mode {
            if idx < d.state.series_results.len() {
                return None;
            }
            return d.state.search_results
                .get(idx - d.state.series_results.len())
                .and_then(|b| b.book_id.map(|id| (id, b.file_path.clone())));
        }
        if d.state.drilled_kind.is_some() {
            let sorted = d.state.drilled_books_sorted();
            return sorted.get(idx).map(|b| (b.id, b.path.clone()));
        }
        match d.state.browse_mode {
            Some(BrowseMode::Books) => {
                d.state.books.get(idx).map(|b| (b.id, b.path.clone()))
            }
            _ => None,
        }
    }

    /// NSImage for a table row's cover, if already fetched (main thread).
    fn cover_image_for_row(row: NSInteger) -> Option<Retained<NSImage>> {
        let d = data().lock().unwrap();
        let (id, _) = row_book(row)?;
        let bytes = d.covers.get(&id)?;
        nsimage_from_jpeg(bytes)
    }

    /// JPEG bytes → NSImage. NSImage is not MainThreadOnly, so this
    /// works on workers too (cover decode stays off the main thread;
    /// only NSImageView assignment needs main).
    fn nsimage_from_jpeg(bytes: &[u8]) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(bytes);
        let img: Option<Retained<NSImage>> =
            unsafe { msg_send![NSImage::alloc(), initWithData: &*data] };
        img
    }

    fn set_detail_image(ctrl: &CatalogController, bytes: Option<&[u8]>) {
        let img = bytes.and_then(nsimage_from_jpeg);
        if let Some(v) = unsafe { wref(ctrl.ivars(), |w| w.detail_image) } {
            v.setImage(img.as_deref());
        }
    }

    // ------------------------------------------------------------------
    // Widget helpers (main thread only).
    // ------------------------------------------------------------------

    fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    fn ns(s: &str) -> Retained<NSString> {
        NSString::from_str(s)
    }

    fn make_label(
        mtm: MainThreadMarker,
        parent: &NSView,
        text: &str,
        x: f64,
        y: f64,
        w: f64,
    ) -> Retained<NSTextField> {
        let v: Retained<NSTextField> =
            unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame(x, y, w, 22.0)] };
        v.setStringValue(&ns(text));
        v.setEditable(false);
        v.setBezeled(false);
        v.setDrawsBackground(false);
        v.setSelectable(false);
        parent.addSubview(&v);
        v
    }

    fn make_field(
        mtm: MainThreadMarker,
        parent: &NSView,
        x: f64,
        y: f64,
        w: f64,
    ) -> Retained<NSTextField> {
        let v: Retained<NSTextField> =
            unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame(x, y, w, 24.0)] };
        parent.addSubview(&v);
        v
    }

    fn make_secure(
        mtm: MainThreadMarker,
        parent: &NSView,
        x: f64,
        y: f64,
        w: f64,
    ) -> Retained<NSSecureTextField> {
        let v: Retained<NSSecureTextField> =
            unsafe { msg_send![NSSecureTextField::alloc(mtm), initWithFrame: frame(x, y, w, 24.0)] };
        parent.addSubview(&v);
        v
    }
    fn get_field(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSTextField>,
    ) -> Option<String> {
        unsafe { wref(iv, f) }.map(|v| v.stringValue().to_string())
    }

    fn get_secure(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSSecureTextField>,
    ) -> Option<String> {
        unsafe { wref(iv, f) }.map(|v| v.stringValue().to_string())
    }

    fn set_field(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSTextField>,
        v: &str,
    ) {
        if let Some(t) = unsafe { wref(iv, f) } {
            t.setStringValue(&ns(v));
        }
    }

    fn set_secure(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSSecureTextField>,
        v: &str,
    ) {
        if let Some(t) = unsafe { wref(iv, f) } {
            t.setStringValue(&ns(v));
        }
    }

    fn set_badge(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSTextField>,
        v: &str,
    ) {
        set_field(iv, f, v);
    }

    fn set_form_status(ctrl: &CatalogController, msg: &str) {
        set_field(ctrl.ivars(), |w| w.f_status, msg);
    }

    fn set_status(ctrl: &CatalogController, msg: &str) {
        set_field(ctrl.ivars(), |w| w.status, msg);
        data().lock().unwrap().status = msg.to_string();
    }

    fn popup_index(
        iv: &Mutex<Widgets>,
        f: fn(&Widgets) -> Option<*mut NSPopUpButton>,
    ) -> Option<usize> {
        unsafe { wref(iv, f) }
            .map(|p| p.indexOfSelectedItem())
            .filter(|i| *i >= 0)
            .map(|i| i as usize)
    }

    fn selected_row(ctrl: &CatalogController) -> i64 {
        unsafe { wref(ctrl.ivars(), |w| w.table) }
            .map(|t| t.selectedRow() as i64)
            .unwrap_or(-1)
    }

    fn show_win(iv: &Mutex<Widgets>, f: fn(&Widgets) -> Option<*mut NSWindow>) {
        if let Some(w) = unsafe { wref(iv, f) } {
            w.makeKeyAndOrderFront(None);
        }
    }

    fn hide_win(iv: &Mutex<Widgets>, f: fn(&Widgets) -> Option<*mut NSWindow>) {
        if let Some(w) = unsafe { wref(iv, f) } {
            w.orderOut(None);
        }
    }

    fn refresh_table(ctrl: &CatalogController) {
        if let Some(t) = unsafe { wref(ctrl.ivars(), |w| w.table) } {
            t.reloadData();
        }
        refresh_back(ctrl);
    }

    fn refresh_back(ctrl: &CatalogController) {
        let show = {
            let d = data().lock().unwrap();
            d.state.drilled_kind.is_some() || d.state.is_search_mode
        };
        if let Some(b) = unsafe { wref(ctrl.ivars(), |w| w.back_btn) } {
            b.setHidden(!show);
        }
    }

    fn refresh_sort_popup(ctrl: &CatalogController) {
        let (labels, current) = {
            let d = data().lock().unwrap();
            let orders = d.state.available_sort_orders();
            let cur = d.state.sort_order.unwrap_or(CatalogSortOrder::Author);
            let labels: Vec<String> =
                orders.iter().map(|o| o.as_str().to_string()).collect();
            let pos = orders.iter().position(|o| *o == cur).unwrap_or(0);
            (labels, pos)
        };
        if let Some(p) = unsafe { wref(ctrl.ivars(), |w| w.sort_popup) } {
            p.removeAllItems();
            for l in &labels {
                p.addItemWithTitle(&ns(l));
            }
            p.selectItemAtIndex(current as NSInteger);
        }
    }

    fn refresh_status_from_state(ctrl: &CatalogController) {
        let msg = {
            let d = data().lock().unwrap();
            if let Some(e) = &d.state.db_error {
                e.clone()
            } else if !d.state.kobo_status.is_empty() {
                d.state.kobo_status.clone()
            } else if !d.status.is_empty() {
                d.status.clone()
            } else {
                d.state.mode_count_text()
            }
        };
        set_status(ctrl, &msg);
    }

    // ------------------------------------------------------------------
    // Table content derived from CatalogState.
    // ------------------------------------------------------------------

    fn visible_count() -> usize {
        let d = data().lock().unwrap();
        if d.state.is_search_mode {
            return d.state.search_results.len() + d.state.series_results.len();
        }
        if d.state.drilled_kind.is_some() {
            return d.state.drilled_books_sorted().len();
        }
        match d.state.browse_mode {
            Some(BrowseMode::Books) => d.state.books.len(),
            Some(BrowseMode::Author) => d.state.authors.len(),
            Some(BrowseMode::Series) => d.state.series.len(),
            Some(BrowseMode::Tags) => d.state.tags.len(),
            None => 0,
        }
    }

    fn row_text(row: NSInteger, is_title: bool) -> String {
        let d = data().lock().unwrap();
        let idx = row as usize;
        if d.state.is_search_mode {
            // Series hits first, then books (mirrors the Slint layout).
            if idx < d.state.series_results.len() {
                let s = &d.state.series_results[idx];
                return if is_title {
                    s.name.clone()
                } else {
                    format!("{} books", s.book_count)
                };
            }
            let b = &d.state.search_results[idx - d.state.series_results.len()];
            return if is_title {
                b.title.clone().unwrap_or_else(|| b.file_name.clone())
            } else {
                b.author.clone().unwrap_or_default()
            };
        }
        if d.state.drilled_kind.is_some() {
            let sorted = d.state.drilled_books_sorted();
            if let Some(b) = sorted.get(idx) {
                return if is_title { b.title.clone() } else { b.author.clone() };
            }
            return String::new();
        }
        match d.state.browse_mode {
            Some(BrowseMode::Books) => d.state.books.get(idx).map_or_else(String::new, |b| {
                if is_title { b.title.clone() } else { b.author.clone() }
            }),
            Some(BrowseMode::Author) => d.state.authors.get(idx).map_or_else(String::new, |a| {
                if is_title { a.name.clone() } else { format!("{} books", a.book_count) }
            }),
            Some(BrowseMode::Series) => d.state.series.get(idx).map_or_else(String::new, |s| {
                if is_title { s.name.clone() } else { format!("{} books", s.book_count) }
            }),
            Some(BrowseMode::Tags) => d.state.tags.get(idx).map_or_else(String::new, |t| {
                if is_title { t.name.clone() } else { format!("{} books", t.book_count) }
            }),
            None => String::new(),
        }
    }

    // ------------------------------------------------------------------
    // Workers (core on background threads; UI via run_on_main).
    // ------------------------------------------------------------------

    fn worker_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    fn snapshot_settings() -> Settings {
        data().lock().unwrap().settings.clone()
    }

    fn spawn_reload() {
        let me = ControllerPtr(controller_ptr());
        std::thread::spawn(move || {
            let rt = worker_runtime();
            let settings = snapshot_settings();
            let (mode, order) = {
                let d = data().lock().unwrap();
                (
                    d.state.browse_mode.unwrap_or(BrowseMode::Books),
                    d.state.sort_order.unwrap_or(CatalogSortOrder::Author),
                )
            };
            let outcome: Result<(), String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = db::open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let mut d = data().lock().unwrap();
                match mode {
                    BrowseMode::Books => {
                        d.state.books = db::fetch_books(
                            &db,
                            &source,
                            "",
                            order.is_descending(),
                            order.is_by_author(),
                            order.is_by_date(),
                        )
                        .map_err(|e| e.to_string())?;
                        d.state.total_count =
                            db::fetch_count(&db, "").map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Author => {
                        d.state.authors = db::all_authors(&db, &source, order.is_descending())
                            .map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Series => {
                        d.state.series = db::all_series(
                            &db,
                            &source,
                            order.is_descending(),
                            order.is_by_author(),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    BrowseMode::Tags => {
                        d.state.tags = db::all_tags(&db, order.is_descending())
                            .map_err(|e| e.to_string())?;
                    }
                }
                d.state.db_error = None;
                d.status.clear();
                Ok(())
            });
            dispatch2::run_on_main(move |_mtm| {
                let c = ctrl_of(me);
                if let Err(e) = outcome {
                    let mut d = data().lock().unwrap();
                    d.state.db_error = Some(e);
                }
                refresh_table(c);
                refresh_status_from_state(c);
                prefetch_covers();
            });
        });
    }

    fn spawn_search(query: String) {
        let me = ControllerPtr(controller_ptr());
        std::thread::spawn(move || {
            let rt = worker_runtime();
            let settings = snapshot_settings();
            let outcome: Result<(), String> = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = db::open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let params = db::SearchParams {
                    query,
                    ..Default::default()
                };
                let found = db::search_books(&db, &source, &params).map_err(|e| e.to_string())?;
                let mut d = data().lock().unwrap();
                d.state.is_search_mode = true;
                d.state.search_results = found.into_iter().map(Into::into).collect();
                d.state.series_results = vec![];
                d.state.db_error = None;
                d.status.clear();
                Ok(())
            });
            dispatch2::run_on_main(move |_mtm| {
                let c = ctrl_of(me);
                if let Err(e) = outcome {
                    data().lock().unwrap().state.db_error = Some(e);
                }
                refresh_table(c);
                refresh_status_from_state(c);
                prefetch_covers();
            });
        });
    }

    fn spawn_drill(mode: BrowseMode, title: String, id: i64) {
        let me = ControllerPtr(controller_ptr());
        std::thread::spawn(move || {
            let rt = worker_runtime();
            let settings = snapshot_settings();
            // Tag name snapshot for the tag-drill query.
            let tag_name = {
                let d = data().lock().unwrap();
                d.state.tags.iter().find(|t| t.id == id).map(|t| t.name.clone())
            };
            let outcome: Result<Vec<catalog_core::models::AuthorBook>, String> =
                rt.block_on(async {
                    let source =
                        FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                    let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                    let db = db::open_memory_db(&bytes).map_err(|e| e.to_string())?;
                    let books = match mode {
                        BrowseMode::Author => {
                            db::books_by_author(&db, &source, id).map_err(|e| e.to_string())?
                        }
                        BrowseMode::Series => {
                            db::books_by_series(&db, &source, id).map_err(|e| e.to_string())?
                        }
                        BrowseMode::Tags => {
                            let p = db::SearchParams {
                                tag: tag_name.unwrap_or(title.clone()),
                                ..Default::default()
                            };
                            db::search_books(&db, &source, &p)
                                .map_err(|e| e.to_string())?
                                .into_iter()
                                .map(|s| catalog_core::models::AuthorBook {
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
            dispatch2::run_on_main(move |_mtm| {
                let c = ctrl_of(me);
                match outcome {
                    Ok(books) => {
                        let mut d = data().lock().unwrap();
                        d.state.drilled_kind = Some(mode);
                        d.state.drilled_title = Some(title);
                        d.state.drilled_books = books;
                        d.state.db_error = None;
                    }
                    Err(e) => {
                        data().lock().unwrap().state.db_error = Some(e);
                    }
                }
                refresh_table(c);
                refresh_status_from_state(c);
                prefetch_covers();
            });
        });
    }

    fn open_detail(id: i64, path: String) {
        let me = ControllerPtr(controller_ptr());
        std::thread::spawn(move || {
            let rt = worker_runtime();
            let settings = snapshot_settings();
            let outcome: Result<
                (catalog_core::models::BookDetail, Option<Vec<u8>>),
                String,
            > = rt.block_on(async {
                let source =
                    FileSource::from_settings(&settings).map_err(|e| e.to_string())?;
                let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
                let db = db::open_memory_db(&bytes).map_err(|e| e.to_string())?;
                let detail = db::book_detail(&db, id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("no book id {id}"))?;
                let cover = catalog_core::covers::fetch_cover_cached(&source, &path)
                    .await
                    .and_then(|raw| catalog_core::covers::scale_cover(&raw, 148, 220));
                Ok((detail, cover))
            });
            dispatch2::run_on_main(move |_mtm| {
                let c = ctrl_of(me);
                match outcome {
                    Ok((d, cover)) => {
                        {
                            let mut data = data().lock().unwrap();
                            data.detail_book = Some((
                                d.id,
                                d.title.clone(),
                                d.author.clone(),
                            ));
                            if let Some(cov) = &cover {
                                data.covers.insert(d.id, cov.clone());
                            }
                        }
                        fill_detail(c, &d);
                        set_detail_image(c, cover.as_deref());
                        show_win(c.ivars(), |w| w.detail_win);
                        // The table may now have this cover too.
                        refresh_table(c);
                    }
                    Err(e) => {
                        data().lock().unwrap().state.db_error = Some(e);
                        refresh_status_from_state(c);
                    }
                }
            });
        });
    }

    /// Background cover prefetch for the visible book rows: at most 4
    /// concurrent fetches (Swift capped at 6), table refresh about twice
    /// a second as covers land. Disk-cached covers resolve instantly.
    fn prefetch_covers() {
        let (source, items) = {
            let d = data().lock().unwrap();
            let source = match FileSource::from_settings(&d.settings) {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows: Vec<(i64, String)> = if d.state.is_search_mode {
                d.state
                    .search_results
                    .iter()
                    .filter_map(|b| b.book_id.map(|id| (id, b.file_path.clone())))
                    .collect()
            } else if d.state.drilled_kind.is_some() {
                d.state
                    .drilled_books_sorted()
                    .into_iter()
                    .map(|b| (b.id, b.path))
                    .collect()
            } else {
                match d.state.browse_mode {
                    Some(BrowseMode::Books) => d
                        .state
                        .books
                        .iter()
                        .map(|b| (b.id, b.path.clone()))
                        .collect(),
                    _ => vec![],
                }
            };
            let missing: Vec<(i64, String)> = rows
                .into_iter()
                .filter(|(id, _)| !d.covers.contains_key(id))
                .collect();
            (source, missing)
        };
        if items.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            let rt = worker_runtime();
            rt.block_on(async {
                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
                let (tx, mut rx) =
                    tokio::sync::mpsc::unbounded_channel::<(i64, Vec<u8>)>();
                for (id, path) in items {
                    let sem = sem.clone();
                    let tx = tx.clone();
                    let source = source.clone();
                    tokio::spawn(async move {
                        let Ok(_permit) = sem.acquire_owned().await else {
                            return;
                        };
                        if let Some(raw) =
                            catalog_core::covers::fetch_cover_cached(&source, &path).await
                        {
                            if let Some(thumb) =
                                catalog_core::covers::scale_cover(&raw, 44, 64)
                            {
                                let _ = tx.send((id, thumb));
                            }
                        }
                    });
                }
                drop(tx);
                let mut since_flush = 0u32;
                let mut last = std::time::Instant::now();
                while let Some((id, thumb)) = rx.recv().await {
                    data().lock().unwrap().covers.insert(id, thumb);
                    since_flush += 1;
                    if since_flush >= 20
                        || last.elapsed() > std::time::Duration::from_millis(800)
                    {
                        since_flush = 0;
                        last = std::time::Instant::now();
                        let me = ControllerPtr(controller_ptr());
                        dispatch2::run_on_main(move |_mtm| {
                            refresh_table(ctrl_of(me));
                        });
                    }
                }
                let me = ControllerPtr(controller_ptr());
                dispatch2::run_on_main(move |_mtm| {
                    refresh_table(ctrl_of(me));
                });
            });
        });
    }

    /// Kobo open flow (Swift KoboLauncher.runKoboSSH): probe → predicted
    /// path → strict index match → KOReader launch. Fail-closed.
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

    // ------------------------------------------------------------------
    // Settings form sync (main thread; pure field shuffling).
    // ------------------------------------------------------------------

    fn fill_settings_form(ctrl: &CatalogController) {
        let draft = {
            let d = data().lock().unwrap();
            SettingsDraft::load(&d.settings)
        };
        let iv = ctrl.ivars();
        set_field(iv, |w| w.f_local, &draft.local_dir);
        set_field(iv, |w| w.f_server, &draft.smb_server);
        set_field(iv, |w| w.f_share, &draft.smb_share);
        set_field(iv, |w| w.f_calibre, &draft.calibre_path);
        set_field(iv, |w| w.f_user, &draft.smb_user);
        set_secure(iv, |w| w.f_pass, &draft.smb_pass);
        set_field(iv, |w| w.f_domain, &draft.smb_domain);
        set_field(iv, |w| w.f_kobos, &draft.kobo_ips.join(", "));
        set_field(iv, |w| w.f_kobo_default, &draft.kobo_ip);
        set_field(iv, |w| w.f_status, "");
        set_field(iv, |w| w.f_local_badge, "");
        set_field(iv, |w| w.f_smb_badge, "");
        set_field(iv, |w| w.f_db_badge, "");
        set_field(iv, |w| w.f_kobo_badge, "");
        if let Some(p) = unsafe { wref(iv, |w| w.f_source) } {
            p.selectItemAtIndex(if draft.library_source == "local" { 1 } else { 0 });
        }
        if let Some(p) = unsafe { wref(iv, |w| w.f_theme) } {
            p.selectItemAtIndex(draft.theme_preference as NSInteger);
        }
    }

    fn read_draft_fields(ctrl: &CatalogController) -> SettingsDraft {
        let mut draft = {
            let d = data().lock().unwrap();
            SettingsDraft::load(&d.settings)
        };
        let iv = ctrl.ivars();
        draft.local_dir = get_field(iv, |w| w.f_local).unwrap_or_default();
        draft.smb_server = get_field(iv, |w| w.f_server).unwrap_or_default();
        draft.smb_share = get_field(iv, |w| w.f_share).unwrap_or_default();
        draft.calibre_path = get_field(iv, |w| w.f_calibre).unwrap_or_default();
        draft.smb_user = get_field(iv, |w| w.f_user).unwrap_or_default();
        draft.smb_pass = get_secure(iv, |w| w.f_pass).unwrap_or_default();
        draft.smb_domain = get_field(iv, |w| w.f_domain).unwrap_or_default();
        draft.kobo_ips = get_field(iv, |w| w.f_kobos)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        draft.kobo_ip = get_field(iv, |w| w.f_kobo_default).unwrap_or_default();
        if let Some(p) = unsafe { wref(iv, |w| w.f_source) } {
            draft.library_source = if p.indexOfSelectedItem() == 1 {
                "local".to_string()
            } else {
                "smb".to_string()
            };
        }
        if let Some(p) = unsafe { wref(iv, |w| w.f_theme) } {
            draft.theme_preference = p.indexOfSelectedItem() as i32;
        }
        draft
    }

    fn save_settings_form(ctrl: &CatalogController) -> bool {
        let mut draft = read_draft_fields(ctrl);
        // Kobo default falls back to first listed IP (Swift addKobo parity).
        if draft.kobo_ip.is_empty() && !draft.kobo_ips.is_empty() {
            draft.kobo_ip = draft.kobo_ips[0].clone();
        }
        let mut settings = data().lock().unwrap().settings.clone();
        let ok = draft.apply_to(&mut settings);
        // Surface the draft status + persist Kobo even when SMB blocks.
        set_field(ctrl.ivars(), |w| w.f_status, &draft.status);
        if ok {
            let _ = settings.save();
            data().lock().unwrap().settings = settings;
        } else {
            // Persist Kobo default even when SMB validation blocks
            // dismissal (Swift save() parity).
            let mut s = data().lock().unwrap().settings.clone();
            draft.save_kobo_into(&mut s);
            let _ = s.save();
            data().lock().unwrap().settings = s;
        }
        // Refresh the kobo-default field (fallback may have filled it).
        set_field(ctrl.ivars(), |w| w.f_kobo_default, &draft.kobo_ip);
        ok
    }

    fn fill_detail(
        ctrl: &CatalogController,
        d: &catalog_core::models::BookDetail,
    ) {
        let mut text = format!("{}\n{}\n", d.title, d.author);
        if let Some(s) = &d.series {
            text.push_str(&format!("\nSeries: {s} #{}\n", d.series_index));
        }
        if let Some(t) = &d.tags {
            text.push_str(&format!("\n{t}\n"));
        }
        if let Some(p) = &d.publisher {
            text.push_str(&format!("\nPublisher: {p}\n"));
        }
        if let Some(i) = &d.isbn {
            text.push_str(&format!("\nISBN: {i}\n"));
        }
        if let Some(c) = &d.comments {
            if !c.is_empty() {
                text.push_str(&format!("\n{c}\n"));
            }
        }
        if let Some(t) = unsafe { wref(ctrl.ivars(), |w| w.detail_text) } {
            t.setString(&ns(&text));
        }
    }

    // ------------------------------------------------------------------
    // UI construction (main thread).
    // ------------------------------------------------------------------

    struct ControllerPtr(*const CatalogController);

    // SAFETY: installed once on the main thread before any worker spawns;
    // dereferenced only on the main thread via run_on_main completions.
    unsafe impl Send for ControllerPtr {}
    unsafe impl Sync for ControllerPtr {}

    static CONTROLLER_PTR: OnceLock<ControllerPtr> = OnceLock::new();

    /// Reborrow the (leaked, main-thread-confined) controller from a
    /// worker completion. Taking the whole `ControllerPtr` by value keeps
    /// Rust 2021 disjoint capture from grabbing the raw `*const` field.
    fn ctrl_of(me: ControllerPtr) -> &'static CatalogController {
        unsafe { &*me.0 }
    }

    fn controller_ptr() -> *const CatalogController {
        CONTROLLER_PTR.get().expect("controller not installed").0
    }

    fn button(
        mtm: MainThreadMarker,
        parent: &NSView,
        title: &str,
        bounds: NSRect,
        ctrl: &CatalogController,
        action: objc2::runtime::Sel,
    ) -> Retained<NSButton> {
        let v: Retained<NSButton> =
            unsafe { msg_send![NSButton::alloc(mtm), initWithFrame: bounds] };
        v.setTitle(&ns(title));
        unsafe {
            let _: () = msg_send![&v, setTarget: ctrl];
            let _: () = msg_send![&v, setAction: action];
        }
        parent.addSubview(&v);
        v
    }

    fn popup(
        mtm: MainThreadMarker,
        parent: &NSView,
        bounds: NSRect,
        ctrl: &CatalogController,
        action: objc2::runtime::Sel,
        items: &[&str],
    ) -> Retained<NSPopUpButton> {
        let v: Retained<NSPopUpButton> = unsafe {
            msg_send![NSPopUpButton::alloc(mtm), initWithFrame: bounds, pullsDown: false]
        };
        for t in items {
            v.addItemWithTitle(&ns(t));
        }
        unsafe {
            let _: () = msg_send![&v, setTarget: ctrl];
            let _: () = msg_send![&v, setAction: action];
        }
        parent.addSubview(&v);
        v
    }

    fn make_window(mtm: MainThreadMarker, title: &str, x: f64, y: f64, w: f64, h: f64) -> Retained<NSWindow> {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        let window: Retained<NSWindow> = unsafe {
            msg_send![
                NSWindow::alloc(mtm),
                initWithContentRect: frame(x, y, w, h),
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        window.setTitle(&ns(title));
        window
    }

    fn content_of(window: &NSWindow) -> Retained<NSView> {
        window.contentView().expect("window content")
    }

    fn build_main_window(
        mtm: MainThreadMarker,
        ctrl: &CatalogController,
    ) -> Retained<NSWindow> {
        let window = make_window(mtm, "Jocala Catalog", 200.0, 160.0, 980.0, 720.0);
        let content = content_of(&window);

        // Toolbar row. Search runs from the Search button (NSSearchField
        // Return-key action delivery is unreliable for target/action).
        let search: Retained<NSSearchField> =
            unsafe { msg_send![NSSearchField::alloc(mtm), initWithFrame: frame(12.0, 682.0, 220.0, 26.0)] };
        content.addSubview(&search);
        button(mtm, &content, "Search", frame(238.0, 680.0, 70.0, 28.0), ctrl, sel!(doSearch:));
        button(mtm, &content, "Reload", frame(316.0, 680.0, 80.0, 28.0), ctrl, sel!(doReload:));
        button(mtm, &content, "Settings", frame(404.0, 680.0, 80.0, 28.0), ctrl, sel!(openSettings:));
        button(mtm, &content, "Open…", frame(492.0, 680.0, 80.0, 28.0), ctrl, sel!(openLibrary:));
        let mode = popup(
            mtm,
            &content,
            frame(580.0, 682.0, 110.0, 26.0),
            ctrl,
            sel!(modeChanged:),
            &["Books", "Author", "Series", "Tags"],
        );
        let sort = popup(
            mtm,
            &content,
            frame(698.0, 682.0, 110.0, 26.0),
            ctrl,
            sel!(sortChanged:),
            &["Author"],
        );
        let back = button(mtm, &content, "‹ Back", frame(816.0, 680.0, 80.0, 28.0), ctrl, sel!(goBack:));
        back.setHidden(true);
        let status = make_label(mtm, &content, "", 12.0, 652.0, 956.0);

        // Table.
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: frame(12.0, 12.0, 956.0, 630.0)] };
        scroll.setHasVerticalScroller(true);
        let table: Retained<NSTableView> =
            unsafe { msg_send![NSTableView::alloc(mtm), initWithFrame: frame(0.0, 0.0, 940.0, 600.0)] };
        let cover_col: Retained<NSTableColumn> = unsafe {
            msg_send![NSTableColumn::alloc(mtm), initWithIdentifier: &*ns("cover")]
        };
        cover_col.setTitle(&ns(""));
        cover_col.setWidth(52.0);
        // Image cells so objectValue NSImages render as art, not text.
        let cover_cell: Retained<NSImageCell> =
            unsafe { msg_send![NSImageCell::alloc(mtm), init] };
        // SAFETY: cover_cell outlives the column (leaked app-lifetime
        // via the window hierarchy, same as all other widgets here).
        unsafe { cover_col.setDataCell(&cover_cell) };
        let title_col: Retained<NSTableColumn> = unsafe {
            msg_send![NSTableColumn::alloc(mtm), initWithIdentifier: &*ns("title")]
        };
        title_col.setTitle(&ns("Title"));
        title_col.setWidth(508.0);
        let author_col: Retained<NSTableColumn> = unsafe {
            msg_send![NSTableColumn::alloc(mtm), initWithIdentifier: &*ns("author")]
        };
        author_col.setTitle(&ns("Author"));
        author_col.setWidth(360.0);
        table.addTableColumn(&cover_col);
        table.addTableColumn(&title_col);
        table.addTableColumn(&author_col);
        table.setRowHeight(56.0);
        let ds: &ProtocolObject<dyn NSTableViewDataSource> =
            ProtocolObject::from_ref(ctrl);
        unsafe { table.setDataSource(Some(ds)) };
        unsafe {
            let _: () = msg_send![&table, setTarget: ctrl];
            let _: () = msg_send![&table, setDoubleAction: sel!(openSelected:)];
        }
        scroll.setDocumentView(Some(&table));
        content.addSubview(&scroll);

        let iv = ctrl.ivars();
        let mut w = iv.lock().unwrap();
        w.window = Some(ptr_of(&window));
        w.table = Some(ptr_of(&table));
        w.cover_col = Some(ptr_of(&cover_col));
        w.title_col = Some(ptr_of(&title_col));
        w.author_col = Some(ptr_of(&author_col));
        w.search = Some(ptr_of(&search));
        w.status = Some(ptr_of(&status));
        w.mode_popup = Some(ptr_of(&mode));
        w.sort_popup = Some(ptr_of(&sort));
        w.back_btn = Some(ptr_of(&back));
        drop(w);

        window.center();
        window
    }

    fn build_settings_window(
        mtm: MainThreadMarker,
        ctrl: &CatalogController,
    ) -> Retained<NSWindow> {
        let window = make_window(mtm, "Settings", 260.0, 120.0, 600.0, 660.0);
        let content = content_of(&window);
        let mut y = 616.0;
        let step = 34.0;

        make_label(mtm, &content, "Library Source", 16.0, y, 544.0);
        y -= step;
        let source = popup(
            mtm,
            &content,
            frame(170.0, y, 140.0, 26.0),
            ctrl,
            sel!(doNothing:),
            &["SMB", "Local"],
        );
        make_label(mtm, &content, "Source", 16.0, y, 140.0);
        y -= step;
        make_label(mtm, &content, "Local Calibre Path", 16.0, y, 140.0);
        let local = make_field(mtm, &content, 170.0, y, 240.0);
        button(mtm, &content, "Browse", frame(418.0, y - 2.0, 70.0, 28.0), ctrl, sel!(browseLocal:));
        button(mtm, &content, "Test", frame(494.0, y - 2.0, 60.0, 28.0), ctrl, sel!(testLocal:));
        y -= 22.0;
        let local_badge = make_label(mtm, &content, "", 170.0, y, 200.0);
        y -= step - 12.0;

        make_label(mtm, &content, "SMB Server", 16.0, y, 140.0);
        let server = make_field(mtm, &content, 170.0, y, 200.0);
        button(mtm, &content, "Test SMB", frame(378.0, y - 2.0, 80.0, 28.0), ctrl, sel!(testSmb:));
        let smb_badge = make_label(mtm, &content, "", 464.0, y, 60.0);
        y -= step;
        make_label(mtm, &content, "Share", 16.0, y, 140.0);
        let share = make_field(mtm, &content, 170.0, y, 200.0);
        y -= step;
        make_label(mtm, &content, "SMB Calibre Path", 16.0, y, 140.0);
        let calibre = make_field(mtm, &content, 170.0, y, 200.0);
        button(mtm, &content, "Test Calibre", frame(378.0, y - 2.0, 100.0, 28.0), ctrl, sel!(testDb:));
        let db_badge = make_label(mtm, &content, "", 484.0, y, 60.0);
        y -= step;
        make_label(mtm, &content, "User", 16.0, y, 140.0);
        let user = make_field(mtm, &content, 170.0, y, 200.0);
        y -= step;
        make_label(mtm, &content, "Password", 16.0, y, 140.0);
        let pass = make_secure(mtm, &content, 170.0, y, 200.0);
        y -= step;
        make_label(mtm, &content, "Domain", 16.0, y, 140.0);
        let domain = make_field(mtm, &content, 170.0, y, 200.0);
        y -= step;
        make_label(mtm, &content, "Kobo IPs (comma)", 16.0, y, 140.0);
        let kobos = make_field(mtm, &content, 170.0, y, 240.0);
        y -= step;
        make_label(mtm, &content, "Default Kobo IP", 16.0, y, 140.0);
        let kobo_default = make_field(mtm, &content, 170.0, y, 200.0);
        button(mtm, &content, "Test", frame(378.0, y - 2.0, 60.0, 28.0), ctrl, sel!(testKobo:));
        let kobo_badge = make_label(mtm, &content, "", 444.0, y, 60.0);
        y -= step;
        make_label(mtm, &content, "Theme", 16.0, y, 140.0);
        let theme = popup(
            mtm,
            &content,
            frame(170.0, y, 140.0, 26.0),
            ctrl,
            sel!(doNothing:),
            &["System", "Light", "Dark"],
        );
        y -= step;
        let status = make_label(mtm, &content, "", 16.0, y - 8.0, 568.0);
        button(mtm, &content, "Cancel", frame(400.0, 12.0, 80.0, 28.0), ctrl, sel!(cancelSettings:));
        button(mtm, &content, "Save", frame(490.0, 12.0, 80.0, 28.0), ctrl, sel!(saveSettings:));

        // Source/theme popups don't need change actions in v1 (read on Save).
        let iv = ctrl.ivars();
        let mut w = iv.lock().unwrap();
        w.settings_win = Some(ptr_of(&window));
        w.f_source = Some(ptr_of(&source));
        w.f_local = Some(ptr_of(&local));
        w.f_server = Some(ptr_of(&server));
        w.f_share = Some(ptr_of(&share));
        w.f_calibre = Some(ptr_of(&calibre));
        w.f_user = Some(ptr_of(&user));
        w.f_pass = Some(ptr_of(&pass));
        w.f_domain = Some(ptr_of(&domain));
        w.f_kobos = Some(ptr_of(&kobos));
        w.f_kobo_default = Some(ptr_of(&kobo_default));
        w.f_theme = Some(ptr_of(&theme));
        w.f_status = Some(ptr_of(&status));
        w.f_local_badge = Some(ptr_of(&local_badge));
        w.f_smb_badge = Some(ptr_of(&smb_badge));
        w.f_db_badge = Some(ptr_of(&db_badge));
        w.f_kobo_badge = Some(ptr_of(&kobo_badge));
        drop(w);
        window
    }

    fn build_detail_window(
        mtm: MainThreadMarker,
        ctrl: &CatalogController,
    ) -> Retained<NSWindow> {
        let window = make_window(mtm, "Book Details", 300.0, 200.0, 560.0, 480.0);
        let content = content_of(&window);
        let cover: Retained<NSImageView> =
            unsafe { msg_send![NSImageView::alloc(mtm), initWithFrame: frame(12.0, 216.0, 160.0, 240.0)] };
        cover.setImageScaling(NSImageScaling::ScaleProportionallyDown);
        content.addSubview(&cover);
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: frame(184.0, 56.0, 364.0, 412.0)] };
        scroll.setHasVerticalScroller(true);
        let text: Retained<NSTextView> =
            unsafe { msg_send![NSTextView::alloc(mtm), initWithFrame: frame(0.0, 0.0, 348.0, 400.0)] };
        text.setEditable(false);
        scroll.setDocumentView(Some(&text));
        content.addSubview(&scroll);
        button(mtm, &content, "Read on Kobo", frame(12.0, 12.0, 120.0, 28.0), ctrl, sel!(readOnKobo:));
        button(mtm, &content, "Close", frame(142.0, 12.0, 80.0, 28.0), ctrl, sel!(closeDetail:));
        let iv = ctrl.ivars();
        let mut w = iv.lock().unwrap();
        w.detail_win = Some(ptr_of(&window));
        w.detail_text = Some(ptr_of(&text));
        w.detail_image = Some(ptr_of(&cover));
        drop(w);
        window
    }

    fn make_app_menu(mtm: MainThreadMarker, ctrl: &CatalogController) -> Retained<NSMenu> {
        let menu = NSMenu::new(mtm);
        let app_item = NSMenuItem::new(mtm);
        let app_menu = NSMenu::new(mtm);
        let quit = NSMenuItem::new(mtm);
        quit.setTitle(ns_string!("Quit Jocala Catalog"));
        unsafe { quit.setAction(Some(sel!(terminate:))) };
        quit.setKeyEquivalent(ns_string!("q"));
        app_menu.addItem(&quit);
        app_item.setSubmenu(Some(&app_menu));
        menu.addItem(&app_item);

        let file_item = NSMenuItem::new(mtm);
        let file_menu = NSMenu::new(mtm);
        file_item.setTitle(ns_string!("File"));
        let open = NSMenuItem::new(mtm);
        open.setTitle(ns_string!("Open…"));
        unsafe {
            let _: () = msg_send![&open, setTarget: ctrl];
            open.setAction(Some(sel!(openLibrary:)));
        }
        open.setKeyEquivalent(ns_string!("o"));
        file_menu.addItem(&open);
        let reload = NSMenuItem::new(mtm);
        reload.setTitle(ns_string!("Reload"));
        unsafe {
            let _: () = msg_send![&reload, setTarget: ctrl];
            reload.setAction(Some(sel!(doReload:)));
        }
        reload.setKeyEquivalent(ns_string!("r"));
        file_menu.addItem(&reload);
        file_item.setSubmenu(Some(&file_menu));
        menu.addItem(&file_item);
        menu
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("must run on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        let controller: Retained<CatalogController> =
            unsafe { msg_send![CatalogController::class(), new] };
        app.setMainMenu(Some(&make_app_menu(mtm, &controller)));
        app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));

        // Windows live in ivars; the controller lives forever.
        let _main = build_main_window(mtm, &controller);
        let _settings = build_settings_window(mtm, &controller);
        let _detail = build_detail_window(mtm, &controller);
        show_win(controller.ivars(), |w| w.window);
        refresh_sort_popup(&controller);
        let _ = CONTROLLER_PTR.set(ControllerPtr(ptr_of(&controller)));
        std::mem::forget(controller);

        spawn_reload();
        app.activate();
        app.run();
    }
}

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

