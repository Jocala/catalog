// Port of macOS CatalogStore (ReaderCatalogApp.swift) — the single UI store.
// Browse roots + sorts + drill-in + search + per-cell lazy thumbnails
// (SemaphoreSlim(6) mirrors ThumbnailThrottler). Thumbnails are frozen
// BitmapSources published on the UI dispatcher. Errors-only logging.

using System.Collections.ObjectModel;
using System.ComponentModel;
using System.IO;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using CatalogWin.Core;
using CatalogWin.Services;

namespace CatalogWin.UI;

public enum BrowseMode { Books, Author, Series, Tags }
public enum CatalogSortOrder { Author, Az, Za, Date, Oldest }
public enum LibraryViewMode { Grid, List }

public sealed class TileItem : INotifyPropertyChanged
{
    public long Id { get; init; }
    public string Title { get; init; } = "";
    public string Subtitle { get; init; } = "";
    public string Path { get; init; } = "";
    public bool HasCover { get; init; }
    public string Kind { get; init; } = "book"; // book | author | series | tag

    private BitmapSource? _thumb;
    public BitmapSource? Thumb
    {
        get => _thumb;
        set { _thumb = value; OnPropertyChanged(); }
    }

    private bool _thumbRequested;
    public bool ThumbRequested
    {
        get => _thumbRequested;
        set { _thumbRequested = value; OnPropertyChanged(); }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
    private void OnPropertyChanged([CallerMemberName] string? n = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(n));
}

public sealed class CatalogStore : INotifyPropertyChanged
{
    public ObservableCollection<TileItem> Items { get; } = new();
    public ObservableCollection<TileItem> DrilledItems { get; } = new();
    public ObservableCollection<TileItem> SearchItems { get; } = new();

    private readonly SemaphoreSlim _throttler = new(6, 6);
    private readonly HashSet<string> _inFlight = new();
    private readonly Dictionary<string, BitmapSource> _thumbs = new();

    private BrowseMode _browseMode = BrowseMode.Books;
    public BrowseMode BrowseMode
    {
        get => _browseMode;
        set { _browseMode = value; OnPropertyChanged(); }
    }

    private CatalogSortOrder _sortOrder = CatalogSortOrder.Author;
    public CatalogSortOrder SortOrder
    {
        get => _sortOrder;
        set { _sortOrder = value; OnPropertyChanged(); }
    }

    private bool _isLoading = true;
    public bool IsLoading
    {
        get => _isLoading;
        set { _isLoading = value; OnPropertyChanged(); }
    }

    private bool _isDrilling;
    public bool IsDrilling
    {
        get => _isDrilling;
        set { _isDrilling = value; OnPropertyChanged(); }
    }

    private string? _drilledKind;
    public string? DrilledKind
    {
        get => _drilledKind;
        set { _drilledKind = value; OnPropertyChanged(); }
    }

    private string? _drilledTitle;
    public string? DrilledTitle
    {
        get => _drilledTitle;
        set { _drilledTitle = value; OnPropertyChanged(); }
    }

    private bool _isSearchMode;
    public bool IsSearchMode
    {
        get => _isSearchMode;
        set { _isSearchMode = value; OnPropertyChanged(); }
    }

    private string _statusText = "";
    public string StatusText
    {
        get => _statusText;
        set { _statusText = value; OnPropertyChanged(); }
    }

    private string? _dbError;
    public string? DbError
    {
        get => _dbError;
        set { _dbError = value; OnPropertyChanged(); }
    }

    // Fresh start (no library configured) is not an error: the centered
    // No-books empty state points at Settings, so NotConfigured stays
    // silent. Only real failures surface as DbError (MainWindow pops up).
    // Public (not internal): no InternalsVisibleTo in this repo, and the
    // test assembly asserts this mapping directly.
    public static string? MapDbError(Exception ex)
        => ex is CatalogDbException { Kind: CatalogDbKind.NotConfigured } ? null : ex.Message;

