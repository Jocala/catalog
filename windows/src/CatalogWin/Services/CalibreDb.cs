// Port of macos/Sources/ReaderCatalogGUI/SmbCatalogDB.swift (617 lines).
// All 11 queries use SQL verbatim from Swift (canonical Calibre schema).
//
// Snapshot model: always fetch the live metadata.db (SMB or local folder) —
// no cached bytes are ever served, so new books appear without Reindex.
// macOS opens the bytes via sqlite3_deserialize (no file anywhere); the .NET
// binding has no deserialize API, so SMB bytes go through a session temp file
// (deleted after each op) opened Mode=ReadOnly. Local opens directly ReadOnly.
// LIKE semantics match (SQLite, ASCII case-insensitive) on both sides.

using System.Text.RegularExpressions;
using System.IO;
using Microsoft.Data.Sqlite;
using CatalogWin.Core;

namespace CatalogWin.Services;

public sealed class CalibreDb
{
    public static CalibreDb Shared { get; } = new();

    // Serializes DB ops like the Swift actor (temp-file contention + log order).
    private readonly SemaphoreSlim _gate = new(1, 1);

    private CalibreDb() { }

    public void Invalidate()
    {
        // Pure-direct mode (B): nothing cached, no-op (kept for call-site parity).
    }

    private async Task<T> WithDbAsync<T>(
        Func<SqliteConnection, LibraryTarget, T> work,
        AppSettings? settings = null)
    {
        var (target, data) = await SmbReader.SnapshotAsync(settings);
        await _gate.WaitAsync();
        try
        {
            if (target.IsLocal)
            {
                string dbPath = Path.Combine(target.LocalDir, "metadata.db");
                using var conn = new SqliteConnection($"Data Source={dbPath};Mode=ReadOnly");
                await conn.OpenAsync();
                return work(conn, target);
            }
            if (data.Length == 0 || data.Length >= 512 * 1024 * 1024)
                throw CatalogDbException.Corrupt($"unusable size {data.Length}");
            string tmp = Path.Combine(Path.GetTempPath(), $"catalog-meta-{Guid.NewGuid():N}.db");
            try
            {
                await File.WriteAllBytesAsync(tmp, data);
                using var conn = new SqliteConnection($"Data Source={tmp};Mode=ReadOnly");
                await conn.OpenAsync();
                return work(conn, target);
            }
            finally
            {
                try { File.Delete(tmp); } catch { }
            }
        }
        finally
        {
            _gate.Release();
        }
    }

    private static string? ColText(SqliteDataReader r, int col)
        => r.IsDBNull(col) ? null : r.GetString(col);

    private static string IndexPlaceholders(string sql)
    {
        // Microsoft.Data.Sqlite binds by name: rewrite each positional '?'
        // to @p0..@pN in order. Our SQL contains no literal '?' otherwise.
        int i = 0;
        return Regex.Replace(sql, @"\?", _ => $"@p{i++}");
    }

    private static string StripCalibreHtml(string html)
    {
        string s = Regex.Replace(html, "<[^>]+>", "");
        foreach (var (e, re) in new[] { ("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&nbsp;", " ") })
            s = s.Replace(e, re);
        return s.Trim();
    }

