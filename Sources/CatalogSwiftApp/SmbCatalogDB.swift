import Foundation
import SQLite3
import CatalogCore

// Single-database rule: the ONLY database is the Calibre metadata.db living
// on the SMB server configured in Settings. It is fetched via SMB straight
// into memory and deserialized into a :memory: SQLite connection — no file
// copy is ever written (no ~/.jreader/*.db, no remote_smb snapshot, no
// /Volumes mount reads, no /tmp file). Missing metadata.db at the Settings
// path surfaces as CatalogDBError.notFound for an error dialog.

struct SmbCatalogTarget: Sendable {
    let host: String
    let share: String
    let remotePath: String
    let user: String
    let password: String
    let domain: String
    /// Directory of metadata.db inside the share, e.g. "calibre".
    let libRoot: String
    var smbPath: String { "smb://\(host)/\(share)/\(remotePath)" }
    var cacheKey: String { "\(host)/\(share)/\(remotePath)" }
}

/// Either-or library source: SMB (existing) or a local Calibre folder.
/// Stored in UserDefaults as `library_source` ("smb" default | "local") +
/// `local_library_dir` (e.g. "/Volumes/Books/calibre/"), shared with the Qt port.
enum LibrarySource: String, Sendable {
    case smb
    case local
}

/// Resolved catalog target — exactly one side is live.
struct LibraryTarget: Sendable {
    var smb: SmbCatalogTarget?
    /// Local Calibre folder (metadata.db lives directly inside), e.g. "/Volumes/Books/calibre".
    var localDir: String = ""
    var isLocal: Bool { smb == nil }
    var displayPath: String {
        if let t = smb { return t.smbPath }
        return "file://\(localDir)/metadata.db"
    }
    var cacheKey: String {
        if let t = smb { return t.cacheKey }
        return "local:\(localDir)"
    }
}

enum CatalogDBError: LocalizedError {
    case notConfigured(String)
    case notFound(smbPath: String, detail: String)
    case authFailed(smbPath: String, detail: String)
    case network(smbPath: String, detail: String)
    case corrupt(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured(let m): return m
        case .notFound(let p, let d):
            return "Calibre database not found at \(p).\nCheck Settings → SMB server and Calibre path.\n\(d)"
        case .authFailed(let p, let d):
            return "SMB login failed for \(p).\nCheck Settings → SMB user and password.\n\(d)"
        case .network(let p, let d):
            return "Could not reach the Calibre database at \(p).\n\(d)"
        case .corrupt(let m): return "Calibre database unreadable: \(m)"
        }
    }
}

private func classifySMBError(_ error: Error, target: SmbCatalogTarget) -> CatalogDBError {
    let s = "\(type(of: error)): \(error) \(error.localizedDescription)".lowercased()
    let short = String(s.prefix(300))
    if s.contains("sharing") {
        return .network(smbPath: target.smbPath, detail: "Sharing violation — Calibre likely has the library open and locked it. Close Calibre or wait a moment and retry. (\(short))")
    }
    if s.contains("logon") || s.contains("logon_failure") || s.contains("access_denied") || (s.contains("auth") && !s.contains("author")) {
        return .authFailed(smbPath: target.smbPath, detail: short)
    }
    if s.contains("not found") || s.contains("no such") || s.contains("object_name") || s.contains("object name")
        || s.contains("object_path") || s.contains("bad_netpath") || s.contains("not_found") || s.contains("status_") {
        return .notFound(smbPath: target.smbPath, detail: short)
    }
    return .network(smbPath: target.smbPath, detail: short)
}

private func smbBookPath(rel: String, _ t: SmbCatalogTarget) -> String {
    if rel.isEmpty { return "" }
    if rel.hasPrefix("smb://") || rel.hasPrefix("http://") || rel.hasPrefix("https://") { return rel }
    if rel.hasPrefix("/") { return rel }
    let prefix = t.libRoot.isEmpty ? "" : t.libRoot + "/"
    return "smb://\(t.host)/\(t.share)/\(prefix)\(rel)"
}

