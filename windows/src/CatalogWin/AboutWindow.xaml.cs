using System.Diagnostics;
using System.Reflection;
using System.Windows;
using System.Windows.Navigation;

namespace CatalogWin;

// About dialog: product identity + donation appeal + PayPal banner.
// PayPal hosted-button URL is shared with adblink (same account).
public partial class AboutWindow : Window
{
    // Same hosted button as adblink (about.cpp / mainwindow.cpp).
    public const string PayPalUrl =
        "https://www.paypal.com/cgi-bin/webscr?cmd=_s-xclick&hosted_button_id=GKZMW456H6E5W";

    public AboutWindow()
    {
        InitializeComponent();
        UI.Theme.ApplyCurrent();
        var v = Assembly.GetExecutingAssembly().GetName().Version;
        // Trailing zeros add noise ("1.0", not "1.0.0.0"); show only
        // defined, nonzero trailing components.
        VersionText.Text = v is null ? ""
            : "Version " + v.ToString(v.Revision > 0 ? 4 : v.Build > 0 ? 3 : 2);
    }

    private void Link_Navigate(object sender, RequestNavigateEventArgs e)
    {
        Process.Start(new ProcessStartInfo(e.Uri.AbsoluteUri) { UseShellExecute = true });
        e.Handled = true;
    }

    private void Donate_Click(object sender, RoutedEventArgs e)
    {
        Process.Start(new ProcessStartInfo(PayPalUrl) { UseShellExecute = true });
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Close();
}
