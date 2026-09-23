//! Golden tests against the checked-in Calibre fixture
//! (`tests/fixtures/metadata.db`, built from `make-fixture.sql`).
//! Exercises the full FileSource read path: bytes -> memory DB -> queries.

use catalog_core::db::{
    all_authors, all_series, all_tags, book_detail, books_by_author, books_by_series, fetch_books,
    fetch_count, open_memory_db, search_books, FileSource, SearchParams,
};
use std::path::PathBuf;

fn fixture_source() -> FileSource {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("tests");
    dir.push("fixtures");
    FileSource::local(dir)
}

fn fixture_db() -> rusqlite::Connection {
    // Golden path under test: read_db_bytes + open_memory_db.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let bytes = rt.block_on(fixture_source().read_db_bytes()).unwrap();
    assert!(!bytes.is_empty());
    open_memory_db(&bytes).unwrap()
}

#[test]
fn golden_count_is_four() {
    let db = fixture_db();
    assert_eq!(fetch_count(&db, "").unwrap(), 4);
    assert_eq!(fetch_count(&db, "Emma").unwrap(), 1);
    assert_eq!(fetch_count(&db, "Bond").unwrap(), 0);
}

#[test]
fn golden_fetch_books_paths() {
    let db = fixture_db();
    let source = fixture_source();
    let books = fetch_books(&db, &source, "", false, true, false).unwrap();
    assert_eq!(books.len(), 4);
    assert_eq!(books[0].author, "Austen, Jane");
    assert!(books[0].path.ends_with("Jane Austen/Emma (1)"));
    let emma = books.iter().find(|b| b.title == "Emma").unwrap();
    assert_eq!(emma.series.as_deref(), Some("Classics"));
    assert_eq!(emma.tags, vec!["Fiction".to_string(), "Romance".to_string()]);
}

#[test]
fn golden_search_field_syntax() {
    let db = fixture_db();
    let source = fixture_source();
    let p = SearchParams {
        query: "author:doyle".to_string(),
        ..Default::default()
    };
    let found = search_books(&db, &source, &p).unwrap();
    assert_eq!(found.len(), 2);
    let p2 = SearchParams {
        query: "tag:mystery".to_string(),
        ..Default::default()
    };
    assert_eq!(search_books(&db, &source, &p2).unwrap().len(), 2);
}

#[test]
fn golden_tags_series_authors() {
    let db = fixture_db();
    let source = fixture_source();
    let mut tags: Vec<String> =
        all_tags(&db, false).unwrap().into_iter().map(|t| t.name).collect();
    tags.sort();
    assert_eq!(tags, vec!["Fiction", "Mystery", "Romance"]);
    let series = all_series(&db, &source, false, false).unwrap();
    assert_eq!(series.len(), 2);
    let holmes = series.iter().find(|s| s.name == "Holmes").unwrap();
    assert_eq!(books_by_series(&db, &source, holmes.id).unwrap().len(), 2);
    let authors = all_authors(&db, &source, false).unwrap();
    assert_eq!(authors.len(), 2);
    let austen = authors.iter().find(|a| a.name == "Jane Austen").unwrap();
    assert_eq!(books_by_author(&db, &source, austen.id).unwrap().len(), 2);
    let austen_books = books_by_author(&db, &source, austen.id).unwrap();
    let emma_drill = austen_books.iter().find(|b| b.title == "Emma").unwrap();
    assert_eq!(emma_drill.series.as_deref(), Some("Classics"));
    assert!(emma_drill.tags.contains(&"Fiction".to_string()));
}

#[test]
fn golden_detail_strips_html() {
    let db = fixture_db();
    let d = book_detail(&db, 1).unwrap().expect("book 1");
    assert_eq!(d.title, "Emma");
    assert_eq!(d.series.as_deref(), Some("Classics"));
    assert_eq!(d.comments.as_deref(), Some("Highbury matchmaking & mischief."));
    assert_eq!(d.isbn.as_deref(), Some("9780141439587"));
    assert_eq!(d.publisher.as_deref(), Some("Penguin"));
}

#[test]
fn golden_series_search_orders_by_index() {
    let db = fixture_db();
    let source = fixture_source();
    // Title sorts first alphabetically but is last in the series: index
    // order must win over title sort for a series-scoped search.
    db.execute(
        "INSERT INTO books VALUES(5,'Aardvark','Doyle, Arthur Conan','Arthur Conan Doyle/Aardvark (5)',0,'Aardvark','2023-06-03','1903-01-01',9.0,'2023-06-03')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO books_authors_link VALUES(5,2)", [])
        .unwrap();
    db.execute("INSERT INTO books_series_link VALUES(5,2)", [])
        .unwrap();
    let p = SearchParams {
        series: "Holmes".to_string(),
        ..Default::default()
    };
    let found = search_books(&db, &source, &p).unwrap();
    let titles: Vec<&str> = found.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "The Hound of the Baskervilles",
            "A Study in Scarlet",
            "Aardvark"
        ]
    );
}

#[test]
fn golden_missing_dir_errors() {
    let source = FileSource::local("/nonexistent-catalog-dir-xyz");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(rt.block_on(source.read_db_bytes()).is_err());
}
