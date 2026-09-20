-- Minimal Calibre metadata.db fixture for golden tests.
-- Regenerate: sqlite3 metadata.db < make-fixture.sql
-- Schema covers exactly what catalog-core::db queries read.
CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT, author_sort TEXT, path TEXT, has_cover INTEGER, sort TEXT, timestamp TEXT, pubdate TEXT, series_index REAL, last_modified TEXT);
CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT, sort TEXT);
CREATE TABLE books_authors_link(book INTEGER, author INTEGER);
CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_tags_link(book INTEGER, tag INTEGER);
CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT, sort TEXT);
CREATE TABLE books_series_link(book INTEGER, series INTEGER);
CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_publishers_link(book INTEGER, publisher INTEGER);
CREATE TABLE comments(book INTEGER, text TEXT);
CREATE TABLE identifiers(book INTEGER, type TEXT, val TEXT);

INSERT INTO books VALUES
 (1,'Emma','Austen, Jane','Jane Austen/Emma (1)',1,'Emma','2024-01-01','1815-12-23',1.0,'2024-01-01'),
 (2,'Pride and Prejudice','Austen, Jane','Jane Austen/Pride and Prejudice (2)',1,'Pride and Prejudice','2024-01-02','1813-01-28',2.0,'2024-01-02'),
 (3,'The Hound of the Baskervilles','Doyle, Arthur Conan','Arthur Conan Doyle/The Hound of the Baskervilles (3)',1,'Hound of the Baskervilles, The','2023-06-01','1902-04-01',1.0,'2023-06-01'),
 (4,'A Study in Scarlet','Doyle, Arthur Conan','Arthur Conan Doyle/A Study in Scarlet (4)',0,'Study in Scarlet, A','2023-06-02','1887-11-01',2.0,'2023-06-02');
INSERT INTO authors VALUES
 (1,'Jane Austen','Austen, Jane'),
 (2,'Arthur Conan Doyle','Doyle, Arthur Conan');
INSERT INTO books_authors_link VALUES (1,1),(2,1),(3,2),(4,2);
INSERT INTO tags VALUES (1,'Fiction'),(2,'Romance'),(3,'Mystery');
INSERT INTO books_tags_link VALUES (1,1),(1,2),(2,1),(2,2),(3,1),(3,3),(4,1),(4,3);
INSERT INTO series VALUES (1,'Classics','Classics'),(2,'Holmes','Holmes');
INSERT INTO books_series_link VALUES (1,1),(2,1),(3,2),(4,2);
INSERT INTO publishers VALUES (1,'Penguin');
INSERT INTO books_publishers_link VALUES (1,1),(2,1);
INSERT INTO comments VALUES (1,'<p>Highbury matchmaking &amp; mischief.</p>'),(3,'<p>Baskerville <b>hall</b>.</p>');
INSERT INTO identifiers VALUES (1,'isbn','9780141439587');
