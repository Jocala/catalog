using System.Diagnostics;
using System.IO;
using System.Net.NetworkInformation;
using System.Windows;
using System.Windows.Controls;
using Microsoft.Win32;
using CatalogWin.Core;
using CatalogWin.Services;

namespace CatalogWin;

public partial class SettingsWindow : Window
{
    private AppSettings _s;

    public SettingsWindow()
    {
        InitializeComponent();
        _s = SettingsStore.Load();
        if (_s.LibrarySource == "local") SrcLocal.IsChecked = true; else SrcSmb.IsChecked = true;
        LocalDirBox.Text = _s.LocalLibraryDir;
        var srv = _s.SmbServers.FirstOrDefault();
        ServerBox.Text = srv?.Host ?? "";
        ShareBox.Text = srv?.Shares?.FirstOrDefault()?.Name ?? "";
        CalibreBox.Text = srv?.Shares?.FirstOrDefault()?.CalibreMetadataPath ?? "";
        UserBox.Text = srv?.User ?? "";
        DomainBox.Text = srv?.Domain ?? "";
        string host = srv?.Host ?? "";
        if (host.Length > 0) PassBox.Password = _s.PasswordFor(host) ?? "";
        RefreshKoboList();
        ThemeBox.SelectedIndex = Math.Clamp(_s.ThemePreference, 0, 2);
        UpdateSourceEnabled();
    }

    private void RefreshKoboList()
    {
        KoboList.ItemsSource = null;
        KoboList.ItemsSource = _s.KoboIps
            .Select(ip => ip == _s.KoboIp ? $"★ {ip}" : $"  {ip}")
            .ToList();
    }

    private static string StripStar(string s) => s.Trim().TrimStart('★').Trim();

    private void Source_Changed(object sender, RoutedEventArgs e) => UpdateSourceEnabled();

    private void UpdateSourceEnabled()
    {
        bool local = SrcLocal.IsChecked == true;
        LocalDirBox.IsEnabled = BrowseBtn.IsEnabled = TestLocalBtn.IsEnabled = local;
        SmbGroup.IsEnabled = !local;
        SmbGroup.Opacity = local ? 0.5 : 1;
    }

    private void BrowseLocal_Click(object sender, RoutedEventArgs e)
    {
        var dlg = new OpenFolderDialog { Title = "Choose a Calibre folder…" };
        if (dlg.ShowDialog() == true)
            LocalDirBox.Text = dlg.FolderName;
    }

    private void TestLocal_Click(object sender, RoutedEventArgs e)
    {
        string dir = SettingsStore.NormalizedLocalDir(LocalDirBox.Text);
        string db = System.IO.Path.Combine(dir, "metadata.db");
        bool ok = dir.Length > 0 && File.Exists(db);
        LocalPass.Text = ok ? "Pass" : "Fail";
        LocalPass.Foreground = ok ? System.Windows.Media.Brushes.Green : System.Windows.Media.Brushes.Red;
        StatusText.Text = ok ? $"Local OK: {db}" : $"metadata.db not found in {dir}";
    }

