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
    // macOS parity, both in-memory only (never persisted):
    // per-row password reveal + per-row ping test results, keyed by IP.
    private readonly HashSet<string> _revealedKobo = new();
    private readonly Dictionary<string, string> _koboTestResults = new();

    /// True when Save persisted a changed library source (or its
    /// credentials) — MainWindow re-reads only then, so Kobo/theme-only
    /// saves stay free (Mac parity).
    public bool SettingsChanged { get; private set; }
    // Snapshot of the persisted library source, taken at construction.
    private string _origSource = "smb";
    private string _origLocalDir = "";
    private string _origHost = "";
    private string _origShare = "";
    private string _origCalibre = "";
    private string _origUser = "";
    private string _origDomain = "";
    private string _origPass = "";

    public SettingsWindow()
    {
        InitializeComponent();
        UI.Theme.ApplyCurrent();
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
        NewKoboBox.KeyDown += (_, e) =>
        {
            if (e.Key == System.Windows.Input.Key.Enter) KoboAdd_Click(NewKoboBox, new RoutedEventArgs());
        };
        ThemeBox.SelectedIndex = Math.Clamp(_s.ThemePreference, 0, 2);
        UpdateSourceEnabled();
        // Snapshot the persisted library source for Save_Click's reload check.
        _origSource = _s.LibrarySource;
        _origLocalDir = SettingsStore.NormalizedLocalDir(_s.LocalLibraryDir);
        _origHost = srv?.Host ?? "";
        _origShare = srv?.Shares?.FirstOrDefault()?.Name ?? "";
        _origCalibre = (srv?.Shares?.FirstOrDefault()?.CalibreMetadataPath ?? "").Replace('\\', '/').TrimStart('/');
        _origUser = srv?.User ?? "";
        _origDomain = srv?.Domain ?? "";
        _origPass = host.Length > 0 ? _s.PasswordFor(host) ?? "" : "";
    }

    // macOS KoboDevice rows parity: star (default) + editable IP +
    // Password + Show + per-row ping Test + trash. Rows are rebuilt on
    // add/remove/rename/star; password keystrokes write straight through
    // to _s so the model is never behind the boxes.
    private void RefreshKoboList()
    {
        KoboRows.Children.Clear();
        foreach (string ip in _s.KoboIps.ToList())
            KoboRows.Children.Add(BuildKoboRow(ip));
    }

    private static readonly System.Windows.Media.Brush StarOn =
        System.Windows.Media.Brushes.Goldenrod;
    private static readonly System.Windows.Media.Brush StarOff =
        System.Windows.Media.Brushes.Gray;

    private System.Windows.Controls.Border BuildKoboRow(string ip)
    {
        string committed = ip; // row's IP as last committed (rename target)
        bool isDefault = committed == _s.KoboIp;

        var row = new StackPanel { Orientation = Orientation.Horizontal };
        var card = new System.Windows.Controls.Border
        {
            Child = row,
            CornerRadius = new CornerRadius(6),
            Background = new System.Windows.Media.SolidColorBrush(
                System.Windows.Media.Color.FromArgb(0x0F, 0x80, 0x80, 0x80)),
            Padding = new Thickness(4),
            Margin = new Thickness(0, 0, 0, 4),
        };

        var star = new Button
        {
            Content = isDefault ? "★" : "☆",
            Foreground = isDefault ? StarOn : StarOff,
            Width = 34, Padding = new Thickness(0), FontSize = 14,
            HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Center,
            Background = System.Windows.Media.Brushes.Transparent,
            BorderThickness = new Thickness(0),
            ToolTip = isDefault ? "Default" : "Set as default",
        };
        star.Click += (_, _) =>
        {
            _s.KoboIp = committed;
            KoboStatus.Text = $"Default Kobo: {committed}";
            RefreshKoboList();
        };
        row.Children.Add(star);

        var ipBox = new TextBox
        {
            Text = committed, Width = 150, FontFamily = new System.Windows.Media.FontFamily("Consolas"),
            VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0),
            ToolTip = "IP address",
        };
        void CommitIp()
        {
            string next = ipBox.Text.Trim();
            if (next.Length == 0 || next == committed) { ipBox.Text = committed; return; }
            _s.RenameKobo(committed, next);
            if (_revealedKobo.Remove(committed)) _revealedKobo.Add(next);
            if (_koboTestResults.Remove(committed, out string? r)) _koboTestResults[next] = r;
            RefreshKoboList();
        }
        ipBox.KeyDown += (_, e) =>
        {
            if (e.Key == System.Windows.Input.Key.Enter) CommitIp();
        };
        ipBox.LostFocus += (_, _) => CommitIp();
        UI.Watermark.SetText(ipBox, "IP address");
        row.Children.Add(ipBox);

        row.Children.Add(new TextBlock
        {
            Text = "Password", Width = 60, VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(8, 0, 0, 0),
        });
        var passBox = new PasswordBox { Width = 100, VerticalAlignment = VerticalAlignment.Center };
        var passShow = new TextBox
        {
            Width = 100, FontFamily = new System.Windows.Media.FontFamily("Consolas"),
            VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed,
        };
        string initPw = _s.KoboPasswordFor(committed) ?? "";
        passBox.Password = initPw;
        passShow.Text = initPw;
        UI.Watermark.SetText(passBox, "optional");
        UI.Watermark.SetText(passShow, "optional");
        bool revealed = _revealedKobo.Contains(committed);
        passBox.Visibility = revealed ? Visibility.Collapsed : Visibility.Visible;
        passShow.Visibility = revealed ? Visibility.Visible : Visibility.Collapsed;
        passBox.PasswordChanged += (_, _) =>
        {
            if (passShow.Visibility == Visibility.Visible) return;
            _s.SetKoboPassword(committed, passBox.Password);
        };
        passShow.TextChanged += (_, _) =>
        {
            if (passShow.Visibility != Visibility.Visible) return;
            _s.SetKoboPassword(committed, passShow.Text);
        };
        row.Children.Add(passBox);
        row.Children.Add(passShow);

        var showBtn = new Button
        {
            Content = revealed ? "Hide" : "Show", Width = 52,
            Margin = new Thickness(6, 0, 0, 0), Padding = new Thickness(8, 0, 8, 0),
        };
        showBtn.Click += (_, _) =>
        {
            if (passShow.Visibility == Visibility.Visible)
            {
                passBox.Password = passShow.Text;
                passShow.Visibility = Visibility.Collapsed;
                passBox.Visibility = Visibility.Visible;
                showBtn.Content = "Show";
                _revealedKobo.Remove(committed);
            }
            else
            {
                passShow.Text = passBox.Password;
                passBox.Visibility = Visibility.Collapsed;
                passShow.Visibility = Visibility.Visible;
                showBtn.Content = "Hide";
                _revealedKobo.Add(committed);
            }
        };
        row.Children.Add(showBtn);

        var pill = new TextBlock
        {
            Width = 45, FontWeight = FontWeights.Bold, FontSize = 11,
            VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0),
        };
        void SetPill(string? result)
        {
            if (result is null) { pill.Text = "Pass"; pill.Opacity = 0; return; }
            pill.Opacity = 1;
            pill.Text = result;
            pill.Foreground = result == "Pass"
                ? System.Windows.Media.Brushes.Green : System.Windows.Media.Brushes.Red;
        }
        _koboTestResults.TryGetValue(committed, out string? saved);
        SetPill(saved);

        var testBtn = new Button
        {
            Content = "Test", Width = 52, Margin = new Thickness(8, 0, 0, 0),
            Padding = new Thickness(8, 0, 8, 0),
            ToolTip = "Ping this Kobo (awake check)",
        };
        testBtn.Click += async (_, _) =>
        {
            SetPill(null);
            testBtn.IsEnabled = false;
            try
            {
                using var ping = new Ping();
                var reply = await ping.SendPingAsync(committed, 1000);
                string r = reply.Status == IPStatus.Success ? "Pass" : "Fail";
                _koboTestResults[committed] = r;
                SetPill(r);
                KoboStatus.Text = r == "Pass"
                    ? $"{committed}: {reply.RoundtripTime}ms awake"
                    : $"{committed}: {reply.Status} (sleeping?)";
            }
            catch (Exception ex)
            {
                _koboTestResults[committed] = "Fail";
                SetPill("Fail");
                KoboStatus.Text = $"{committed}: unreachable ({ex.Message})";
            }
            finally { testBtn.IsEnabled = true; }
        };
        row.Children.Add(testBtn);

        var trash = new Button
        {
            Content = "\U0001F5D1", Width = 34, Margin = new Thickness(6, 0, 0, 0),
            Padding = new Thickness(0), FontSize = 14,
            HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Center,
            Foreground = System.Windows.Media.Brushes.Red,
            Background = System.Windows.Media.Brushes.Transparent,
            BorderThickness = new Thickness(0), ToolTip = "Remove",
        };
        trash.Click += (_, _) =>
        {
            _s.KoboIps.Remove(committed);
            _s.SetKoboPassword(committed, "");
            _revealedKobo.Remove(committed);
            _koboTestResults.Remove(committed);
            if (_s.KoboIp == committed) _s.KoboIp = _s.KoboIps.FirstOrDefault() ?? "";
            RefreshKoboList();
        };
        row.Children.Add(trash);
        row.Children.Add(pill);

        return card;
    }

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
            KoboStatus.Text = $"Added {ip}";
            RefreshKoboList();
            NewKoboBox.Text = "";
        }
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
            KoboPasswords = new Dictionary<string, string>(_s.KoboPasswords),
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
        // Re-read the library only when the source (or its credentials)
        // changed — a Kobo/theme-only save stays free (Mac parity).
        var srv = s.SmbServers.FirstOrDefault();
        string pass = PassShowBox.Visibility == Visibility.Visible ? PassShowBox.Text : PassBox.Password;
        SettingsChanged = s.LibrarySource != _origSource
            || s.LocalLibraryDir != _origLocalDir
            || (srv?.Host ?? "") != _origHost
            || (srv?.Shares?.FirstOrDefault()?.Name ?? "") != _origShare
            || (srv?.Shares?.FirstOrDefault()?.CalibreMetadataPath ?? "") != _origCalibre
            || (srv?.User ?? "") != _origUser
            || (srv?.Domain ?? "") != _origDomain
            || pass != _origPass;
        SettingsStore.Save(s);
        UI.Theme.Apply(s.ThemePreference);
        StatusText.Text = "Saved.";
        DialogResult = true;
    }

    private void Cancel_Click(object sender, RoutedEventArgs e) => Close();
}
