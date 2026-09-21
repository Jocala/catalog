using System.Windows;
using CatalogWin.Services;

namespace CatalogWin.UI;

// Applies the Settings > Theme preference (System/Light/Dark — same
// indices as macOS theme_preference) through WPF's official Fluent theme
// (.NET 9+): Application.ThemeMode re-themes every stock control template,
// so dark mode is complete instead of spotty. The three app-chrome brushes
// (toolbar/panel bands, muted text, tile placeholder) still swap here.
public static class Theme
{
    public const string LightSource = "UI/LightTheme.xaml";
    public const string DarkSource = "UI/DarkTheme.xaml";

    // Pure mapping (unit-testable, no WPF): 2 = dark, 1 = light,
    // anything else (0 = system, unknown) follows the OS.
    public static ThemeMode MapPreference(int preference) => preference switch
    {
        2 => ThemeMode.Dark,
        1 => ThemeMode.Light,
        _ => ThemeMode.System,
    };

    public static int CurrentPreference()
    {
        try
        {
            return Math.Clamp(SettingsStore.Load().ThemePreference, 0, 2);
        }
        catch
        {
            return 0;
        }
    }

    public static void ApplyCurrent() => Apply(CurrentPreference());

    public static void Apply(int preference)
    {
        var app = Application.Current;
        if (app is null)
            return;
        app.ThemeMode = MapPreference(preference);
        SwapChrome(preference);
    }

    // Chrome brushes track the effective mode. System follows the OS
    // app-mode key (default light when unknown).
    private static void SwapChrome(int preference)
    {
        bool dark = preference == 2 || (preference == 0 && !SystemLight());
        var app = Application.Current;
        if (app is null)
            return;
        var merged = app.Resources.MergedDictionaries;
        var source = new Uri(dark ? DarkSource : LightSource, UriKind.Relative);
        for (int i = 0; i < merged.Count; i++)
        {
            string? s = merged[i].Source?.OriginalString;
            if (s is not null && (s.EndsWith("LightTheme.xaml") || s.EndsWith("DarkTheme.xaml")))
            {
                if (s == source.OriginalString)
                    return; // already applied
                merged[i] = new ResourceDictionary { Source = source };
                return;
            }
        }
        merged.Add(new ResourceDictionary { Source = source });
    }

    public static bool SystemLight()
    {
        try
        {
            using var key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            if (key?.GetValue("AppsUseLightTheme") is int v)
                return v != 0;
        }
        catch
        {
        }
        return true;
    }
}