private func localBookPath(rel: String, dir: String) -> String {
    if rel.isEmpty { return "" }
    if rel.hasPrefix("smb://") || rel.hasPrefix("http://") || rel.hasPrefix("https://") { return rel }
    var base = dir.trimmingCharacters(in: .whitespacesAndNewlines)
    if base.hasPrefix("file://") { base = String(base.dropFirst("file://".count)) }
    while base.hasSuffix("/") { base = String(base.dropLast()) }
    if rel.hasPrefix("/") { return rel }
    return base.isEmpty ? rel : "\(base)/\(rel)"
}

private func bookPath(rel: String, _ t: LibraryTarget) -> String {
    if t.isLocal { return localBookPath(rel: rel, dir: t.localDir) }
    guard let s = t.smb else { return rel }
    return smbBookPath(rel: rel, s)
}

private func colText(_ stmt: OpaquePointer?, _ col: Int32) -> String? {
    guard sqlite3_column_type(stmt, col) != SQLITE_NULL,
          let c = sqlite3_column_text(stmt, col) else { return nil }
    return String(cString: c)
}

private func stripCalibreHTML(_ html: String) -> String {
    var s = html.replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
    for (e, r) in [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&nbsp;", " ")] {
        s = s.replacingOccurrences(of: e, with: r)
    }
    return s.trimmingCharacters(in: .whitespacesAndNewlines)
}

