// Port of macos CatalogCore/Models/SmbCredentials.swift + UserDefaults settings keys.
// All settings live in one JSON file (%APPDATA%/com.jocala.Catalog/settings.json,
// adblink.json-style) instead of UserDefaults. Keys mirror macOS:
// library_source, local_library_dir, smb_servers, smb_pass_<host>,
// kobo_ip, kobo_ips, kobo_pass_<ip>, theme_preference.

using System.Text.Json;
using System.Text.Json.Serialization;
using System.IO;

namespace CatalogWin.Services;

public sealed record SmbShare(string Name = "", string CalibreMetadataPath = "");
public sealed record SmbServer(
    string Label = "",
    string Host = "",
    int Port = 445,
    string User = "",
    string Domain = "",
    List<SmbShare>? Shares = null)
{
    public string DisplayName => string.IsNullOrEmpty(Label) ? Host : Label;
};

public sealed class AppSettings
{
    public string LibrarySource { get; set; } = "smb"; // "smb" | "local"
    public string LocalLibraryDir { get; set; } = "";
    public List<SmbServer> SmbServers { get; set; } = new();
    public Dictionary<string, string> Passwords { get; set; } = new(); // host -> password (smb_pass_<host>)
    public string KoboIp { get; set; } = "";
    public List<string> KoboIps { get; set; } = new();
    public Dictionary<string, string> KoboPasswords { get; set; } = new(); // ip -> password (kobo_pass_<ip>)
    public int ThemePreference { get; set; } = 0;

    public string? PasswordFor(string host)
        => Passwords.TryGetValue(host, out string? p) && !string.IsNullOrEmpty(p) ? p : null;

    public void SetPassword(string host, string password) => Passwords[host] = password;

    // macOS KoboDevice parity: optional per-IP SSH password for the Kobo
    // list. Empty means key-only (BatchMode equivalent), exactly like macOS
    // returning nil. Stored in settings.json next to the SMB passwords —
    // same protection level as the rest of this file (see header).
    public string? KoboPasswordFor(string ip)
    {
        ip = (ip ?? "").Trim();
        return KoboPasswords.TryGetValue(ip, out string? p) && !string.IsNullOrEmpty(p) ? p : null;
    }

    public void SetKoboPassword(string ip, string password)
    {
        ip = (ip ?? "").Trim();
        if (ip.Length == 0) return;
        if (string.IsNullOrEmpty(password)) KoboPasswords.Remove(ip);
        else KoboPasswords[ip] = password;
    }

    // macOS in-place IP edit parity: renaming a Kobo row carries its
    // password to the new IP and follows the default marker. Last write
    // wins when the new IP already exists (macOS allows duplicate rows;
    // this dict cannot).
    public void RenameKobo(string oldIp, string newIp)
    {
        oldIp = (oldIp ?? "").Trim();
        newIp = (newIp ?? "").Trim();
        if (oldIp.Length == 0 || newIp.Length == 0 || oldIp == newIp) return;
        int idx = KoboIps.IndexOf(oldIp);
        if (idx < 0) return;
        KoboIps[idx] = newIp;
        if (KoboPasswords.Remove(oldIp, out string? pw) && !string.IsNullOrEmpty(pw))
            KoboPasswords[newIp] = pw;
        if (KoboIp == oldIp) KoboIp = newIp;
    }
}

public static class SettingsStore
{
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    private static string SettingsFile
        => Path.Combine(Core.AppPaths.Base, "settings.json");

    public static AppSettings Load()
    {
        try
        {
            if (File.Exists(SettingsFile))
            {
                string json = File.ReadAllText(SettingsFile);
                if (JsonSerializer.Deserialize<AppSettings>(json, JsonOpts) is AppSettings s)
                    return s;
            }
        }
        catch
        {
        }
        return new AppSettings();
    }

    public static void Save(AppSettings s)
    {
        try
        {
            File.WriteAllText(SettingsFile, JsonSerializer.Serialize(s, JsonOpts));
        }
        catch (Exception ex)
        {
            Core.AppLog.Shared.Error("Settings", $"save failed: {ex.Message}");
        }
    }

    public static string NormalizedLocalDir(string dir)
    {
        dir = (dir ?? "").Trim();
        if (dir.StartsWith("file://", StringComparison.OrdinalIgnoreCase))
            dir = dir["file://".Length..];
        while (dir.EndsWith('/') && dir.Length > 1)
            dir = dir[..^1];
        return dir;
    }
}