    private string? _koboError;
    public string? KoboError
    {
        get => _koboError;
        set { _koboError = value; OnPropertyChanged(); }
    }

    private string _koboStatus = "";
    public string KoboStatus
    {
        get => _koboStatus;
        set { _koboStatus = value; OnPropertyChanged(); }
    }

    // Raw rows behind the tiles (for drill/sort without re-query).
    private List<AuthorBook> _drilledRows = new();
    private List<AuthorSummary> _authorRows = new();
    private List<SeriesSummary> _seriesRows = new();
    private readonly CalibreDb _db = CalibreDb.Shared;

    public event PropertyChangedEventHandler? PropertyChanged;
    private void OnPropertyChanged([CallerMemberName] string? n = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(n));

    public List<CatalogSortOrder> AvailableSortOrders()
    {
        if (DrilledKind is not null)
        {
            return DrilledKind switch
            {
                "author" or "series" => new() { CatalogSortOrder.Az, CatalogSortOrder.Za, CatalogSortOrder.Date, CatalogSortOrder.Oldest },
                _ => new() { CatalogSortOrder.Author, CatalogSortOrder.Az, CatalogSortOrder.Za, CatalogSortOrder.Date, CatalogSortOrder.Oldest },
            };
        }
        return BrowseMode switch
        {
            BrowseMode.Books => new() { CatalogSortOrder.Author, CatalogSortOrder.Az, CatalogSortOrder.Za, CatalogSortOrder.Date, CatalogSortOrder.Oldest },
            BrowseMode.Author => new() { CatalogSortOrder.Az, CatalogSortOrder.Za },
            BrowseMode.Series => new() { CatalogSortOrder.Author, CatalogSortOrder.Az, CatalogSortOrder.Za },
            _ => new() { CatalogSortOrder.Az, CatalogSortOrder.Za },
        };
    }

    private void ClampSortOrder()
    {
        var avail = AvailableSortOrders();
        if (!avail.Contains(SortOrder)) SortOrder = avail[0];
    }

    public string ModeCountText()
    {
        if (DrilledKind is not null) return $"{DrilledItems.Count} books";
        return BrowseMode switch
        {
            BrowseMode.Books => $"{Items.Count} books",
            BrowseMode.Author => $"{Items.Count} authors",
            BrowseMode.Series => $"{Items.Count} series",
            _ => $"{Items.Count} tags",
        };
    }

    public async Task LoadAsync()
    {
        IsLoading = true;
        DrilledKind = null; DrilledTitle = null; _drilledRows.Clear();
        ClampSortOrder();
        bool desc = SortOrder is CatalogSortOrder.Za or CatalogSortOrder.Oldest;
        bool byAuthor = SortOrder == CatalogSortOrder.Author;
        bool byDate = SortOrder is CatalogSortOrder.Date or CatalogSortOrder.Oldest;
        try
        {
            Items.Clear();
            switch (BrowseMode)
            {
                case BrowseMode.Books:
                {
                    var books = await _db.FetchBooksAsync("", desc, byAuthor, byDate);
                    int total = await _db.FetchCountAsync();
                    foreach (var b in books)
                        Items.Add(new TileItem { Id = b.Id, Title = b.Title, Subtitle = b.Author, Path = b.Path, HasCover = b.HasCover });
                    AppLog.Shared.Info("CatalogStore", $"load done mode=books books={books.Count} total={total}");
                    break;
                }
                case BrowseMode.Author:
                {
                    _authorRows = await _db.AllAuthorsAsync(desc);
                    foreach (var a in _authorRows)
                        Items.Add(new TileItem { Id = a.Id, Title = a.Name, Subtitle = $"{a.BookCount} books", Path = a.FirstBookPath ?? "", Kind = "author" });
                    break;
                }
                case BrowseMode.Series:
                {
                    _seriesRows = await _db.AllSeriesAsync(desc, byAuthor);
                    foreach (var s in _seriesRows)
                        Items.Add(new TileItem { Id = s.Id, Title = s.Name, Subtitle = $"{s.BookCount} books", Path = s.FirstBookPath ?? "", Kind = "series" });
                    break;
                }
                case BrowseMode.Tags:
                {
                    var tags = await _db.AllTagsAsync(desc);
                    foreach (var tg in tags)
                        Items.Add(new TileItem { Id = tg.Id, Title = tg.Name, Subtitle = $"{tg.BookCount} books", Path = "", Kind = "tag" });
                    break;
                }
            }
            DbError = null;
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("CatalogStore", $"load failed mode={BrowseMode} {ex.Message}");
            Items.Clear();
            DbError = MapDbError(ex);
        }
        finally
        {
            IsLoading = false;
            StatusText = KoboStatus.Length > 0 ? KoboStatus : ModeCountText();
        }
    }

