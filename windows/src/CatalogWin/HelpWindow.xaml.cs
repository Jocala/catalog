using System.IO;
using System.Windows;

namespace CatalogWin;

// Internal help viewer (WebView2, Edge engine — same help.html ships in
// the Mac app). File-local navigation stays in the viewer; web links open
// in the default browser.
public partial class HelpWindow : Window
{
    public HelpWindow()
    {
        InitializeComponent();
        UI.Theme.ApplyCurrent();
        Loaded += async (_, _) => await InitAsync();
    }

    private async Task InitAsync()
    {
        try
        {
            string path = Path.Combine(AppContext.BaseDirectory, "Assets", "help.html");
            if (!File.Exists(path))
            {
                ShowError("Help file is missing.");
                return;
            }
            await HelpView.EnsureCoreWebView2Async();
            // Web content follows the WebView profile scheme, not the WPF
            // theme — mirror the app's Theme setting (System = follow OS).
            HelpView.CoreWebView2.Profile.PreferredColorScheme = UI.Theme.CurrentPreference() switch
            {
                2 => Microsoft.Web.WebView2.Core.CoreWebView2PreferredColorScheme.Dark,
                1 => Microsoft.Web.WebView2.Core.CoreWebView2PreferredColorScheme.Light,
                _ => Microsoft.Web.WebView2.Core.CoreWebView2PreferredColorScheme.Auto,
            };
            HelpView.CoreWebView2.NavigationStarting += Core_NavigationStarting;
            HelpView.CoreWebView2.NavigationCompleted += (_, _) => SyncNavButtons();
            HelpView.Source = new Uri(path);
        }
        catch (Exception ex)
        {
            // WebView2 Runtime missing or unusable on this machine.
            ShowError("Help viewer unavailable: " + ex.Message);
        }
    }

    private void Core_NavigationStarting(object? sender, Microsoft.Web.WebView2.Core.CoreWebView2NavigationStartingEventArgs e)
    {
        string uri = e.Uri ?? "";
        // In-page Close link (help.html header/footer, catalog:close).
        // Same scheme is intercepted by the Mac WKWebView viewer.
        if (uri.StartsWith("catalog:close", StringComparison.OrdinalIgnoreCase))
        {
            e.Cancel = true;
            Close();
            return;
        }
        if (uri.StartsWith("http://", StringComparison.OrdinalIgnoreCase)
            || uri.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
        {
            e.Cancel = true;
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(uri) { UseShellExecute = true });
        }
    }

    private void SyncNavButtons()
    {
        BackBtn.IsEnabled = HelpView.CanGoBack;
        FwdBtn.IsEnabled = HelpView.CanGoForward;
    }

    private void ShowError(string message)
    {
        ErrorText.Text = message;
        ErrorText.Visibility = Visibility.Visible;
    }

    private void Back_Click(object sender, RoutedEventArgs e)
    {
        if (HelpView.CanGoBack) HelpView.GoBack();
    }

    private void Fwd_Click(object sender, RoutedEventArgs e)
    {
        if (HelpView.CanGoForward) HelpView.GoForward();
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Close();
}
