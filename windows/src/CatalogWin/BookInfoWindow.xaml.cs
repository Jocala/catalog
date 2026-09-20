using System.Windows;
using System.Windows.Media.Imaging;
using CatalogWin.Core;
using CatalogWin.Services;
using CatalogWin.UI;

namespace CatalogWin;

public partial class BookInfoWindow : Window
{
    private readonly CatalogBook _book;
    private readonly CatalogStore _store;

    public BookInfoWindow(CatalogBook book, CatalogStore store)
    {
        InitializeComponent();
        _book = book;
        _store = store;
        TitleText.Text = book.Title;
        AuthorText.Text = book.Author;
        CoverImg.Source = store.CoverSync(book.CoverHash, book.Path) ?? store.CoverSync(null, book.Path);
        if (CoverImg.Source is null && book.HasCover)
            _ = LoadCoverAsync();
        _ = LoadDetailAsync();
    }

    private async Task LoadCoverAsync()
    {
        BitmapSource? img = await Covers.ThumbnailAsync(_book.Path, _book.CoverHash);
        if (img is not null)
            Dispatcher.Invoke(() => CoverImg.Source = img);
    }

    private async Task LoadDetailAsync()
    {
        DetailSpin.Visibility = Visibility.Visible;
        try
        {
            BookDetail? d = await CalibreDb.Shared.BookDetailAsync(_book.Id);
            if (d is null) return;
            Dispatcher.Invoke(() =>
            {
                if (!string.IsNullOrEmpty(d.Series))
                    SeriesText.Text = $"Series: {d.Series} #{d.SeriesIndex:g}";
                if (!string.IsNullOrEmpty(d.Tags)) TagsText.Text = d.Tags;
                if (!string.IsNullOrEmpty(d.Publisher)) PubText.Text = $"Publisher: {d.Publisher}";
                if (!string.IsNullOrEmpty(d.Isbn)) IsbnText.Text = $"ISBN: {d.Isbn}";
                if (!string.IsNullOrEmpty(d.Comments))
                {
                    CommentsText.Text = d.Comments;
                    CommentsBox.Visibility = Visibility.Visible;
                }
            });
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("CatalogStore", $"detail failed id={_book.Id} {ex.Message}");
        }
        finally
        {
            Dispatcher.Invoke(() => DetailSpin.Visibility = Visibility.Collapsed);
        }
    }

    private async void Read_Click(object sender, RoutedEventArgs e)
    {
        KoboStatus.Text = "Opening on Kobo…";
        string? err = null;
        await KoboLauncher.RunKoboAsync(
            $"{_book.Author} {_book.Title}", _book.Title, _book.Author,
            onStatus: s => Dispatcher.Invoke(() => KoboStatus.Text = s),
            onError: msg => err = msg);
        if (err is null)
        {
            _store.SetKoboStatus($"Opened on Kobo: {_book.Title}");
            Close();
        }
        else
        {
            MessageBox.Show(err, "Kobo", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Close();
}