    public void ExitDrill()
    {
        DrilledKind = null; DrilledTitle = null; _drilledRows.Clear();
        DrilledItems.Clear();
    }

    public async Task DrillAuthorAsync(long id, string name)
    {
        DrilledKind = "author"; DrilledTitle = name; _drilledRows.Clear();
        ClampSortOrder(); IsDrilling = true;
        try
        {
            _drilledRows = await _db.BooksByAuthorAsync(id);
            RefreshDrilledTiles();
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("CatalogStore", $"drillAuthor failed {ex.Message}");
            DbError = MapDbError(ex);
        }
        finally { IsDrilling = false; }
    }

    public async Task DrillSeriesAsync(long id, string name)
    {
        DrilledKind = "series"; DrilledTitle = name; _drilledRows.Clear();
        ClampSortOrder(); IsDrilling = true;
        try
        {
            _drilledRows = await _db.BooksBySeriesAsync(id);
            RefreshDrilledTiles();
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("CatalogStore", $"drillSeries failed {ex.Message}");
            DbError = MapDbError(ex);
        }
        finally { IsDrilling = false; }
    }

    public async Task DrillTagAsync(string tagName)
    {
        DrilledKind = "tags"; DrilledTitle = tagName; _drilledRows.Clear();
        ClampSortOrder(); IsDrilling = true;
        try
        {
            var found = await _db.SearchBooksAsync(tag: tagName);
            _drilledRows = found.Select(b => new AuthorBook(
                b.Id, b.Title, b.Author, b.Path, "", "", "", b.AuthorSort)).ToList();
            RefreshDrilledTiles();
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("CatalogStore", $"drillTag failed {ex.Message}");
            DbError = MapDbError(ex);
        }
        finally { IsDrilling = false; }
    }

    private void RefreshDrilledTiles()
    {
        DrilledItems.Clear();
        foreach (var b in DrilledBooksSorted())
            DrilledItems.Add(new TileItem { Id = b.Id, Title = b.Title, Subtitle = b.Author, Path = b.Path, HasCover = true });
        StatusText = ModeCountText();
    }

    public void ResortDrilled() => RefreshDrilledTiles();

    private List<AuthorBook> DrilledBooksSorted()
    {
        // Drilled rows arrive in catalogue order; apply the toolbar sort here.
        // Calibre timestamps sort lexicographically; books without one fall back to title.
        return SortOrder switch
        {
            CatalogSortOrder.Author => _drilledRows.OrderBy(b =>
                string.IsNullOrEmpty(b.AuthorSort) ? b.Author : b.AuthorSort,
                StringComparer.CurrentCultureIgnoreCase).ToList(),
            CatalogSortOrder.Az => _drilledRows.OrderBy(b => b.Title, StringComparer.CurrentCultureIgnoreCase).ToList(),
            CatalogSortOrder.Za => _drilledRows.OrderByDescending(b => b.Title, StringComparer.CurrentCultureIgnoreCase).ToList(),
            CatalogSortOrder.Date => _drilledRows.OrderByDescending(b => b.Timestamp)
                .ThenBy(b => b.Title, StringComparer.CurrentCultureIgnoreCase).ToList(),
            _ => _drilledRows.OrderBy(b => b.Timestamp)
                .ThenBy(b => b.Title, StringComparer.CurrentCultureIgnoreCase).ToList(),
        };
    }

