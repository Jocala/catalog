using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Threading;
using CatalogWinUICore;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace CatalogWinUI;

public sealed class TileItem : INotifyPropertyChanged
{
    public long Id { get; init; }
    public string Title { get; init; } = "";
    public string Subtitle { get; init; } = "";
    public string Path { get; init; } = "";
    public bool HasCover { get; init; }

    private BitmapImage? _cover;
    public BitmapImage? Cover
    {
        get => _cover;
        set { _cover = value; OnPropertyChanged(); }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
    private void OnPropertyChanged([CallerMemberName] string? n = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(n));
}

public sealed partial class MainPage : Page
{
    // Phase 1 reads the already-configured shared-schema file on the
    // build VM. TEMPORARY — Phase 2 owns Settings UI + the new folder.
    private const string LegacySettingsPath =
        @"C:\Users\jeff\AppData\Roaming\com.jocala.Catalog\settings.json";

    private readonly ObservableCollection<TileItem> _tiles = new();
    private readonly SemaphoreSlim _coverGate = new(6, 6);
    private readonly HashSet<string> _inflight = new();
    private readonly object _inflightGate = new();
    private readonly CoverCache _covers;
    private CancellationTokenSource? _loadCts;
    private string _config = "";
    private int _preparedSeen;
    private bool _ready;
    private bool _probed;
    private bool _syncingBoxes;

    private static readonly string[] BrowseModes = ["Books", "Authors", "Series", "Tags"];
    private static readonly string[] BookSorts = ["Author", "A-Z", "Z-A", "Date", "Oldest"];
    private static readonly string[] AlphaSorts = ["A-Z", "Z-A"];

    public MainPage()
    {
        InitializeComponent();
        string coverRoot = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Jocala", "Catalog", "Covers");
        _covers = new CoverCache(coverRoot);
        Tiles.ItemsSource = _tiles;
        Tiles.ElementPrepared += Tiles_Prepared;
        BrowseBox.ItemsSource = BrowseModes;
        BrowseBox.SelectedIndex = 0;
        Loaded += async (_, _) => await ReloadAsync();
    }

    // Covers are visibility-gated: only realized (on-screen) tiles fetch,
    // decode, and marshal. Filling all N thousand at load drowns the UI
    // thread in texture uploads (measured: frozen gallery, zero covers).
    private void Tiles_Prepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs e)
    {
        // Index-based lookup: the realized element's DataContext is not
        // reliably the item here (measured: container without TileItem
        // context), but the args index always addresses the source.
        // Stale events across a reload are harmless: the fetch closure
        // holds the tile object, and cancellation stops the fill.
        TileItem? tile = null;
        if (e.Element is FrameworkElement fe && fe.DataContext is TileItem dc)
            tile = dc;
        else if (e.Index >= 0 && e.Index < _tiles.Count)
            tile = _tiles[e.Index];
        if (tile is null)
        {
            Log.Info("CoverPrep", $"prepared: no tile (type={e.Element.GetType().FullName} index={e.Index})");
            return;
        }
        int n = Interlocked.Increment(ref _preparedSeen);
        if (n <= 5)
        {
            Log.Info("CoverPrep",
                $"#{n} hasCover={tile.HasCover} pathEmpty={string.IsNullOrEmpty(tile.Path)} " +
                $"hasImage={tile.Cover is not null} title={tile.Title}");
        }
        if (tile.Cover is not null || !tile.HasCover || string.IsNullOrEmpty(tile.Path))
            return;
        lock (_inflightGate)
        {
            if (!_inflight.Add(tile.Path)) return;
        }
        CancellationToken ct = _loadCts?.Token ?? CancellationToken.None;
        Task.Run(async () =>
        {
            try
            {
                await _coverGate.WaitAsync(ct).ConfigureAwait(false);
                try
                {
                    byte[]? jpeg;
                    if (_covers.TryGet(tile.Path, out string file))
                    {
                        jpeg = await File.ReadAllBytesAsync(file, ct);
                    }
                    else
                    {
                        jpeg = await Native.CoverAsync(_config, tile.Path, 160, 220);
                        if (jpeg is null) return;
                        _covers.Put(tile.Path, jpeg);
                    }
                    if (ct.IsCancellationRequested) return;
                    byte[] copy = jpeg;
                    DispatcherQueue.TryEnqueue(async () =>
                    {
                        try
                        {
                            if (ct.IsCancellationRequested) return;
                            // Small-JPEG decode on the UI thread: the bytes
                            // are already here; worker decode proved
                            // unreliable, and this is sub-ms per tile.
                            var bmp = new BitmapImage();
                            using var stream = new InMemoryRandomAccessStream();
                            await stream.WriteAsync(copy.AsBuffer());
                            stream.Seek(0);
                            await bmp.SetSourceAsync(stream);
                            tile.Cover = bmp;
                        }
                        catch (Exception ex)
                        {
                            Log.Error("CoverUI", $"{tile.Path}: {ex.Message}");
                        }
                    });
                }
                finally
                {
                    _coverGate.Release();
                }
            }
            catch (OperationCanceledException)
            {
            }
            catch (Exception ex)
            {
                Log.Error("Cover", $"{tile.Path}: {ex.Message}");
            }
            finally
            {
                lock (_inflightGate) _inflight.Remove(tile.Path);
            }
        }, ct);
    }