    // Sort mirrors iOS BrowseView: author = author_sort chain (historical
    // default), az/za = Calibre title-sort b.sort, date/newest/oldest =
    // b.timestamp. No reading_progress table exists, so there is no Current.
    public Task<List<CatalogBook>> FetchBooksAsync(
        string q = "", bool sortDescending = false, bool sortByAuthor = false,
        bool sortByDate = false, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            string sql = "SELECT b.id, b.title, b.author_sort, b.path, b.has_cover FROM books b";
            var args = new List<string>();
            if (q.Length > 0)
            {
                sql += " WHERE b.title LIKE ? OR b.author_sort LIKE ?";
                args.Add($"%{q}%"); args.Add($"%{q}%");
            }
            if (sortByDate)
                sql += $" ORDER BY b.timestamp {(sortDescending ? "ASC" : "DESC")}";
            else if (sortByAuthor)
                sql += $" ORDER BY b.author_sort {(sortDescending ? "DESC" : "")}, b.title";
            else
                sql += $" ORDER BY b.sort {(sortDescending ? "DESC" : "")}";
            using var cmd = conn.CreateCommand();
            cmd.CommandText = IndexPlaceholders(sql);
            for (int i = 0; i < args.Count; i++) cmd.Parameters.AddWithValue($"@p{i}", args[i]);
            var @out = new List<CatalogBook>();
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                @out.Add(new CatalogBook(
                    r.GetInt64(0),
                    ColText(r, 1) ?? "?",
                    ColText(r, 2) ?? "?",
                    SmbReader.BookPath(ColText(r, 3) ?? "", t),
                    r.GetInt64(4) != 0,
                    null));
            }
            return @out;
        }, settings);

    public Task<int> FetchCountAsync(string q = "", AppSettings? settings = null)
        => WithDbAsync((conn, _) =>
        {
            string sql = "SELECT count(*) FROM books";
            var args = new List<string>();
            if (q.Length > 0)
            {
                sql += " WHERE title LIKE ? OR author_sort LIKE ?";
                args.Add($"%{q}%"); args.Add($"%{q}%");
            }
            using var cmd = conn.CreateCommand();
            cmd.CommandText = IndexPlaceholders(sql);
            for (int i = 0; i < args.Count; i++) cmd.Parameters.AddWithValue($"@p{i}", args[i]);
            return Convert.ToInt32(cmd.ExecuteScalar());
        }, settings);

    public Task<List<TagSummary>> AllTagsAsync(bool sortDescending = false, AppSettings? settings = null)
        => WithDbAsync((conn, _) =>
        {
            var @out = new List<TagSummary>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText =
                $"SELECT t.id, t.name, COUNT(*) FROM tags t JOIN books_tags_link btl ON t.id=btl.tag GROUP BY t.id ORDER BY t.name {(sortDescending ? "DESC" : "")}";
            using var r = cmd.ExecuteReader();
            while (r.Read())
                @out.Add(new TagSummary(r.GetInt64(0), r.GetString(1), r.GetInt32(2)));
            return @out;
        }, settings);

    public Task<List<AuthorSummary>> AllAuthorsAsync(bool sortDescending = false, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var @out = new List<AuthorSummary>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText = $"""
                SELECT a.id, a.name, a.sort, COUNT(*) AS book_count,
                       (SELECT b.path FROM books b JOIN books_authors_link bal2 ON b.id = bal2.book
                        WHERE bal2.author = a.id ORDER BY b.sort LIMIT 1) AS first_book_path
                FROM authors a JOIN books_authors_link bal ON a.id = bal.author
                GROUP BY a.id ORDER BY a.sort {(sortDescending ? "DESC" : "")}
                """;
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string? rawFirst = ColText(r, 4);
                @out.Add(new AuthorSummary(
                    r.GetInt64(0), r.GetString(1), ColText(r, 2) ?? "",
                    (int)r.GetInt64(3),
                    rawFirst is null ? null : SmbReader.BookPath(rawFirst, t)));
            }
            return @out;
        }, settings);

    public Task<List<AuthorBook>> BooksByAuthorAsync(long authorId, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var @out = new List<AuthorBook>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText = """
                SELECT b.id, b.title,
                       (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
                       (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
                       b.path, b.timestamp
                FROM books b JOIN books_authors_link bal ON b.id = bal.book
                WHERE bal.author = @p0 ORDER BY b.sort
                """;
            cmd.Parameters.AddWithValue("@p0", authorId);
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string rel = ColText(r, 4) ?? "";
                @out.Add(new AuthorBook(r.GetInt64(0), r.GetString(1), ColText(r, 2) ?? "Unknown",
                    SmbReader.BookPath(rel, t), "", ColText(r, 5) ?? "", "", ColText(r, 3) ?? ""));
            }
            return @out;
        }, settings);

    public Task<List<SeriesSummary>> AllSeriesAsync(
        bool sortDescending = false, bool sortByAuthor = false, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var @out = new List<SeriesSummary>();
            // Sort mirrors iOS LibraryManager.allSeries: author chain first
            // when sortByAuthor, otherwise Calibre series-sort s.sort.
            string orderClause = sortByAuthor
                ? $"author_sort COLLATE NOCASE ASC, s.sort COLLATE NOCASE {(sortDescending ? "DESC" : "ASC")}"
                : $"s.sort {(sortDescending ? "DESC" : "")}";
            using var cmd = conn.CreateCommand();
            cmd.CommandText = $"""
                SELECT s.id, s.name, COUNT(*) AS book_count,
                       (SELECT b.path FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                        WHERE bsl2.series = s.id ORDER BY b.sort LIMIT 1) AS first_path,
                       (SELECT a.sort FROM books b JOIN books_series_link bsl2 ON b.id = bsl2.book
                        JOIN books_authors_link bal ON bal.book = b.id JOIN authors a ON bal.author = a.id
                        WHERE bsl2.series = s.id ORDER BY b.series_index LIMIT 1) AS author_sort
                FROM series s JOIN books_series_link bsl ON s.id = bsl.series
                GROUP BY s.id ORDER BY {orderClause}
                """;
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string? rawFirst = ColText(r, 3);
                @out.Add(new SeriesSummary(
                    r.GetInt64(0), r.GetString(1), (int)r.GetInt64(2),
                    rawFirst is null ? null : SmbReader.BookPath(rawFirst, t),
                    ColText(r, 4)));
            }
            return @out;
        }, settings);

    public Task<List<SeriesSummary>> SeriesByTagAsync(long tagId, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var @out = new List<SeriesSummary>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText = """
                SELECT s.id, s.name, COUNT(*) AS total_books,
                       (SELECT b.path FROM books b JOIN books_series_link bsl2 ON bsl2.book=b.id WHERE bsl2.series=s.id ORDER BY b.series_index LIMIT 1) AS first_path,
                       (SELECT a.name FROM books b JOIN books_series_link bsl2 ON bsl2.book=b.id JOIN books_authors_link bal ON bal.book=b.id JOIN authors a ON a.id=bal.author WHERE bsl2.series=s.id ORDER BY b.series_index LIMIT 1) AS author
                FROM series s JOIN books_series_link bsl ON bsl.series=s.id
                WHERE EXISTS (SELECT 1 FROM books b2 JOIN books_tags_link btl2 ON btl2.book=b2.id JOIN books_series_link bsl2 ON bsl2.book=b2.id                 WHERE bsl2.series=s.id AND btl2.tag=@p0)
                GROUP BY s.id ORDER BY s.sort
                """;
            cmd.Parameters.AddWithValue("@p0", tagId);
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string? rawFirst = ColText(r, 3);
                @out.Add(new SeriesSummary(
                    r.GetInt64(0), r.GetString(1), (int)r.GetInt64(2),
                    rawFirst is null ? null : SmbReader.BookPath(rawFirst, t),
                    ColText(r, 4)));
            }
            return @out;
        }, settings);

    public Task<List<AuthorBook>> BooksBySeriesAsync(long seriesId, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var @out = new List<AuthorBook>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText = """
                SELECT b.id, b.title,
                       (SELECT a.name FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author,
                       (SELECT a.sort FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE bal.book = b.id LIMIT 1) AS author_sort,
                       b.path, b.timestamp
                FROM books b JOIN books_series_link bsl ON b.id = bsl.book
                WHERE bsl.series = @p0 ORDER BY b.series_index
                """;
            cmd.Parameters.AddWithValue("@p0", seriesId);
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string rel = ColText(r, 4) ?? "";
                @out.Add(new AuthorBook(r.GetInt64(0), r.GetString(1), ColText(r, 2) ?? "Unknown",
                    SmbReader.BookPath(rel, t), "", ColText(r, 5) ?? "", "", ColText(r, 3) ?? ""));
            }
            return @out;
        }, settings);

    public Task<List<SearchedBook>> SearchBooksAsync(
        string query = "", string title = "", string author = "",
        string series = "", string tag = "", string publisher = "",
        string dateType = "pubdate", string dateFrom = "", string dateTo = "",
        bool sortDescending = false, AppSettings? settings = null)
        => WithDbAsync((conn, t) =>
        {
            var conditions = new List<string>();
            var bindings = new List<string>();
            void AddLike(string field, string value)
            {
                if (value.Length > 0) { conditions.Add(field); bindings.Add($"%{value}%"); }
            }
            if (query.Length > 0)
            {
                var fieldMap = new[] {
                    ("title", "b.title"), ("author", "a.name"), ("series", "s.name"),
                    ("tag", "tg.name"), ("publisher", "pub.name"), ("comments", "c.text"),
                };
                var known = new HashSet<string>(fieldMap.Select(f => f.Item1));
                int ci = query.IndexOf(':');
                if (ci >= 0 && known.Contains(query[..ci]))
                {
                    string field = query[..ci];
                    string value = query[(ci + 1)..].Trim();
                    if (field == "author")
                    {
                        conditions.Add("(a.name LIKE ? OR a.sort LIKE ?)");
                        bindings.Add($"%{value}%"); bindings.Add($"%{value}%");
                    }
                    else
                    {
                        AddLike($"{fieldMap.First(f => f.Item1 == field).Item2} LIKE ?", value);
                    }
                }
                else
                {
                    string like = $"%{query}%";
                    conditions.Add("(b.title LIKE ? OR a.name LIKE ? OR a.sort LIKE ? OR s.name LIKE ? OR tg.name LIKE ? OR pub.name LIKE ? OR c.text LIKE ?)");
                    for (int i = 0; i < 7; i++) bindings.Add(like);
                }
            }
            AddLike("b.title LIKE ?", title);
            if (author.Length > 0)
            {
                conditions.Add("(a.name LIKE ? OR a.sort LIKE ?)");
                bindings.Add($"%{author}%"); bindings.Add($"%{author}%");
            }
            AddLike("s.name LIKE ?", series);
            AddLike("tg.name LIKE ?", tag);
            AddLike("pub.name LIKE ?", publisher);
            string dateColumn = dateType switch
            {
                "timestamp" => "b.timestamp",
                "last_modified" => "b.last_modified",
                _ => "b.pubdate",
            };
            if (dateFrom.Length > 0) { conditions.Add($"strftime('%Y', {dateColumn}) >= ?"); bindings.Add(dateFrom); }
            if (dateTo.Length > 0) { conditions.Add($"strftime('%Y', {dateColumn}) <= ?"); bindings.Add(dateTo); }
            string whereClause = conditions.Count == 0 ? "" : "WHERE " + string.Join(" AND ", conditions);
            using var cmd = conn.CreateCommand();
            cmd.CommandText = IndexPlaceholders($"""
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
                {whereClause}
                ORDER BY b.sort {(sortDescending ? "DESC" : "")}
                """);
            foreach (string b in bindings) cmd.Parameters.AddWithValue($"@p{cmd.Parameters.Count}", b);
            var @out = new List<SearchedBook>();
            using var r = cmd.ExecuteReader();
            while (r.Read())
            {
                string rel = ColText(r, 3) ?? "";
                string tagsStr = ColText(r, 5) ?? "";
                @out.Add(new SearchedBook(
                    r.GetInt64(0), r.GetString(1), ColText(r, 2) ?? "Unknown",
                    SmbReader.BookPath(rel, t), ColText(r, 4),
                    tagsStr.Length == 0 ? Array.Empty<string>() : tagsStr.Split(", "),
                    "", ColText(r, 6) ?? ""));
            }
            return @out;
        }, settings);

    public Task<BookDetail?> BookDetailAsync(long bookId, AppSettings? settings = null)
        => WithDbAsync((conn, _) =>
        {
            using var cmd = conn.CreateCommand();
            cmd.CommandText = """
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
                WHERE b.id = @p0 LIMIT 1
                """;
            cmd.Parameters.AddWithValue("@p0", bookId);
            using var r = cmd.ExecuteReader();
            if (!r.Read()) return null;
            string? rawComments = ColText(r, 4);
            return new BookDetail(
                bookId, r.GetString(0), r.GetString(1), ColText(r, 2),
                (float)r.GetDouble(3),
                rawComments is null ? null : StripCalibreHtml(rawComments),
                ColText(r, 5), ColText(r, 6), ColText(r, 7), ColText(r, 8),
                ColText(r, 9) ?? "", ColText(r, 10) ?? "");
        }, settings);
}
