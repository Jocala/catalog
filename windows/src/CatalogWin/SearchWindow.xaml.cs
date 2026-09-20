using System.Windows;
using System.Windows.Controls;
using CatalogWin.Core;
using CatalogWin.Services;

namespace CatalogWin;

/// Search form (macOS SearchFormSheet parity): General / Title / Author /
/// Series dropdown / Tag dropdown / tag-expand toggle + Search.
/// Publisher + Year removed (2026-09-18) — stays removed.
public partial class SearchWindow : Window
{
    public event Func<List<SearchedBook>?, List<SeriesSummary>?, string?, Task>? SearchDone;

    public SearchWindow()
    {
        InitializeComponent();
        Loaded += async (_, _) => await FillDropdownsAsync();
    }

    private async Task FillDropdownsAsync()
    {
        try
        {
            var tags = await CalibreDb.Shared.AllTagsAsync();
            TagBox.ItemsSource = tags.Select(t => t.Name).ToList();
            var series = await CalibreDb.Shared.AllSeriesAsync();
            SeriesBox.ItemsSource = series.Select(s => s.Name).ToList();
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("Search", $"dropdown fill failed: {ex.Message}");
        }
    }

    private static string BoxText(ComboBox b) => (b.Text ?? "").Trim();

    private async void Search_Click(object sender, RoutedEventArgs e)
    {
        string q = QueryBox.Text.Trim();
        string title = TitleBox.Text.Trim();
        string author = AuthorBox.Text.Trim();
        string series = BoxText(SeriesBox);
        string tag = BoxText(TagBox);
        if (q.Length == 0 && title.Length == 0 && author.Length == 0 && series.Length == 0 && tag.Length == 0)
            return;
        SearchSpin.Visibility = Visibility.Visible;
        try
        {
            // tagExpand: a tag-only search shows series results (onSearchSeries).
            if (tag.Length > 0 && q.Length == 0 && title.Length == 0 && author.Length == 0 && series.Length == 0
                && ExpandCheck.IsChecked == true)
            {
                var allTags = await CalibreDb.Shared.AllTagsAsync();
                var match = allTags.FirstOrDefault(t =>
                    t.Name.Equals(tag, StringComparison.OrdinalIgnoreCase));
                if (match is not null)
                {
                    var ss = await CalibreDb.Shared.SeriesByTagAsync(match.Id);
                    if (SearchDone is not null) await SearchDone(null, ss, match.Name);
                    Close();
                    return;
                }
            }
            var results = await CalibreDb.Shared.SearchBooksAsync(
                query: q, title: title, author: author, series: series, tag: tag);
            if (SearchDone is not null) await SearchDone(results, null, null);
            Close();
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Search", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
        finally
        {
            SearchSpin.Visibility = Visibility.Collapsed;
        }
    }

    private void Cancel_Click(object sender, RoutedEventArgs e) => Close();
}