    private async void TestSmb_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            string detail = await SmbReader.TestConnectionAsync(
                ServerBox.Text.Trim(), ShareBox.Text.Trim(),
                UserBox.Text.Trim(), PassBox.Password, DomainBox.Text.Trim());
            SmbPass.Text = "Pass";
            SmbPass.Foreground = System.Windows.Media.Brushes.Green;
            StatusText.Text = detail;
        }
        catch (Exception ex)
        {
            SmbPass.Text = "Fail";
            SmbPass.Foreground = System.Windows.Media.Brushes.Red;
            StatusText.Text = ex.Message;
        }
    }

    private async void TestDb_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            string detail = await SmbReader.TestDatabaseAsync(DraftSettings());
            DbPass.Text = "Pass";
            DbPass.Foreground = System.Windows.Media.Brushes.Green;
            StatusText.Text = detail;
        }
        catch (Exception ex)
        {
            DbPass.Text = "Fail";
            DbPass.Foreground = System.Windows.Media.Brushes.Red;
            StatusText.Text = ex.Message;
        }
    }

    private void ShowPass_Click(object sender, RoutedEventArgs e)
    {
        if (PassShowBox.Visibility == Visibility.Visible)
        {
            PassBox.Password = PassShowBox.Text;
            PassShowBox.Visibility = Visibility.Collapsed;
            PassBox.Visibility = Visibility.Visible;
            ShowPassBtn.Content = "Show";
        }
        else
        {
            PassShowBox.Text = PassBox.Password;
            PassBox.Visibility = Visibility.Collapsed;
            PassShowBox.Visibility = Visibility.Visible;
            ShowPassBtn.Content = "Hide";
        }
    }

    private void KoboAdd_Click(object sender, RoutedEventArgs e)
    {
        string ip = NewKoboBox.Text.Trim();
        if (ip.Length > 0 && !_s.KoboIps.Contains(ip))
        {
            _s.KoboIps.Add(ip);
            if (_s.KoboIp.Length == 0) _s.KoboIp = ip;
            RefreshKoboList();
            NewKoboBox.Text = "";
        }
    }

    private void KoboRemove_Click(object sender, RoutedEventArgs e)
    {
        if (KoboList.SelectedItem is string sel)
        {
            string ip = StripStar(sel);
            _s.KoboIps.Remove(ip);
            if (_s.KoboIp == ip) _s.KoboIp = _s.KoboIps.FirstOrDefault() ?? "";
            RefreshKoboList();
        }
    }

    private void KoboStar_Click(object sender, RoutedEventArgs e)
    {
        if (KoboList.SelectedItem is string sel)
        {
            _s.KoboIp = StripStar(sel);
            RefreshKoboList();
        }
    }

    private async void KoboTest_Click(object sender, RoutedEventArgs e)
    {
        string ip = KoboList.SelectedItem is string sel ? StripStar(sel) : _s.KoboIp;
        if (ip.Length == 0) { KoboTestResult.Text = "no Kobo IP"; return; }
        KoboTestResult.Text = "pinging…";
        try
        {
            using var ping = new Ping();
            var reply = await ping.SendPingAsync(ip, 1000);
            KoboTestResult.Text = reply.Status == IPStatus.Success
                ? $"{ip}: {reply.RoundtripTime}ms awake"
                : $"{ip}: {reply.Status} (sleeping?)";
        }
        catch (Exception ex)
        {
            KoboTestResult.Text = $"{ip}: unreachable ({ex.Message})";
        }
    }

    private void Reindex_Click(object sender, RoutedEventArgs e)
    {
        CalibreDb.Shared.Invalidate();
        DialogResult = true; // main window reloads
    }

    private void OpenData_Click(object sender, RoutedEventArgs e)
    {
        Process.Start(new ProcessStartInfo
        {
            FileName = AppPaths.Base,
            UseShellExecute = true,
        });
    }

    private AppSettings DraftSettings()
    {
        string pass = PassShowBox.Visibility == Visibility.Visible ? PassShowBox.Text : PassBox.Password;
        string host = ServerBox.Text.Trim();
        var s = new AppSettings
        {
            LibrarySource = SrcLocal.IsChecked == true ? "local" : "smb",
            LocalLibraryDir = SettingsStore.NormalizedLocalDir(LocalDirBox.Text),
            KoboIp = _s.KoboIp,
            KoboIps = new List<string>(_s.KoboIps),
            ThemePreference = ThemeBox.SelectedIndex,
            SmbServers = new List<SmbServer>
            {
                new("", host, 445, UserBox.Text.Trim(), DomainBox.Text.Trim(),
                    new List<SmbShare> { new(ShareBox.Text.Trim(), CalibreBox.Text.Trim().Replace('\\', '/').TrimStart('/')) }),
            },
        };
        if (host.Length > 0) s.SetPassword(host, pass);
        return s;
    }

    private void Save_Click(object sender, RoutedEventArgs e)
    {
        var s = DraftSettings();
        SettingsStore.Save(s);
        StatusText.Text = "Saved.";
        DialogResult = true;
    }

    private void Cancel_Click(object sender, RoutedEventArgs e) => Close();
}
