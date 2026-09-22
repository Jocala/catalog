//! catalog-ffi: C ABI over `catalog-core` for native GUI shells.
//!
//! One crate, one contract, every platform: SwiftUI (macOS) via Swift
//! `ctypes`, WPF (Windows) via P/Invoke, any future shell via the same
//! header. JSON in, JSON out — the DTOs already derive Serialize.
//!
//! Rules:
//! - Every function is **blocking** (SMB + SQLite run on a shared tokio
//!   runtime via `block_on`). Never call on a UI thread — C# wraps with
//!   `Task.Run`, Swift with `Task.detached`.
//! - Every `*mut c_char` return is owned by the caller; free with
//!   [`catalog_string_free`]. Binary blobs use [`CatalogBytes`] +
//!   [`catalog_bytes_free`].
//! - Errors come back as `{"error":"..."}` JSON, never a null pointer
//!   (null = misuse such as a null argument). No `unwrap()` on any path
//!   reachable from these entry points; panics are caught at the boundary.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::{Mutex, OnceLock};

use catalog_core::db::{self, FileSource, SearchParams};
use catalog_core::{covers, kobo};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("catalog-ffi")
            .enable_all()
            .build()
            .expect("catalog-ffi tokio runtime")
    })
}

/// Null-safe C string read.
fn c_str(p: *const c_char) -> Result<String, String> {
    if p.is_null() {
        return Err("null string argument".to_string());
    }
    // SAFETY: non-null per check; caller guarantees NUL-termination.
    let s = unsafe { CStr::from_ptr(p) };
    Ok(s.to_string_lossy().into_owned())
}

fn ok_json(payload: serde_json::Value) -> *mut c_char {
    let body = serde_json::json!({"ok": payload}).to_string();
    CString::new(body).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn err_json(msg: impl Into<String>) -> *mut c_char {
    let body = serde_json::json!({"error": msg.into()}).to_string();
    CString::new(body).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn run<F>(f: F) -> *mut c_char
where
    F: FnOnce() -> Result<serde_json::Value, String> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(Ok(v)) => ok_json(v),
        Ok(Err(e)) => err_json(e),
        Err(_) => err_json("internal panic"),
    }
}

