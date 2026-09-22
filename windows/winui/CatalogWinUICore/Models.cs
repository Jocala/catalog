// Query-result carriers only; no behaviour. Property names mirror the
// Rust serde fields exactly (snake_case) — the engine speaks snake,
// the shell speaks PascalCase only in member names.

using System.Text.Json.Serialization;

namespace CatalogWinUICore;

public sealed record BookDto(
    [JsonPropertyName("id")] long Id,
    [JsonPropertyName("title")] string Title,
    [JsonPropertyName("author")] string Author,
    [JsonPropertyName("path")] string Path,
    [JsonPropertyName("has_cover")] bool HasCover,
    [JsonPropertyName("cover_hash")] string? CoverHash);

public sealed record AuthorDto(
    [JsonPropertyName("id")] long Id,
    [JsonPropertyName("name")] string Name,
    [JsonPropertyName("sort")] string Sort,
    [JsonPropertyName("book_count")] int BookCount,
    [JsonPropertyName("first_book_path")] string? FirstBookPath);

public sealed record SeriesDto(
    [JsonPropertyName("id")] long Id,
    [JsonPropertyName("name")] string Name,
    [JsonPropertyName("book_count")] int BookCount,
    [JsonPropertyName("first_book_path")] string? FirstBookPath,
    [JsonPropertyName("author")] string? Author = null);

public sealed record TagDto(
    [JsonPropertyName("id")] long Id,
    [JsonPropertyName("name")] string Name,
    [JsonPropertyName("book_count")] int BookCount);
