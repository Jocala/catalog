using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace CatalogWinUI;

public sealed partial class MainPage : Page
{
    public MainPage()
    {
        InitializeComponent();
        var items = Enumerable.Range(1, 3000)
            .Select(i => $"Book {i:0000} — Author {(i % 137) + 1}")
            .ToList();
        Tiles.ItemsSource = items;
        CountText.Text = $"{items.Count} items";
    }

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
}