    public async Task LoadThumbnailAsync(TileItem item)
    {
        if (item.Thumb is not null || item.ThumbRequested) return;
        if (item.Kind == "tag" || string.IsNullOrEmpty(item.Path)) return;
        lock (_inFlight)
        {
            if (!_inFlight.Add(item.Path)) return;
        }
        item.ThumbRequested = true;
        await _throttler.WaitAsync();
        try
        {
            if (_thumbs.TryGetValue(item.Path, out BitmapSource? cached))
            {
                PublishThumb(item, cached);
                return;
            }
            // Fast path: covers/<fileName-hash-independent> — Covers handles coverHash=null;
            // live SMB cover.jpg download otherwise. No mount reads.
            BitmapSource? img = await Covers.ThumbnailAsync(item.Path, null);
            if (img is not null)
            {
                lock (_thumbs) _thumbs[item.Path] = img;
                PublishThumb(item, img);
            }
        }
        finally
        {
            lock (_inFlight) _inFlight.Remove(item.Path);
            _throttler.Release();
        }

        void PublishThumb(TileItem it, BitmapSource bmp)
        {
            Application.Current?.Dispatcher.Invoke(() => it.Thumb = bmp);
        }
    }

    public BitmapSource? CoverSync(string? coverHash, string path)
    {
        if (!string.IsNullOrEmpty(coverHash))
        {
            string f = Path.Combine(AppPaths.CoversDir, coverHash + ".jpg");
            if (File.Exists(f))
            {
                try
                {
                    using var ms = new MemoryStream(File.ReadAllBytes(f));
                    var bmp = new BitmapImage();
                    bmp.BeginInit();
                    bmp.CacheOption = BitmapCacheOption.OnLoad;
                    bmp.StreamSource = ms;
                    bmp.EndInit();
                    bmp.Freeze();
                    return bmp;
                }
                catch
                {
                }
            }
        }
        lock (_thumbs)
        {
            if (_thumbs.TryGetValue(path, out BitmapSource? t)) return t;
        }
        return null;
    }

    public void EnterSearch(List<SearchedBook> results)
    {
        SearchItems.Clear();
        foreach (var r in results)
            SearchItems.Add(new TileItem
            {
                Id = r.Id, Title = r.Title, Subtitle = r.Author,
                Path = r.Path, HasCover = true,
            });
        IsSearchMode = true;
        StatusText = $"{SearchItems.Count} results";
    }

    public void EnterSeriesResults(List<SeriesSummary> series, string tagName)
    {
        SearchItems.Clear();
        foreach (var s in series)
            SearchItems.Add(new TileItem
            {
                Id = s.Id, Title = s.Name, Subtitle = $"{s.BookCount} books",
                Path = s.FirstBookPath ?? "", Kind = "series",
            });
        IsSearchMode = true;
        StatusText = $"{series.Count} series with \"{tagName}\"";
    }

    public void ExitSearch()
    {
        IsSearchMode = false;
        SearchItems.Clear();
        StatusText = ModeCountText();
    }

    public void SetKoboStatus(string s)
    {
        KoboStatus = s;
        StatusText = s.Length > 0 ? s : ModeCountText();
    }

    public async Task OpenOnKoboAsync(CatalogBook book)
    {
        SetKoboStatus($"Opening “{book.Author} {book.Title}” on Kobo…");
        string output = await KoboLauncher.RunKoboAsync(
            $"{book.Author} {book.Title}", book.Title, book.Author,
            onStatus: s => Application.Current?.Dispatcher.Invoke(() => SetKoboStatus(s)),
            onError: e => Application.Current?.Dispatcher.Invoke(() => KoboError = e));
        _ = output;
        await Task.Delay(6000);
        Application.Current?.Dispatcher.Invoke(() =>
        {
            if (KoboStatus.StartsWith("Opened") || KoboStatus.StartsWith("Kobo failed"))
                SetKoboStatus("");
        });
    }
}
