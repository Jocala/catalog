// Guards the About donate link: same PayPal hosted button as adblink
// (about.cpp / mainwindow.cpp). Const-only access — no WPF instance, no STA.

using Xunit;

namespace CatalogWin.Tests;

public sealed class AboutTests
{
    [Fact]
    public void DonateUrl_MatchesAdblinkHostedButton()
    {
        Assert.Equal(
            "https://www.paypal.com/cgi-bin/webscr?cmd=_s-xclick&hosted_button_id=GKZMW456H6E5W",
            AboutWindow.PayPalUrl);
    }
}