actor SmbCatalogDB {
    static let shared = SmbCatalogDB()
    private var cachedData: Data?
    private var cachedKey: String?

    func invalidate() {
        // Pure-direct mode (B): no snapshot cache is retained, so there is
        // nothing to drop. Kept as a no-op so existing call sites
        // (Settings Save, error-dialog Retry) still compile.
        cachedData = nil
        cachedKey = nil
    }

    nonisolated func librarySource() -> LibrarySource {
        let raw = UserDefaults.standard.string(forKey: "library_source") ?? "smb"
        return raw == "local" ? .local : .smb
    }

    nonisolated func localLibraryDir() -> String {
        var dir = UserDefaults.standard.string(forKey: "local_library_dir") ?? ""
        dir = dir.trimmingCharacters(in: .whitespacesAndNewlines)
        if dir.hasPrefix("file://") { dir = String(dir.dropFirst("file://".count)) }
        while dir.hasSuffix("/") && dir.count > 1 { dir = String(dir.dropLast()) }
        return dir
    }

    nonisolated func resolveTarget() throws -> LibraryTarget {
        // Either-or: local wins when selected; SMB fields are never touched.
        if librarySource() == .local {
            let dir = localLibraryDir()
            guard !dir.isEmpty else {
                throw CatalogDBError.notConfigured("No local library folder — open Settings → Source → Local and pick your Calibre folder first.")
            }
            let dbPath = (dir as NSString).appendingPathComponent("metadata.db")
            guard FileManager.default.fileExists(atPath: dbPath) else {
                throw CatalogDBError.notFound(smbPath: "file://\(dbPath)", detail: "metadata.db not found in that folder")
            }
            return LibraryTarget(smb: nil, localDir: dir)
        }
        guard let server = SmbServer.saved.first, !server.host.isEmpty else {
            throw CatalogDBError.notConfigured("No SMB server configured — open Settings → SMB and save the server first.")
        }
        guard let share = server.shares.first, !share.name.isEmpty else {
            throw CatalogDBError.notConfigured("No SMB share configured — open Settings → SMB and save the server first.")
        }
        guard !share.calibreMetadataPath.isEmpty else {
            throw CatalogDBError.notConfigured("No Calibre path configured — open Settings → SMB and save the Calibre path first.")
        }
        let remote = share.calibreMetadataPath
        let root: String
        if let slash = remote.lastIndex(of: "/") { root = String(remote[..<slash]) } else { root = "" }
        return LibraryTarget(smb: SmbCatalogTarget(
            host: server.host, share: share.name, remotePath: remote,
            user: server.user, password: KeychainHelper.read(account: server.host) ?? "",
            domain: server.domain, libRoot: root
        ))
    }

    private func snapshot() async throws -> (LibraryTarget, Data) {
        // Pure-direct mode (B): always fetch the live metadata.db — from SMB
        // or from the local folder. No cached bytes are ever served, so every
        // load sees newly added books (e.g. a new series).
        // The cachedData/cachedKey fields are retained only so invalidate()
        // keeps compiling; they are never read here.
        let t = try resolveTarget()
        if t.isLocal {
            let dbPath = (t.localDir as NSString).appendingPathComponent("metadata.db")
            let data: Data
            do {
                data = try Data(contentsOf: URL(fileURLWithPath: dbPath), options: .mappedIfSafe)
            } catch {
                throw CatalogDBError.notFound(smbPath: "file://\(dbPath)", detail: "\(error.localizedDescription)".prefix(300).description)
            }
            guard !data.isEmpty else {
                throw CatalogDBError.notFound(smbPath: "file://\(dbPath)", detail: "empty file")
            }
            cachedData = data
            cachedKey = t.cacheKey
            ReaderLog.shared.i("SmbCatalogDB", "read \(data.count) bytes from file://\(dbPath)")
            return (t, data)
        }
        guard let s = t.smb else {
            throw CatalogDBError.corrupt("no library target")
        }
        let data: Data
        do {
            data = try await SmbService.downloadData(
                host: s.host, user: s.user, password: s.password,
                share: s.share, domain: s.domain, remotePath: s.remotePath
            )
        } catch {
            throw classifySMBError(error, target: s)
        }
        guard !data.isEmpty else {
            throw CatalogDBError.notFound(smbPath: s.smbPath, detail: "empty file")
        }
        cachedData = data
        cachedKey = t.cacheKey
        ReaderLog.shared.i("SmbCatalogDB", "fetched \(data.count) bytes from \(s.smbPath)")
        return (t, data)
    }

    /// Open the cached snapshot as a :memory: SQLite DB via sqlite3_deserialize.
    /// The bytes are copied into a sqlite3_malloc buffer owned by SQLite
    /// (FREEONCLOSE) — no file is created anywhere.
    func withDB<T>(_ work: (OpaquePointer, LibraryTarget) throws -> T) async throws -> T {
        let (t, data) = try await snapshot()
        var db: OpaquePointer?
        guard sqlite3_open(":memory:", &db) == SQLITE_OK, let mem = db else {
            throw CatalogDBError.corrupt("could not open :memory: database")
        }
        var closed = false
        defer { if !closed { sqlite3_close(mem) } }
        let sz = data.count
        guard sz > 0, sz < 512 * 1024 * 1024, let buf = sqlite3_malloc64(UInt64(sz)) else {
            throw CatalogDBError.corrupt("unusable size \(sz)")
        }
        let copied: Bool = data.withUnsafeBytes { src in
            guard let base = src.baseAddress else { return false }
            memcpy(buf, base, sz)
            return true
        }
        guard copied else {
            sqlite3_free(buf)
            throw CatalogDBError.corrupt("copy failed")
        }
        let u8 = buf.assumingMemoryBound(to: UInt8.self)
        // Writable is the default; FREEONCLOSE hands the malloc buffer to SQLite.
        let flags = UInt32(SQLITE_DESERIALIZE_FREEONCLOSE)
        let rc = sqlite3_deserialize(mem, "main", u8, Int64(sz), Int64(sz), flags)
        guard rc == SQLITE_OK else {
            sqlite3_free(buf)
            throw CatalogDBError.corrupt("deserialize rc=\(rc)")
        }
        do {
            let out = try work(mem, t)
            sqlite3_close(mem)
            closed = true
            return out
        } catch {
            sqlite3_close(mem)
            closed = true
            throw error
        }
    }

    // MARK: - Catalog queries (canonical Calibre schema)

    func fetchBooks(search q: String, sortDescending: Bool = false, sortByAuthor: Bool = false, sortByDate: Bool = false) async throws -> [CatalogBook] {
        try await withDB { db, t in
            var sql = "SELECT b.id, b.title, b.author_sort, b.path, b.has_cover FROM books b"
            var args: [String] = []
            if !q.isEmpty {
                sql += " WHERE b.title LIKE ? OR b.author_sort LIKE ?"
                args = ["%\(q)%", "%\(q)%"]
            }
            // Sort mirrors iOS BrowseView: .author = author_sort chain (the
            // historical default here), .az/.za = Calibre title-sort b.sort,
            // .date/.newest/.oldest = b.timestamp. No reading_progress table
            // exists in a live Calibre metadata.db, so there is no Current.
            if sortByDate {
                sql += " ORDER BY b.timestamp \(sortDescending ? "ASC" : "DESC")"
            } else if sortByAuthor {
                sql += " ORDER BY b.author_sort \(sortDescending ? "DESC" : ""), b.title"
            } else {
                sql += " ORDER BY b.sort \(sortDescending ? "DESC" : "")"
            }
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return [] }
            defer { sqlite3_finalize(s) }
            for (i, a) in args.enumerated() { sqlite3_bind_text(s, Int32(i + 1), (a as NSString).utf8String, -1, nil) }
            var out: [CatalogBook] = []
            while sqlite3_step(s) == SQLITE_ROW {
                let id = sqlite3_column_int64(s, 0)
                let title = colText(s, 1) ?? "?"
                let author = colText(s, 2) ?? "?"
                let rel = colText(s, 3) ?? ""
                let hasCover = sqlite3_column_int64(s, 4) != 0
                out.append(CatalogBook(id: id, title: title, author: author,
                                       path: bookPath(rel: rel, t), hasCover: hasCover, coverHash: nil))
            }
            return out
        }
    }

    func fetchCount(search q: String) async throws -> Int {
        try await withDB { db, _ in
            var sql = "SELECT count(*) FROM books"
            var args: [String] = []
            if !q.isEmpty {
                sql += " WHERE title LIKE ? OR author_sort LIKE ?"
                args = ["%\(q)%", "%\(q)%"]
            }
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return 0 }
            defer { sqlite3_finalize(s) }
            for (i, a) in args.enumerated() { sqlite3_bind_text(s, Int32(i + 1), (a as NSString).utf8String, -1, nil) }
            return sqlite3_step(s) == SQLITE_ROW ? Int(sqlite3_column_int64(s, 0)) : 0
        }
    }

    /// EPUB source file for a book: calibre `data` row (format='EPUB') joined
    /// to `books.path`. Returned paths are share-relative (SMB) or absolute
    /// (local) — ready for SmbService.downloadData / Data(contentsOf:).
    struct EpubSource: Sendable {
        /// Share-relative (SMB) or absolute (local) file path.
        let filePath: String
        /// True when filePath is SMB share-relative (needs downloadData).
        let isSMB: Bool
        /// SMB connection fields (valid only when isSMB).
        let host: String
        let share: String
        let user: String
        let password: String
        let domain: String
        /// Calibre's uncompressed_size — shown in the Sync dialog as ~size.
        let size: Int64
    }

    func epubSource(bookId: Int64) async throws -> EpubSource? {
        guard bookId > 0 else { return nil }
        return try await withDB { db, t in
            let sql = """
                SELECT d.name, d.uncompressed_size, b.path FROM data d
                JOIN books b ON b.id = d.book
                WHERE d.book = ? AND d.format = 'EPUB' LIMIT 1
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return nil }
            defer { sqlite3_finalize(s) }
            sqlite3_bind_int64(s, 1, bookId)
            guard sqlite3_step(s) == SQLITE_ROW else { return nil }
            let name = colText(s, 0) ?? ""
            let size = sqlite3_column_int64(s, 1)
            let dirRel = colText(s, 2) ?? ""
            guard !name.isEmpty, !dirRel.isEmpty else { return nil }
            let file = "\(name).epub"
            if t.isLocal {
                let dir = localBookPath(rel: dirRel, dir: t.localDir)
                return EpubSource(filePath: (dir as NSString).appendingPathComponent(file),
                                  isSMB: false, host: "", share: "", user: "",
                                  password: "", domain: "", size: size)
            }
            guard let smb = t.smb else { return nil }
            let prefix = smb.libRoot.isEmpty ? "" : smb.libRoot + "/"
            return EpubSource(filePath: "\(prefix)\(dirRel)/\(file)",
                              isSMB: true, host: smb.host, share: smb.share,
                              user: smb.user, password: smb.password,
                              domain: smb.domain, size: size)
        }
    }

    func allTags(sortDescending: Bool = false) async throws -> [TagSummary] {
        try await withDB { db, _ in
            var out: [TagSummary] = []
            var stmt: OpaquePointer?
            let sql = "SELECT t.id, t.name, COUNT(*) FROM tags t JOIN books_tags_link btl ON t.id=btl.tag GROUP BY t.id ORDER BY t.name \(sortDescending ? "DESC" : "")"
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            while sqlite3_step(s) == SQLITE_ROW {
                out.append(TagSummary(id: sqlite3_column_int64(s, 0), name: String(cString: sqlite3_column_text(s, 1)), bookCount: Int(sqlite3_column_int(s, 2))))
            }
            return out
        }
    }

    func allAuthors(sortDescending: Bool = false) async throws -> [AuthorSummary] {
        try await withDB { db, t in
            var out: [AuthorSummary] = []
            var stmt: OpaquePointer?
            let sql = """
                SELECT a.id, a.name, a.sort, COUNT(*) AS book_count,
                       (SELECT b.path FROM books b JOIN books_authors_link bal2 ON b.id = bal2.book
                        WHERE bal2.author = a.id ORDER BY b.sort LIMIT 1) AS first_book_path
                FROM authors a JOIN books_authors_link bal ON a.id = bal.author
                GROUP BY a.id ORDER BY a.sort \(sortDescending ? "DESC" : "")
                """
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            while sqlite3_step(s) == SQLITE_ROW {
                let rawFirst = colText(s, 4)
                out.append(AuthorSummary(
                    id: sqlite3_column_int64(s, 0),
                    name: String(cString: sqlite3_column_text(s, 1)),
                    sort: colText(s, 2) ?? "",
                    bookCount: Int(sqlite3_column_int64(s, 3)),
                    firstBookPath: rawFirst.map { bookPath(rel: $0, t) }
                ))
            }
            return out
        }
    }

    func booksByAuthor(id authorId: Int64) async throws -> [AuthorBook] {
        try await withDB { db, t in
            var out: [AuthorBook] = []
            let sql = """
                SELECT b.id, b.title,
                       (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
                       (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
                       b.path, b.timestamp
                FROM books b JOIN books_authors_link bal ON b.id = bal.book
                WHERE bal.author = ? ORDER BY b.sort
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            sqlite3_bind_int64(s, 1, authorId)
            while sqlite3_step(s) == SQLITE_ROW {
                let rel = colText(s, 4) ?? ""
                out.append(AuthorBook(id: sqlite3_column_int64(s, 0),
                                      title: String(cString: sqlite3_column_text(s, 1)),
                                      author: colText(s, 2) ?? "Unknown",
                                      path: bookPath(rel: rel, t),
                                      coverHash: "", timestamp: colText(s, 5) ?? "", root_folder: "",
                                      authorSort: colText(s, 3) ?? ""))
            }
            return out
        }
    }

    func allSeries(sortDescending: Bool = false, sortByAuthor: Bool = false) async throws -> [SeriesSummary] {
        try await withDB { db, t in
            var out: [SeriesSummary] = []
            var stmt: OpaquePointer?
            // Sort mirrors iOS LibraryManager.allSeries: author chain first
            // when sortByAuthor, otherwise Calibre series-sort s.sort.
            let orderClause: String
            if sortByAuthor {
                orderClause = "author_sort COLLATE NOCASE ASC, s.sort COLLATE NOCASE \(sortDescending ? "DESC" : "ASC")"
            } else {
                orderClause = "s.sort \(sortDescending ? "DESC" : "")"
            }
            let sql = """
                SELECT s.id, s.name, COUNT(*) AS book_count,
                       (SELECT b.path FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                        WHERE bsl2.series = s.id ORDER BY b.sort LIMIT 1) AS first_path,
                       (SELECT a.sort FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                        JOIN books_authors_link bal ON bal.book = b.id JOIN authors a ON bal.author = a.id
                        WHERE bsl2.series = s.id ORDER BY b.series_index LIMIT 1) AS author_sort
                FROM series s JOIN books_series_link bsl ON s.id = bsl.series
                GROUP BY s.id ORDER BY \(orderClause)
                """
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            while sqlite3_step(s) == SQLITE_ROW {
                let rawFirst = colText(s, 3)
                out.append(SeriesSummary(
                    id: sqlite3_column_int64(s, 0),
                    name: String(cString: sqlite3_column_text(s, 1)),
                    bookCount: Int(sqlite3_column_int64(s, 2)),
                    firstBookPath: rawFirst.map { bookPath(rel: $0, t) },
                    author: colText(s, 4)
                ))
            }
            return out
        }
    }

    func seriesByTag(tagId: Int64) async throws -> [SeriesSummary] {
        try await withDB { db, t in
            var out: [SeriesSummary] = []
            let sql = """
                SELECT s.id, s.name, COUNT(*) AS total_books,
                       (SELECT b.path FROM books b JOIN books_series_link bsl2 ON bsl2.book=b.id WHERE bsl2.series=s.id ORDER BY b.series_index LIMIT 1) AS first_path,
                       (SELECT a.name FROM books b JOIN books_series_link bsl2 ON bsl2.book=b.id JOIN books_authors_link bal ON bal.book=b.id JOIN authors a ON a.id=bal.author WHERE bsl2.series=s.id ORDER BY b.series_index LIMIT 1) AS author
                FROM series s JOIN books_series_link bsl ON bsl.series=s.id
                WHERE EXISTS (SELECT 1 FROM books b2 JOIN books_tags_link btl2 ON btl2.book=b2.id JOIN books_series_link bsl2 ON bsl2.book=b2.id WHERE bsl2.series=s.id AND btl2.tag=?)
                GROUP BY s.id ORDER BY s.sort
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            sqlite3_bind_int64(s, 1, tagId)
            while sqlite3_step(s) == SQLITE_ROW {
                let rawFirst = colText(s, 3)
                out.append(SeriesSummary(
                    id: sqlite3_column_int64(s, 0),
                    name: String(cString: sqlite3_column_text(s, 1)),
                    bookCount: Int(sqlite3_column_int64(s, 2)),
                    firstBookPath: rawFirst.map { bookPath(rel: $0, t) },
                    author: colText(s, 4)
                ))
            }
            return out
        }
    }

    func booksBySeries(id seriesId: Int64) async throws -> [AuthorBook] {
        try await withDB { db, t in
            var out: [AuthorBook] = []
            let sql = """
                SELECT b.id, b.title,
                       (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
                       (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
                       b.path, b.timestamp
                FROM books b JOIN books_series_link bsl ON b.id = bsl.book
                WHERE bsl.series = ? ORDER BY b.series_index
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return out }
            defer { sqlite3_finalize(s) }
            sqlite3_bind_int64(s, 1, seriesId)
            while sqlite3_step(s) == SQLITE_ROW {
                let rel = colText(s, 4) ?? ""
                out.append(AuthorBook(id: sqlite3_column_int64(s, 0),
                                      title: String(cString: sqlite3_column_text(s, 1)),
                                      author: colText(s, 2) ?? "Unknown",
                                      path: bookPath(rel: rel, t),
                                      coverHash: "", timestamp: colText(s, 5) ?? "", root_folder: "",
                                      authorSort: colText(s, 3) ?? ""))
            }
            return out
        }
    }

    func searchBooks(query q: String = "", title: String = "", author: String = "",
                     series: String = "", tag: String = "", publisher: String = "",
                     dateType: String = "pubdate", dateFrom: String = "", dateTo: String = "",
                     sortDescending: Bool = false) async throws -> [SearchedBook] {
        try await withDB { db, t in
            var conditions: [String] = []
            var bindings: [String] = []
            func addLike(_ field: String, _ value: String) {
                if !value.isEmpty { conditions.append(field); bindings.append("%\(value)%") }
            }
            if !q.isEmpty {
                let fieldMap: [(String, String)] = [
                    ("title", "b.title"), ("author", "a.name"), ("series", "s.name"),
                    ("tag", "tg.name"), ("publisher", "pub.name"), ("comments", "c.text"),
                ]
                let known = Set(fieldMap.map { $0.0 })
                if let ci = q.firstIndex(of: ":"), known.contains(String(q[..<ci])) {
                    let field = String(q[..<ci])
                    let value = String(q[q.index(after: ci)...]).trimmingCharacters(in: .whitespaces)
                    if field == "author" {
                        conditions.append("(a.name LIKE ? OR a.sort LIKE ?)")
                        bindings.append("%\(value)%"); bindings.append("%\(value)%")
                    } else {
                        addLike("\(fieldMap.first(where: { $0.0 == field })!.1) LIKE ?", value)
                    }
                } else {
                    let like = "%\(q)%"
                    conditions.append("(b.title LIKE ? OR a.name LIKE ? OR a.sort LIKE ? OR s.name LIKE ? OR tg.name LIKE ? OR pub.name LIKE ? OR c.text LIKE ?)")
                    for _ in 0..<7 { bindings.append(like) }
                }
            }
            addLike("b.title LIKE ?", title)
            if !author.isEmpty {
                conditions.append("(a.name LIKE ? OR a.sort LIKE ?)")
                bindings.append("%\(author)%"); bindings.append("%\(author)%")
            }
            addLike("s.name LIKE ?", series)
            addLike("tg.name LIKE ?", tag)
            addLike("pub.name LIKE ?", publisher)
            let dateColumn: String
            switch dateType {
            case "timestamp": dateColumn = "b.timestamp"
            case "last_modified": dateColumn = "b.last_modified"
            default: dateColumn = "b.pubdate"
            }
            if !dateFrom.isEmpty { conditions.append("strftime('%Y', \(dateColumn)) >= ?"); bindings.append(dateFrom) }
            if !dateTo.isEmpty { conditions.append("strftime('%Y', \(dateColumn)) <= ?"); bindings.append(dateTo) }
            let whereClause = conditions.isEmpty ? "" : "WHERE " + conditions.joined(separator: " AND ")
            let sql = """
                SELECT DISTINCT b.id, b.title, a.name, b.path, s.name,
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
                \(whereClause)
                ORDER BY b.sort \(sortDescending ? "DESC" : "")
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return [] }
            defer { sqlite3_finalize(s) }
            for (i, b) in bindings.enumerated() { sqlite3_bind_text(s, Int32(i + 1), (b as NSString).utf8String, -1, nil) }
            var out: [SearchedBook] = []
            while sqlite3_step(s) == SQLITE_ROW {
                let rel = colText(s, 3) ?? ""
                let tagsStr = colText(s, 5) ?? ""
                out.append(SearchedBook(
                    id: sqlite3_column_int64(s, 0),
                    title: String(cString: sqlite3_column_text(s, 1)),
                    author: colText(s, 2) ?? "Unknown",
                    path: bookPath(rel: rel, t),
                    series: colText(s, 4),
                    tags: tagsStr.isEmpty ? [] : tagsStr.components(separatedBy: ", "),
                    coverHash: "",
                    authorSort: colText(s, 6) ?? ""
                ))
            }
            return out
        }
    }

    func bookDetail(for bookId: Int64) async throws -> BookDetail? {
        try await withDB { db, _ in
            let sql = """
                SELECT b.title, a.name, s.name, b.series_index, c.text,
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
                WHERE b.id = ? LIMIT 1
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let s = stmt else { return nil }
            defer { sqlite3_finalize(s) }
            sqlite3_bind_int64(s, 1, bookId)
            guard sqlite3_step(s) == SQLITE_ROW else { return nil }
            let rawComments = colText(s, 4)
            return BookDetail(
                id: bookId,
                title: String(cString: sqlite3_column_text(s, 0)),
                author: String(cString: sqlite3_column_text(s, 1)),
                series: colText(s, 2),
                seriesIndex: Float(sqlite3_column_double(s, 3)),
                comments: rawComments.map(stripCalibreHTML),
                tags: colText(s, 5),
                publisher: colText(s, 6),
                isbn: colText(s, 7),
                pubdate: colText(s, 8),
                timestamp: colText(s, 9) ?? "",
                authorSort: colText(s, 10) ?? ""
            )
        }
    }
}