fn cfg_get(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Build the [`FileSource`] from the config JSON. Shape:
/// `{"source":"smb","local_dir":"","host":"","share":"","remote_dir":"",
/// "user":"","pass":"","domain":""}` — for `"source":"local"` only
/// `local_dir` matters. Passwords arrive per-call; nothing is persisted
/// here (each shell keeps its own store: Keychain / settings JSON).
fn file_source(cfg: &serde_json::Value) -> Result<FileSource, String> {
    let source = cfg_get(cfg, "source");
    if source == "local" {
        let dir = cfg_get(cfg, "local_dir");
        if dir.is_empty() {
            return Err("not configured: no local library folder".to_string());
        }
        return Ok(FileSource::local(dir));
    }
    Ok(FileSource::smb_with_creds(
        &cfg_get(cfg, "host"),
        &cfg_get(cfg, "share"),
        &cfg_get(cfg, "remote_dir"),
        &cfg_get(cfg, "user"),
        &cfg_get(cfg, "pass"),
        &cfg_get(cfg, "domain"),
    ))
}

fn open_db(
    cfg: &serde_json::Value,
) -> Result<(rusqlite::Connection, FileSource), String> {
    // Process-wide metadata.db byte cache (mirrors WPF SmbCatalogDB:
    // single in-memory DB, Reload bypasses). Keyed by the full config
    // JSON (passwords ride along but never leave memory); 60s TTL.
    // Stale-read window matches the other ports: Calibre-side changes
    // appear after the TTL or an explicit Reload.
    type DbCache = Mutex<std::collections::HashMap<String, (Vec<u8>, std::time::Instant)>>;
    static CACHE: OnceLock<DbCache> = OnceLock::new();
    static TTL: std::time::Duration = std::time::Duration::from_secs(60);
    let src = file_source(cfg)?;
    let key = serde_json::to_string(cfg).unwrap_or_default();
    let fresh = cfg_get(cfg, "fresh") == "1";
    let bytes = if !fresh {
        CACHE
            .get_or_init(|| DbCache::new(std::collections::HashMap::new()))
            .lock()
            .map_err(|e| format!("db cache poisoned: {e}"))?
            .get(&key)
            .filter(|(_, t)| t.elapsed() < TTL)
            .map(|(b, _)| b.clone())
    } else {
        None
    };
    let bytes = match bytes {
        Some(b) => b,
        None => {
            let b = runtime()
                .block_on(src.read_db_bytes())
                .map_err(|e| e.to_string())?;
            if let Ok(mut cache) = CACHE
                .get_or_init(|| DbCache::new(std::collections::HashMap::new()))
                .lock()
            {
                cache.insert(key, (b.clone(), std::time::Instant::now()));
            }
            b
        }
    };
    let conn = db::open_memory_db(&bytes).map_err(|e| e.to_string())?;
    Ok((conn, src))
}

fn search_params(v: &serde_json::Value) -> SearchParams {
    SearchParams {
        query: cfg_get(v, "query"),
        title: cfg_get(v, "title"),
        author: cfg_get(v, "author"),
        series: cfg_get(v, "series"),
        tag: cfg_get(v, "tag"),
        publisher: cfg_get(v, "publisher"),
        date_from: cfg_get(v, "date_from"),
        date_to: cfg_get(v, "date_to"),
        sort_descending: v
            .get("sort_descending")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Ownership helpers
// ---------------------------------------------------------------------------

/// Free a string returned by any `catalog_*` function.
// C ABI: raw pointer without `unsafe` is conventional for free functions;
// null is accepted and ignored (see body).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn catalog_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: pointer came from `CString::into_raw` in this crate.
    unsafe {
        drop(CString::from_raw(s));
    }
}

/// Binary blob (cover JPEG bytes). Null `ptr` + zero `len` = miss.
#[repr(C)]
pub struct CatalogBytes {
    pub ptr: *mut u8,
    pub len: usize,
}

/// Free a blob returned by [`catalog_cover`].
// C ABI: by-value struct, no deref of caller memory — but the lint fires
// on the raw field; allow with the same justification as string_free.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn catalog_bytes_free(b: CatalogBytes) {
    if b.ptr.is_null() || b.len == 0 {
        return;
    }
    // SAFETY: buffer was built from a `Vec<u8>` via `into_raw_parts` below.
    unsafe {
        drop(Vec::from_raw_parts(b.ptr, b.len, b.len));
    }
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

/// `{"ok":{"version":"0.1.0"}}` — lets shells assert the DLL they loaded.
#[no_mangle]
pub extern "C" fn catalog_version() -> *mut c_char {
    ok_json(serde_json::json!({"version": env!("CARGO_PKG_VERSION")}))
}

// ---------------------------------------------------------------------------
// Library queries (each opens the DB, queries, drops it — same single-DB
// rule as core: live metadata.db straight into :memory:, no local copy)
// ---------------------------------------------------------------------------

/// `{"ok":{"books":N}}` — validates a library (parity gate entry point).
#[no_mangle]
pub extern "C" fn catalog_open_count(config_json: *const c_char) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let (conn, _src) = open_db(&cfg)?;
        let n = db::fetch_count(&conn, "").map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"books": n}))
    })
}

/// Root book list. Flags mirror the CLI/GUI root ordering.
#[no_mangle]
pub extern "C" fn catalog_fetch_books(
    config_json: *const c_char,
    query: *const c_char,
    sort_descending: bool,
    sort_by_author: bool,
    sort_by_date: bool,
) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let q = c_str(query).unwrap_or_default();
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let (conn, src) = open_db(&cfg)?;
        let books =
            db::fetch_books(&conn, &src, &q, sort_descending, sort_by_author, sort_by_date)
                .map_err(|e| e.to_string())?;
        serde_json::to_value(&books).map_err(|e| e.to_string())
    })
}

