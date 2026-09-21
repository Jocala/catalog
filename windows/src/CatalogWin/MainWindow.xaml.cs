using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using CatalogWin.Core;
using CatalogWin.Services;

namespace CatalogWin;

public partial class MainWindow : Window
{
    private readonly UI.CatalogStore _store = new();
    private UI.LibraryViewMode _viewMode = UI.LibraryViewMode.Grid;
    // Windowed feeds: UniformGrid renders ~14 rows; spacers preserve extent.
    private readonly UI.ItemWindow _gridWin = new();
    private readonly UI.ItemWindow _drillWin = new();
    private readonly UI.ItemWindow _searchWin = new();
    private bool _loadingCombos;
    private bool _uiReady; // set after InitializeComponent; XAML fires Checked during load
    private string? _shownDbError;
    private string? _shownKoboError;

    public MainWindow()
    {
        InitializeComponent();
        UI.Theme.ApplyCurrent();
        // Data root (covers/thumbnails caches) — nothing else is created here:
        // no library.db, no Books tree (single-DB rule).
        _ = AppPaths.CoversDir;
        _ = AppPaths.ThumbnailsDir;

        BrowseBox.ItemsSource = Enum.GetNames(typeof(UI.BrowseMode));
        BrowseBox.SelectedIndex = 0;
        GridItems.ItemsSource = _gridWin.Window;
        ListItems.ItemsSource = _store.Items;
        DrillItems.ItemsSource = _drillWin.Window;
        SearchItems.ItemsSource = _searchWin.Window;

        _store.Items.CollectionChanged += (_, _) => FeedGrid();
        _store.DrilledItems.CollectionChanged += (_, _) => FeedDrill();
        _store.SearchItems.CollectionChanged += (_, _) => FeedSearch();
        _gridWin.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(UI.ItemWindow.TopPad)) GridTopPad.Height = _gridWin.TopPad;
            if (e.PropertyName == nameof(UI.ItemWindow.BottomPad)) GridBottomPad.Height = _gridWin.BottomPad;
        };
        _drillWin.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(UI.ItemWindow.TopPad)) DrillTopPad.Height = _drillWin.TopPad;
            if (e.PropertyName == nameof(UI.ItemWindow.BottomPad)) DrillBottomPad.Height = _drillWin.BottomPad;
        };
        _searchWin.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(UI.ItemWindow.TopPad)) SearchTopPad.Height = _searchWin.TopPad;
            if (e.PropertyName == nameof(UI.ItemWindow.BottomPad)) SearchBottomPad.Height = _searchWin.BottomPad;
        };

        _store.PropertyChanged += Store_Changed;
        _uiReady = true;
        Loaded += async (_, _) => await _store.LoadAsync();
    }

    private void Store_Changed(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(UI.CatalogStore.BrowseMode)
            or nameof(UI.CatalogStore.SortOrder)
            or nameof(UI.CatalogStore.IsLoading)
            or nameof(UI.CatalogStore.DrilledKind)
            or nameof(UI.CatalogStore.IsSearchMode)
            or nameof(UI.CatalogStore.StatusText))
            Dispatcher.Invoke(RefreshView);

        if (e.PropertyName == nameof(UI.CatalogStore.DbError) && _store.DbError is not null && _store.DbError != _shownDbError)
        {
            _shownDbError = _store.DbError;
            Dispatcher.Invoke(() =>
            {
                var r = MessageBox.Show($"{_store.DbError}\n\nOpen Settings?", "Calibre Database",
                    MessageBoxButton.YesNoCancel, MessageBoxImage.Warning);
                if (r == MessageBoxResult.Yes) OpenSettings();
                else if (r == MessageBoxResult.No) _ = _store.LoadAsync();
                _store.DbError = null;
            });
        }
        if (e.PropertyName == nameof(UI.CatalogStore.KoboError) && _store.KoboError is not null && _store.KoboError != _shownKoboError)
        {
            _shownKoboError = _store.KoboError;
            Dispatcher.Invoke(() =>
            {
                MessageBox.Show(_store.KoboError, "Kobo", MessageBoxButton.OK, MessageBoxImage.Warning);
                _store.KoboError = null;
            });
        }
    }

    private void RefreshView()
    {
        if (!_uiReady) return;
        _loadingCombos = true;        BrowseBox.SelectedItem = _store.BrowseMode.ToString();
        var sorts = _store.AvailableSortOrders();
        SortBox.ItemsSource = sorts.Select(SortDisplay).ToList();
        SortBox.SelectedItem = SortDisplay(_store.SortOrder);
        _loadingCombos = false;

        StatusText.Text = _store.StatusText;
        bool searching = _store.IsSearchMode;
        LibraryBtn.Visibility = searching ? Visibility.Visible : Visibility.Collapsed;

        LoadingView.Visibility = _store.IsLoading ? Visibility.Visible : Visibility.Collapsed;
        bool drilled = _store.DrilledKind is not null;
        DrillView.Visibility = drilled && !searching ? Visibility.Visible : Visibility.Collapsed;
        SearchView.Visibility = searching ? Visibility.Visible : Visibility.Collapsed;
        bool showMain = !drilled && !searching && !_store.IsLoading;
        GridScroll.Visibility = showMain && _viewMode == UI.LibraryViewMode.Grid ? Visibility.Visible : Visibility.Collapsed;
        ListItems.Visibility = showMain && _viewMode == UI.LibraryViewMode.List ? Visibility.Visible : Visibility.Collapsed;

        bool empty = showMain && _store.Items.Count == 0;
        EmptyView.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;

        if (drilled)
        {
            DrillTitle.Text = _store.DrilledTitle ?? "";
            DrillCount.Text = $"{_store.DrilledItems.Count} books";
            DrillSpin.Visibility = _store.IsDrilling ? Visibility.Visible : Visibility.Collapsed;
        }
        if (showMain && _viewMode == UI.LibraryViewMode.List)
            _ = LoadListThumbsAsync();
    }

    private async Task LoadListThumbsAsync()
    {
        foreach (var it in _store.Items.OfType<UI.TileItem>().Take(400))
            await _store.LoadThumbnailAsync(it);
    }

    // ---- windowed grid feeds (UniformGrid fixed 4 + spacers) ----

    private void FeedGrid() => Dispatcher.Invoke(() =>
    {
        double off = GridScroll.VerticalOffset;
        _gridWin.SetSource(_store.Items);
        _gridWin.OnScroll(off, GridScroll.ViewportHeight);
    });

    private void FeedDrill() => Dispatcher.Invoke(() =>
    {
        double off = DrillScroll.VerticalOffset;
        _drillWin.SetSource(_store.DrilledItems);
        _drillWin.OnScroll(off, DrillScroll.ViewportHeight);
    });

    private void FeedSearch() => Dispatcher.Invoke(() =>
    {
        double off = SearchView.VerticalOffset;
        _searchWin.SetSource(_store.SearchItems);
        _searchWin.OnScroll(off, SearchView.ViewportHeight);
    });

    private void Grid_ScrollChanged(object sender, ScrollChangedEventArgs e)
        => _gridWin.OnScroll(GridScroll.VerticalOffset, GridScroll.ViewportHeight);

    private void Grid_SizeChanged(object sender, SizeChangedEventArgs e)
        => FeedGrid();

    private void Drill_ScrollChanged(object sender, ScrollChangedEventArgs e)
        => _drillWin.OnScroll(DrillScroll.VerticalOffset, DrillScroll.ViewportHeight);

    private void Drill_SizeChanged(object sender, SizeChangedEventArgs e)
        => FeedDrill();

    private void Search_ScrollChanged(object sender, ScrollChangedEventArgs e)
        => _searchWin.OnScroll(SearchView.VerticalOffset, SearchView.ViewportHeight);

    private void Search_SizeChanged(object sender, SizeChangedEventArgs e)
        => FeedSearch();

    // ---- toolbar ----

    private async void Reload_Click(object sender, RoutedEventArgs e)
    {
        _store.ExitSearch();
        await _store.LoadAsync();
    }

    private void Library_Click(object sender, RoutedEventArgs e) => _store.ExitSearch();

    private void Settings_Click(object sender, RoutedEventArgs e) => OpenSettings();

    private void OpenDataFolder_Click(object sender, RoutedEventArgs e)
    {
        System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
        {
            FileName = AppPaths.Base,
            UseShellExecute = true,
        });
    }

    private void Exit_Click(object sender, RoutedEventArgs e) => Application.Current.Shutdown();

    // macOS WindowGroup parity: File > New Window (Ctrl+N) opens another
    // library window with its own store. Shutdown stays OnLastWindowClose.
    private void NewWindow_Executed(object sender, System.Windows.Input.ExecutedRoutedEventArgs e)
    {
        new MainWindow().Show();
    }

    private void About_Click(object sender, RoutedEventArgs e)
    {
        var w = new AboutWindow { Owner = this };
        w.ShowDialog();
    }

    // Same help.html ships in the Mac app (Sources/CatalogSwiftApp/Resources).
    private void Help_Click(object sender, RoutedEventArgs e)
    {
        new HelpWindow { Owner = this }.Show();
    }

    private void OpenSettings()
    {
        var w = new SettingsWindow { Owner = this };
        // Save re-reads only when the library source (or its credentials)
        // changed — Kobo/theme-only saves stay free (Mac parity).
        if (w.ShowDialog() == true && w.SettingsChanged)
            _ = _store.LoadAsync();
    }

    private void Search_Click(object sender, RoutedEventArgs e)
    {
        var w = new SearchWindow { Owner = this };
        w.SearchDone += async (results, seriesResults, tagName) =>
        {
            if (seriesResults is not null)
                _store.EnterSeriesResults(seriesResults, tagName ?? "");
            else if (results is not null)
                _store.EnterSearch(results);
            await Task.CompletedTask;
        };
        w.ShowDialog();
    }

    private async void Browse_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (!_uiReady || _loadingCombos || BrowseBox.SelectedItem is not string s) return;
        _store.BrowseMode = Enum.Parse<UI.BrowseMode>(s);
        _store.ExitSearch();
        await _store.LoadAsync();
    }

    private static string SortDisplay(UI.CatalogSortOrder o) => o switch
    {
        UI.CatalogSortOrder.Author => "Author",
        UI.CatalogSortOrder.Az => "A–Z",
        UI.CatalogSortOrder.Za => "Z–A",
        UI.CatalogSortOrder.Date => "Newest",
        _ => "Oldest",
    };

    private static UI.CatalogSortOrder ParseSort(string s) => s switch
    {
        "A–Z" => UI.CatalogSortOrder.Az,
        "Z–A" => UI.CatalogSortOrder.Za,
        "Newest" => UI.CatalogSortOrder.Date,
        "Oldest" => UI.CatalogSortOrder.Oldest,
        _ => UI.CatalogSortOrder.Author,
    };

    private async void Sort_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (!_uiReady || _loadingCombos || SortBox.SelectedItem is not string s) return;
        _store.SortOrder = ParseSort(s);
        if (_store.DrilledKind is null && !_store.IsSearchMode)
            await _store.LoadAsync();
        else if (_store.DrilledKind is not null)
            _store.ResortDrilled();
    }

    private void ViewGrid_Checked(object sender, RoutedEventArgs e)
    {
        if (!_uiReady) return;
        _viewMode = UI.LibraryViewMode.Grid;
        RefreshView();
    }

    private void ViewList_Checked(object sender, RoutedEventArgs e)
    {
        if (!_uiReady) return;
        _viewMode = UI.LibraryViewMode.List;
        RefreshView();
    }

    // ---- tiles ----

    private void Tile_Loaded(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement fe && fe.DataContext is UI.TileItem item)
            _ = _store.LoadThumbnailAsync(item);
    }

    private async void Tile_Click(object sender, MouseButtonEventArgs e)
    {
        if (sender is not FrameworkElement fe) return;
        var item = (fe.Tag as UI.TileItem) ?? fe.DataContext as UI.TileItem;
        if (item is null) return;
        await ActivateTileAsync(item);
    }

    private async Task ActivateTileAsync(UI.TileItem item)
    {
        if (_store.IsSearchMode)
        {
            if (item.Kind == "series")
            {
                try
                {
                    var books = await CalibreDb.Shared.BooksBySeriesAsync(item.Id);
                    _store.EnterSearch(books.Select(b => new SearchedBook(
                        b.Id, b.Title, b.Author, b.Path, null,
                        Array.Empty<string>(), "", b.AuthorSort)).ToList());
                }
                catch (Exception ex) { _store.DbError = ex.Message; }
            }
            else
            {
                OpenDetail(ToCatalogBook(item));
            }
            return;
        }
        if (_store.DrilledKind is not null)
        {
            OpenDetail(ToCatalogBook(item));
            return;
        }
        switch (item.Kind)
        {
            case "author":
                await _store.DrillAuthorAsync(item.Id, item.Title);
                break;
            case "series":
                await _store.DrillSeriesAsync(item.Id, item.Title);
                break;
            case "tag":
                await _store.DrillTagAsync(item.Title);
                break;
            default:
                OpenDetail(ToCatalogBook(item));
                break;
        }
    }

    private static CatalogBook ToCatalogBook(UI.TileItem item)
        => new(item.Id, item.Title, item.Subtitle, item.Path, item.HasCover, null);

    private void OpenDetail(CatalogBook book)
    {
        var w = new BookInfoWindow(book, _store) { Owner = this };
        w.ShowDialog();
    }

    private void List_Open(object sender, MouseButtonEventArgs e)
    {
        if (ListItems.SelectedItem is UI.TileItem item && item.Kind == "book")
            OpenDetail(ToCatalogBook(item));
    }

    private UI.TileItem? ContextItem(object sender)
    {
        if (sender is MenuItem mi && mi.Parent is ContextMenu cm)
        {
            if (cm.PlacementTarget is FrameworkElement fe)
                return (fe.Tag as UI.TileItem) ?? fe.DataContext as UI.TileItem;
            if (ListItems.SelectedItem is UI.TileItem sel) return sel;
        }
        return null;
    }

    private void CtxDetails_Click(object sender, RoutedEventArgs e)
    {
        if (ContextItem(sender) is UI.TileItem item && item.Kind == "book")
            OpenDetail(ToCatalogBook(item));
    }

    private async void CtxKobo_Click(object sender, RoutedEventArgs e)
    {
        if (ContextItem(sender) is UI.TileItem item && item.Kind == "book")
            await _store.OpenOnKoboAsync(ToCatalogBook(item));
    }

    private void DrillBack_Click(object sender, RoutedEventArgs e)
    {
        _store.ExitDrill();
        RefreshView();
    }
}
