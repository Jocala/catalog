// Query-result carriers only; no behaviour. Property names mirror the
// Rust serde fields exactly (snake_case) — the engine speaks snake,
// the shell speaks PascalCase only in member names.

using System.Text.Json.Serialization;

namespace CatalogWinUICore;

public sealed record BookDto(
    [property: JsonPropertyName("id")] long Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("author")] string Author,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("has_cover")] bool HasCover,
    [property: JsonPropertyName("cover_hash")] string? CoverHash);

public sealed record AuthorDto(
    [property: JsonPropertyName("id")] long Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("sort")] string Sort,
    [property: JsonPropertyName("book_count")] int BookCount,
    [property: JsonPropertyName("first_book_path")] string? FirstBookPath);

public sealed record SeriesDto(
    [property: JsonPropertyName("id")] long Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("book_count")] int BookCount,
    [property: JsonPropertyName("first_book_path")] string? FirstBookPath,
    [property: JsonPropertyName("author")] string? Author = null);

public sealed record TagDto(
    [property: JsonPropertyName("id")] long Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("book_count")] int BookCount);