/// Tag/author/series tiles + drill-in, by mode string:
/// `"tags" | "authors" | "series" | "author_books:<id>" | "series_books:<id>"`.
#[no_mangle]
pub extern "C" fn catalog_browse(
    config_json: *const c_char,
    mode: *const c_char,
    sort_descending: bool,
    sort_by_author: bool,
) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let mode = c_str(mode)?;
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let (conn, src) = open_db(&cfg)?;
        if mode == "tags" {
            let tags = db::all_tags(&conn, sort_descending).map_err(|e| e.to_string())?;
            return serde_json::to_value(&tags).map_err(|e| e.to_string());
        }
        if mode == "authors" {
            let authors =
                db::all_authors(&conn, &src, sort_descending).map_err(|e| e.to_string())?;
            return serde_json::to_value(&authors).map_err(|e| e.to_string());
        }
        if mode == "series" {
            let series =
                db::all_series(&conn, &src, sort_descending, sort_by_author)
                    .map_err(|e| e.to_string())?;
            return serde_json::to_value(&series).map_err(|e| e.to_string());
        }
        if let Some(id) = mode.strip_prefix("author_books:") {
            let id: i64 = id.parse().map_err(|_| "bad author id".to_string())?;
            let books =
                db::books_by_author(&conn, &src, id).map_err(|e| e.to_string())?;
            return serde_json::to_value(&books).map_err(|e| e.to_string());
        }
        if let Some(id) = mode.strip_prefix("series_books:") {
            let id: i64 = id.parse().map_err(|_| "bad series id".to_string())?;
            let books =
                db::books_by_series(&conn, &src, id).map_err(|e| e.to_string())?;
            return serde_json::to_value(&books).map_err(|e| e.to_string());
        }
        Err(format!("unknown browse mode: {mode}"))
    })
}

/// Full search form in one JSON object (same fields as `SearchParams`).
#[no_mangle]
pub extern "C" fn catalog_search(
    config_json: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let praw = c_str(params_json)?;
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let pval: serde_json::Value =
            serde_json::from_str(&praw).map_err(|e| format!("bad params json: {e}"))?;
        let params = search_params(&pval);
        let (conn, src) = open_db(&cfg)?;
        let books = db::search_books(&conn, &src, &params).map_err(|e| e.to_string())?;
        serde_json::to_value(&books).map_err(|e| e.to_string())
    })
}

/// Book detail. `{"error":...}` when the id is unknown (never null).
#[no_mangle]
pub extern "C" fn catalog_detail(config_json: *const c_char, book_id: i64) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let (conn, _src) = open_db(&cfg)?;
        match db::book_detail(&conn, book_id).map_err(|e| e.to_string())? {
            Some(d) => serde_json::to_value(&d).map_err(|e| e.to_string()),
            None => Err(format!("unknown book id: {book_id}")),
        }
    })
}

// ---------------------------------------------------------------------------
// Covers: disk cache → live source, scaled. Returns raw JPEG bytes.
// Note: the Rust-side path cache (`covers_dir()` under the platform data
// dir) also fills on a hit — harmless if the shell keeps its own cache.
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn catalog_cover(
    config_json: *const c_char,
    book_path: *const c_char,
    max_w: u32,
    max_h: u32,
) -> CatalogBytes {
    let empty = CatalogBytes {
        ptr: std::ptr::null_mut(),
        len: 0,
    };
    let body = || -> Result<Vec<u8>, String> {
        let raw = c_str(config_json)?;
        let path = c_str(book_path)?;
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let src = file_source(&cfg)?;
        let data = runtime()
            .block_on(covers::fetch_cover_cached(&src, &path))
            .ok_or_else(|| "no cover".to_string())?;
        if max_w > 0 && max_h > 0 {
            covers::scale_cover(&data, max_w, max_h).ok_or_else(|| "bad image".to_string())
        } else {
            Ok(data)
        }
    };
    let result = std::panic::catch_unwind(body);
    let data = match result {
        Ok(Ok(d)) if !d.is_empty() => d,
        _ => return empty,
    };
    let mut data = data.into_boxed_slice();
    let out = CatalogBytes {
        ptr: data.as_mut_ptr(),
        len: data.len(),
    };
    std::mem::forget(data);
    out
}

// ---------------------------------------------------------------------------
// Kobo: byte-exact path math over plain strings (no I/O here — the shell
// owns SSH, same split as every other port).
// ---------------------------------------------------------------------------

/// Calibre-predicted Kobo path for title + author_sort (+ optional natural).
#[no_mangle]
pub extern "C" fn catalog_kobo_predict(
    title: *const c_char,
    author_sort: *const c_char,
    authors_natural: *const c_char,
) -> *mut c_char {
    run(|| {
        let title = c_str(title)?;
        let sort = c_str(author_sort)?;
        let natural = c_str(authors_natural).ok().filter(|s| !s.is_empty());
        Ok(serde_json::json!({
            "path": kobo::predicted_path(&title, &sort, natural.as_deref())
        }))
    })
}

