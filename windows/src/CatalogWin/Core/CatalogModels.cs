// Port of macos/Sources/CatalogCore/Models/CatalogModels.swift + ReaderCatalogApp CatalogBook.
// Query-result carriers only; no behaviour.

namespace CatalogWin.Core;

public sealed record CatalogBook(
    long Id,
    string Title,
    string Author,
    string Path,
    bool HasCover,
    string? CoverHash);

public sealed record AuthorBook(
    long Id,
    string Title,
    string Author,
    string Path,
    string CoverHash,
    string Timestamp,
    string RootFolder,
    string AuthorSort = "");

public sealed record AuthorSummary(
    long Id,
    string Name,
    string Sort,
    int BookCount,
    string? FirstBookPath);

public sealed record SeriesSummary(
    long Id,
    string Name,
    int BookCount,
    string? FirstBookPath,
    string? Author = null);

public sealed record TagSummary(
    long Id,
    string Name,
    int BookCount);

public sealed record BookDetail(
    long Id,
    string Title,
    string Author,
    string? Series,
    float SeriesIndex,
    string? Comments,
    string? Tags,
    string? Publisher,
    string? Isbn,
    string? Pubdate,
    string Timestamp,
    string AuthorSort = "");

public sealed record SearchedBook(
    long Id,
    string Title,
    string Author,
    string Path,
    string? Series,
    string[] Tags,
    string CoverHash,
    string AuthorSort = "");
