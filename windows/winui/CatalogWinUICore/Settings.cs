// Settings schema, shared with the other shells (same JSON keys:
// library_source, local_library_dir, smb_servers, Passwords host->pass,
// KoboIps, KoboPasswords ip->pass, ThemePreference).
//
// Phase 1 reads the already-configured file on the build VM (explicit
// path from the app, TEMPORARY — no Settings UI exists yet). Phase 2
// owns the real store: new folder + Settings UI + DPAPI passwords.

using System.Text.Json;
using System.Text.Json.Serialization;

namespace CatalogWinUICore;

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
    public Dictionary<string, string> Passwords { get; set; } = new();
    public List<string> KoboIps { get; set; } = new();
    public Dictionary<string, string> KoboPasswords { get; set; } = new();
    public int ThemePreference { get; set; } = 0;

    public string? PasswordFor(string host)
        => Passwords.TryGetValue(host, out string? p) && !string.IsNullOrEmpty(p) ? p : null;

    /// A library source is configured when the selected source has an
    /// address: first SMB server host, or a local folder. Otherwise this
    /// is a fresh start and the gallery stays silent (empty state).
    public bool HasSource =>
        LibrarySource == "local"
            ? !string.IsNullOrWhiteSpace(LocalLibraryDir)
            : SmbServers.Count > 0 && !string.IsNullOrWhiteSpace(SmbServers[0].Host);
}

public static class SettingsStore
{
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    public static AppSettings LoadFrom(string path)
    {
        try
        {
            if (File.Exists(path))
            {
                string json = File.ReadAllText(path);
                if (JsonSerializer.Deserialize<AppSettings>(json, JsonOpts) is AppSettings s)
                    return s;
            }
        }
        catch
        {
        }
        return new AppSettings();
    }
}