/// Strict title+author match over a JSON array of candidate paths.
/// `{"ok":{"strict":[...],"title_only":[...]}}` — shell fails closed on
/// 0 or >1 combined, exactly like the Swift/C# ports.
#[no_mangle]
pub extern "C" fn catalog_kobo_match(
    candidates_json: *const c_char,
    title: *const c_char,
    author: *const c_char,
) -> *mut c_char {
    run(|| {
        let craw = c_str(candidates_json)?;
        let title = c_str(title)?;
        let author = c_str(author)?;
        let cands: Vec<String> =
            serde_json::from_str(&craw).map_err(|e| format!("bad candidates json: {e}"))?;
        let (strict, title_only) = kobo::strict_match(&cands, &title, &author);
        Ok(serde_json::json!({"strict": strict, "title_only": title_only}))
    })
}

/// russh shell-exec spike: config `{"ip","cmd","timeout_secs",
/// "password","key_file"}`. Empty password falls back to the key file
/// (default `~/.ssh/id_ed25519`). Returns
/// `{"ok":{"output":...,"code":...}}` — 0 ok, 255 auth rejection,
/// 124 overrun, -1 transport failure. Mirrors SSH.NET `SshSync`.
#[no_mangle]
pub extern "C" fn catalog_kobo_ssh(config_json: *const c_char) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ip = s("ip");
        if ip.trim().is_empty() {
            return Err("missing ip".to_string());
        }
        let cmd = s("cmd");
        let timeout: u64 = v.get("timeout_secs").and_then(|x| x.as_u64()).unwrap_or(10);
        let pw = s("password");
        let kf = s("key_file");
        let auth = if !pw.is_empty() {
            kobo::ssh::SshAuth::Password(pw)
        } else if !kf.is_empty() {
            kobo::ssh::SshAuth::KeyFile(kf.into())
        } else {
            kobo::ssh::SshAuth::KeyFile(kobo::ssh::default_key_file())
        };
        let r = runtime().block_on(kobo::ssh::ssh_sync_russh(&ip, &cmd, timeout, auth));
        Ok(serde_json::json!({"output": r.output, "code": r.code}))
    })
}

/// Kobo open orchestration (probe → predict → find → match → open).
/// Config: `{"ip","password","key_file","title","author","book_id",
/// "timeout_secs"}` plus the library `file_source` shape when Sync & Open
/// size consent matters (`book_id` fetches the EPUB size for `Missing`).
/// Empty password falls back to the key file. Returns the outcome object:
/// `{"status":"opened"|"missing"|"ambiguous"|"failed", ...}` — shells map
/// statuses to dialogs exactly like the Swift/C# ports.
#[no_mangle]
pub extern "C" fn catalog_kobo_open(config_json: *const c_char) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ip = s("ip");
        if ip.trim().is_empty() {
            return Err("missing ip".to_string());
        }
        let pw = s("password");
        let kf = s("key_file");
        let auth = if !pw.is_empty() {
            kobo::ssh::SshAuth::Password(pw)
        } else if !kf.is_empty() {
            kobo::ssh::SshAuth::KeyFile(kf.into())
        } else {
            kobo::ssh::SshAuth::KeyFile(kobo::ssh::default_key_file())
        };
        // Library is optional: only needed for Missing.size_bytes consent.
        let library: Option<(FileSource, i64)> = file_source(&v)
            .ok()
            .zip(v.get("book_id").and_then(|x| x.as_i64()));
        // `library` borrows `v` via src? No — FileSource owns; unwrap the pair.
        let out = match library {
            Some((src, id)) => {
                runtime().block_on(kobo::open::open_on_kobo(
                    &ip,
                    &s("title"),
                    &s("author"),
                    auth,
                    Some((&src, id)),
                ))
            }
            None => runtime().block_on(kobo::open::open_on_kobo(
                &ip,
                &s("title"),
                &s("author"),
                auth,
                None,
            )),
        };
        Ok(out.to_json())
    })
}