    private void SetSorts()
    {
        string mode = BrowseBox.SelectedItem as string ?? "Books";
        SortBox.ItemsSource = mode switch
        {
            "Books" => BookSorts,
            "Series" => new[] { "Author", "A-Z", "Z-A" },
            _ => AlphaSorts,
        };
        _syncingBoxes = true;
        SortBox.SelectedIndex = 0;
        _syncingBoxes = false;
    }

    private static (bool desc, bool byAuthor, bool byDate) SortFlags(string sort) => sort switch
    {
        "Z-A" => (true, false, false),
        "Author" => (false, true, false),
        "Date" => (false, false, true),
        "Oldest" => (true, false, true),
        _ => (false, false, false),
    };

    private async void BrowseBox_Changed(object sender, SelectionChangedEventArgs e)
    {
        SetSorts();
        if (_ready) await ReloadAsync();
    }

    private async void SortBox_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_ready && !_syncingBoxes && SortBox.SelectedItem is not null) await ReloadAsync();
    }

    private async void Reload_Click(object sender, RoutedEventArgs e) => await ReloadAsync();

    private void ThemeToggle_Click(object sender, RoutedEventArgs e)
    {
        if (RequestedTheme == ElementTheme.Light)
        {
            RequestedTheme = ElementTheme.Dark;
            ThemeToggle.Content = "Light";
        }
        else
        {
            RequestedTheme = ElementTheme.Light;
            ThemeToggle.Content = "Dark";
        }
    }

    private async Task ReloadAsync()
    {
        _loadCts?.Cancel();
        var cts = new CancellationTokenSource();
        _loadCts = cts;
        CancellationToken ct = cts.Token;
        lock (_inflightGate) _inflight.Clear();

        SetStatus("Loading…");
        EmptyView.Visibility = Visibility.Collapsed;
        try
        {
            AppSettings settings = await Task.Run(() => SettingsStore.LoadFrom(LegacySettingsPath));
            if (!settings.HasSource)
            {
                // Fresh start: silent. The empty state is the whole UX.
                Log.Info("Gallery", "fresh start (no library configured)");
                _tiles.Clear();
                EmptyView.Visibility = Visibility.Visible;
                SetStatus("No library configured");
                _ready = true;
                return;
            }
            SmbServer server = settings.SmbServers[0];
            _config = Native.BuildConfig(settings, server);

            if (!_probed)
            {
                _probed = true;
                string v = await Native.VersionAsync();
                Log.Info("Gallery", $"engine version {v}");
            }
            int total = await Native.GetCountAsync(_config);
            ct.ThrowIfCancellationRequested();

            string mode = BrowseBox.SelectedItem as string ?? "Books";
            string sort = SortBox.SelectedItem as string ?? "Author";
            var (desc, byAuthor, byDate) = SortFlags(sort);

            var tiles = new List<TileItem>();
            if (mode == "Books")
            {
                var books = await Native.FetchBooksAsync(_config, "", desc, byAuthor, byDate);
                tiles.AddRange(books.Select(b => new TileItem
                {
                    Id = b.Id, Title = b.Title, Subtitle = b.Author,
                    Path = b.Path, HasCover = b.HasCover,
                }));
            }
            else if (mode == "Authors")
            {
                var rows = await Native.BrowseAuthorsAsync(_config, desc);
                tiles.AddRange(rows.Select(a => new TileItem
                {
                    Id = a.Id, Title = a.Name,
                    Subtitle = $"{a.BookCount} books", Path = a.FirstBookPath ?? "",
                }));
            }
            else if (mode == "Series")
            {
                var rows = await Native.BrowseSeriesAsync(_config, desc, byAuthor);
                tiles.AddRange(rows.Select(s => new TileItem
                {
                    Id = s.Id, Title = s.Name,
                    Subtitle = $"{s.BookCount} books", Path = s.FirstBookPath ?? "",
                }));
            }
            else
            {
                var rows = await Native.BrowseTagsAsync(_config, desc);
                tiles.AddRange(rows.Select(t => new TileItem
                {
                    Id = t.Id, Title = t.Name,
                    Subtitle = $"{t.BookCount} books",
                }));
            }
            ct.ThrowIfCancellationRequested();

            _tiles.Clear();
            foreach (var t in tiles) _tiles.Add(t);
            SetStatus($"{tiles.Count} shown ({total} books)");
            Log.Info("Gallery", $"load done mode={mode} tiles={tiles.Count} total={total}");
            if (tiles.Count > 0)
            {
                Log.Info("Gallery",
                    $"first tile hasCover={tiles[0].HasCover} pathEmpty={string.IsNullOrEmpty(tiles[0].Path)} " +
                    $"title={tiles[0].Title}");
            }
            _ready = true;
            // Covers fill per visible tile via Tiles_Prepared — nothing
            // to kick here; the repeater realizes on layout.
        }
        catch (OperationCanceledException)
        {
        }
        catch (Exception ex)
        {
            string? msg = Native.MapDbError(ex);
            if (msg is null)
            {
                _tiles.Clear();
                EmptyView.Visibility = Visibility.Visible;
                SetStatus("No library configured");
                return;
            }
            Log.Error("Gallery", $"load failed {msg}");
            SetStatus(msg.Split('\n')[0]);
            await ShowErrorAsync(msg);
        }
    }

    private void SetStatus(string s) => StatusText.Text = s;

    private async Task ShowErrorAsync(string msg)
    {
        try
        {
            var dlg = new ContentDialog
            {
                Title = "Calibre Database",
                Content = msg,
                CloseButtonText = "OK",
                XamlRoot = Content.XamlRoot,
            };
            await dlg.ShowAsync();
        }
        catch
        {
        }
    }
}
