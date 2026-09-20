//! Host-side smoke test of the C ABI: same entry points the WPF/Swift
//! shells will P/Invoke, exercised against the 4-book fixture library.
//! Catches JSON-protocol drift and boundary panics before win10.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use catalog_ffi::*;

fn fixture_config() -> CString {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catalog-core/tests/fixtures"
    );
    CString::new(
        serde_json::json!({"source": "local", "local_dir": dir}).to_string(),
    )
    .unwrap()
}

fn take_str(p: *mut c_char) -> String {
    assert!(!p.is_null(), "ffi returned null string");
    // SAFETY: non-null return from the crate; read then free via its API.
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    catalog_string_free(p);
    s
}

fn ok_of(s: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(s).unwrap();
    assert!(
        v.get("error").is_none(),
        "ffi error: {}",
        v.get("error").unwrap_or(&serde_json::Value::Null)
    );
    v["ok"].clone()
}

#[test]
fn ffi_version_and_abi() {
    let v = ok_of(&take_str(catalog_version()));
    assert_eq!(v["version"], "0.1.0");
    let a = ok_of(&take_str(catalog_abi()));
    assert_eq!(a["ptr_size"], 8);
}

#[test]
fn ffi_count_is_four() {
    let cfg = fixture_config();
    let v = ok_of(&take_str(catalog_open_count(cfg.as_ptr())));
    assert_eq!(v["books"], 4);
}

#[test]
fn ffi_browse_modes() {
    let cfg = fixture_config();
    let c = || cfg.as_ptr();

    let tags = ok_of(&take_str(catalog_browse(
        c(),
        CString::new("tags").unwrap().as_ptr(),
        false,
        false,
    )));
    assert_eq!(tags.as_array().unwrap().len(), 3);

    let authors = ok_of(&take_str(catalog_browse(
        c(),
        CString::new("authors").unwrap().as_ptr(),
        false,
        false,
    )));
    assert_eq!(authors.as_array().unwrap().len(), 2);

    let series = ok_of(&take_str(catalog_browse(
        c(),
        CString::new("series").unwrap().as_ptr(),
        false,
        false,
    )));
    assert_eq!(series.as_array().unwrap().len(), 2);

    // Drill: Austen's id is 1 in the fixture (Emma + Pride and Prejudice).
    let drilled = ok_of(&take_str(catalog_browse(
        c(),
        CString::new("author_books:1").unwrap().as_ptr(),
        false,
        false,
    )));
    assert_eq!(drilled.as_array().unwrap().len(), 2);

    let bad = take_str(catalog_browse(
        c(),
        CString::new("nope").unwrap().as_ptr(),
        false,
        false,
    ));
    assert!(bad.contains("unknown browse mode"));
}

#[test]
fn ffi_search_and_detail() {
    let cfg = fixture_config();
    let params = CString::new(r#"{"author":"austen"}"#).unwrap();
    let hits = ok_of(&take_str(catalog_search(cfg.as_ptr(), params.as_ptr())));
    assert_eq!(hits.as_array().unwrap().len(), 2);

    let d = ok_of(&take_str(catalog_detail(cfg.as_ptr(), 1)));
    assert_eq!(d["title"], "Emma");

    let missing = take_str(catalog_detail(cfg.as_ptr(), 424242));
    assert!(missing.contains("unknown book id"));
}

#[test]
fn ffi_cover_miss_is_empty_not_crash() {
    // Fixture books have no cover art on disk — must return empty, not panic.
    let cfg = fixture_config();
    let path = CString::new("Jane Austen/Emma (1)/Emma.epub").unwrap();
    let b = catalog_cover(cfg.as_ptr(), path.as_ptr(), 200, 260);
    assert!(b.ptr.is_null() && b.len == 0);
    catalog_bytes_free(b);
}

#[test]
fn ffi_kobo_math() {
    let title = CString::new("Emma").unwrap();
    let sort = CString::new("Austen, Jane").unwrap();
    let empty = CString::new("").unwrap();
    let p = ok_of(&take_str(catalog_kobo_predict(
        title.as_ptr(),
        sort.as_ptr(),
        empty.as_ptr(),
    )));
    assert!(p["path"].as_str().unwrap().ends_with(".kepub.epub"));

    let cands = CString::new(r#"["/mnt/onboard/Austen, Jane/Emma - Jane Austen.kepub.epub"]"#).unwrap();
    let m = ok_of(&take_str(catalog_kobo_match(
        cands.as_ptr(),
        title.as_ptr(),
        CString::new("Jane Austen").unwrap().as_ptr(),
    )));
    assert_eq!(m["strict"].as_array().unwrap().len(), 1);
}