/// Kobo Sync & Open: fetch the book's EPUB from the library (same
/// `file_source` config shape as the other calls), push it over the
/// shell channel, then open. Returns the outcome object (see above).
/// Size consent (`Missing.size_bytes`) happens in UI before calling.
#[no_mangle]
pub extern "C" fn catalog_kobo_sync(config_json: *const c_char) -> *mut c_char {
    run(|| {
        let raw = c_str(config_json)?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("bad config json: {e}"))?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let src = file_source(&v)?;
        let ip = s("ip");
        if ip.trim().is_empty() {
            return Err("missing ip".to_string());
        }
        let book_id: i64 = v
            .get("book_id")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| "missing book_id".to_string())?;
        let pw = s("password");
        let kf = s("key_file");
        let auth = if !pw.is_empty() {
            kobo::ssh::SshAuth::Password(pw)
        } else if !kf.is_empty() {
            kobo::ssh::SshAuth::KeyFile(kf.into())
        } else {
            kobo::ssh::SshAuth::KeyFile(kobo::ssh::default_key_file())
        };
        let out =
            runtime().block_on(kobo::sync::sync_and_open(&src, book_id, &ip, auth));
        Ok(out.to_json())
    })
}

/// Opaque helper for shells that need the raw pointer width at compile
/// time (C# `IntPtr` marshalling asserts). Always `{"ok":{"ptr_size":8}}`
/// on 64-bit targets.
#[no_mangle]
pub extern "C" fn catalog_abi() -> *mut c_char {
    ok_json(serde_json::json!({"ptr_size": std::mem::size_of::<*const c_void>()}))
}

// ---------------------------------------------------------------------------
// Kobo handoff — stacking fix (prompt-based, global once per installation)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn catalog_kobo_handoff_check(config_kobo_ip: *const c_char) -> *mut c_char {
    run(|| {
        let ip = c_str(config_kobo_ip)?;
        if ip.trim().is_empty() {
            return Err("not configured: Kobo IP".to_string());
        }
        let check = kobo::handoff::is_handoff_check_cmd();
        let out = runtime().block_on(kobo::ssh_sync(&ip, &check, 6));
        let installed = out.output.contains("ok");
        Ok(serde_json::json!({"installed": installed, "output": out.output, "code": out.code}))
    })
}

#[no_mangle]
pub extern "C" fn catalog_kobo_handoff_ensure(config_kobo_ip: *const c_char) -> *mut c_char {
    run(|| {
        let ip = c_str(config_kobo_ip)?;
        if ip.trim().is_empty() {
            return Err("not configured: Kobo IP".to_string());
        }
        // check first
        let check = kobo::handoff::is_handoff_check_cmd();
        let cur = runtime().block_on(kobo::ssh_sync(&ip, &check, 6));
        if cur.output.contains("ok") {
            return Ok(serde_json::json!({"state": "already", "output": cur.output}));
        }
        for cmd in kobo::handoff::install_cmds() {
            let r = runtime().block_on(kobo::ssh_sync(&ip, &cmd, 10));
            if r.code != 0 {
                return Err(format!("handoff install failed: {}", r.output));
            }
        }
        // verify
        let verify = runtime().block_on(kobo::ssh_sync(&ip, &check, 6));
        if verify.output.contains("ok") {
            Ok(serde_json::json!({"state": "installed", "output": verify.output}))
        } else {
            Err(format!("handoff verify failed: {}", verify.output))
        }
    })
}

/// Global prompt flag — mirrors Settings.kobo_handoff_prompt_done.
#[no_mangle]
pub extern "C" fn catalog_kobo_prompt_done_get() -> *mut c_char {
    run(|| {
        let s = catalog_core::settings::Settings::load();
        Ok(serde_json::json!({"done": s.kobo_handoff_prompt_done}))
    })
}

#[no_mangle]
pub extern "C" fn catalog_kobo_prompt_done_set(done_json: *const c_char) -> *mut c_char {
    run(|| {
        let raw = c_str(done_json)?;
        let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("bad json: {e}"))?;
        let done = v.get("done").and_then(|x| x.as_bool()).unwrap_or(true);
        let mut s = catalog_core::settings::Settings::load();
        s.kobo_handoff_prompt_done = done;
        s.save().map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"done": done}))
    })
}
