// Raw-SMB-only byte reader (Qt rule 1, Windows side).
// SMB goes through OS UNC (\\host\share\...) with Settings credentials via
// WNetAddConnection2W — the C# equivalent of qt SmbClient_win.cpp (Mpr.lib,
// no third-party dep). Never drive mappings, never mount points.
//
// Credential conflicts (WNet ERROR_SESSION_CREDENTIAL_CONFLICT, 1219):
// Windows allows one credential set per host. If the host is already
// connected under different credentials (Explorer mapping, another app),
// we FAIL with guidance instead of tearing down someone else's session.

using System.ComponentModel;
using System.Runtime.InteropServices;
using CatalogWin.Core;
using System.IO;

namespace CatalogWin.Services;

public sealed record LibraryTarget(
    SmbTarget? Smb,
    string LocalDir = "")
{
    public bool IsLocal => Smb is null;
    public string DisplayPath => Smb is not null ? Smb.SmbPath : $"file://{LocalDir}/metadata.db";
    public string CacheKey => Smb is not null ? Smb.CacheKey : $"local:{LocalDir}";
}

public sealed record SmbTarget(
    string Host,
    string Share,
    string RemotePath,
    string User,
    string Password,
    string Domain,
    string LibRoot)
{
    public string SmbPath => $"smb://{Host}/{Share}/{RemotePath}";
    public string CacheKey => $"{Host}/{Share}/{RemotePath}";

    /// OS UNC path for the same file (forward slashes -> backslashes).
    public string UncPath => $@"\\{Host}\{Share}\{RemotePath.Replace('/', '\\')}";
    public string UncDir(string remoteSub)
        => $@"\\{Host}\{Share}\{remoteSub.Replace('/', '\\')}";
}

public static class SmbReader
{
    // Session reuse: a WNet session setup (auth handshake) costs hundreds of
    // ms — doing it per thumbnail would kill interactive performance. The
    // session for the current (host, share, user, domain) is kept; repeat
    // connects with identical credentials are a cheap compare. Any op failure
    // drops the entry so the next op reconnects (covers password changes).
    // Sessions are left cached (never force-disconnected).
    private static readonly object WnetGate = new();
    private static string _sessHost = "", _sessShare = "", _sessUser = "", _sessDomain = "";
    private static bool _sessLive;

    private const int RESOURCETYPE_DISK = 1;
    private const int ERROR_SESSION_CREDENTIAL_CONFLICT = 1219;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NetResource
    {
        public int dwScope;
        public int dwType;
        public int dwDisplayType;
        public int dwUsage;
        public string? lpLocalName;
        public string? lpRemoteName;
        public string? lpComment;
        public string? lpProvider;
    }

    [DllImport("mpr.dll", CharSet = CharSet.Unicode)]
    private static extern int WNetAddConnection2W(
        ref NetResource netResource, string? password, string? username, int flags);

    private static string WinErrorText(int code)
    {
        try
        {
            string m = new Win32Exception(code).Message.Trim();
            if (!string.IsNullOrEmpty(m)) return m;
        }
        catch
        {
        }
        return $"Win32 error {code}";
    }

    /// Connect \\host\share with the given creds (no drive mapping).
    /// Throws CatalogDbException with the same classification tokens as the
    /// libsmbclient backend so callers sort them identically.
    public static void WnetEnsure(SmbTarget s)
    {
        lock (WnetGate)
        {
            if (_sessLive && _sessHost == s.Host && _sessShare == s.Share &&
                _sessUser == s.User && _sessDomain == s.Domain)
                return;
            _sessLive = false;
            _sessHost = _sessShare = _sessUser = _sessDomain = "";

            string user = s.User;
            if (s.Domain.Length > 0 && user.Length > 0 && !user.Contains('\\'))
                user = s.Domain + "\\" + user;
            var nr = new NetResource
            {
                dwType = RESOURCETYPE_DISK,
                lpRemoteName = $@"\\{s.Host}\{s.Share}",
            };
            int r = WNetAddConnection2W(ref nr,
                s.Password.Length == 0 ? null : s.Password,
                user.Length == 0 ? null : user, 0);
            if (r != 0)
                throw ClassifyConnectError(r, s);
            _sessHost = s.Host; _sessShare = s.Share;
            _sessUser = s.User; _sessDomain = s.Domain;
            _sessLive = true;
        }
    }

    private static void DropSession()
    {
        lock (WnetGate)
        {
            _sessLive = false;
            _sessHost = _sessShare = _sessUser = _sessDomain = "";
        }
    }

    private static CatalogDbException ClassifyConnectError(int code, SmbTarget s)
    {
        string what = WinErrorText(code);
        string display = s.SmbPath;
        return code switch
        {
            1326 or 5 or 86 or 1330 or 1331 or 1909 => // ERROR_LOGON_FAILURE, ACCESS_DENIED, INVALID_PASSWORD, ...
                CatalogDbException.AuthFailed(display, $"smb logon failure for {display}: {what}"),
            53 or 67 or 1327 => // ERROR_BAD_NETPATH, BAD_NET_NAME, NO_SUCH_USER
                CatalogDbException.NotFound(display, $"smb share not found at {display}: {what}"),
            ERROR_SESSION_CREDENTIAL_CONFLICT =>
                CatalogDbException.Network(display,
                    $"smb credential conflict for {display}: host is already connected under different credentials " +
                    $"(Explorer mapping or another app) — disconnect it first, then retry. {what}"),
            _ => CatalogDbException.Network(display, $"smb connect to {display} failed: {what}"),
        };
    }

    /// Settings → Test SMB: connect + share reachable.
    public static async Task<string> TestConnectionAsync(
        string host, string share, string user, string password, string domain)
    {
        var probe = new SmbTarget(host, share, "", user, password, domain, "");
        await Task.Run(() => WnetEnsure(probe));
        bool ok = await Task.Run(() => Directory.Exists($@"\\{host}\{share}"));
        return ok ? $"wnet: smb://{host}/{share}/ reachable"
                  : $"wnet: share not found at smb://{host}/{share}/";
    }

    /// Settings → Test Calibre: full metadata.db fetch (byte count).
    public static async Task<string> TestDatabaseAsync(AppSettings? settings = null)
    {
        var (t, data) = await SnapshotAsync(settings);
        return $"{t.DisplayPath}: {data.Length} bytes OK";
    }

    public static LibraryTarget ResolveTarget(AppSettings s)
    {
        // Either-or: local wins when selected; SMB fields are never touched.
        if (s.LibrarySource == "local")
        {
            string dir = SettingsStore.NormalizedLocalDir(s.LocalLibraryDir);
            if (dir.Length == 0)
                throw CatalogDbException.NotConfigured(
                    "No local library folder — open Settings → Source → Local and pick your Calibre folder first.");
            string dbPath = Path.Combine(dir, "metadata.db");
            if (!File.Exists(dbPath))
                throw CatalogDbException.NotFound($"file://{dbPath}", "metadata.db not found in that folder");
            return new LibraryTarget(null, dir);
        }
        SmbServer? server = s.SmbServers.FirstOrDefault();
        if (server is null || string.IsNullOrEmpty(server.Host))
            throw CatalogDbException.NotConfigured(
                "No SMB server configured — open Settings → SMB and save the server first.");
        SmbShare? share = server.Shares?.FirstOrDefault();
        if (share is null || string.IsNullOrEmpty(share.Name))
            throw CatalogDbException.NotConfigured(
                "No SMB share configured — open Settings → SMB and save the server first.");
        if (string.IsNullOrEmpty(share.CalibreMetadataPath))
            throw CatalogDbException.NotConfigured(
                "No Calibre path configured — open Settings → SMB and save the Calibre path first.");
        string remote = share.CalibreMetadataPath.Replace('\\', '/').Trim().Trim('/');
        // Folder input ("calibre") is accepted and resolves to the database,
        // mirroring Rust smb_with_creds + Swift (2026-09-19: a bare folder
        // used to be read as a file -> UnauthorizedAccessException
        // misreported as login failure).
        if (!remote.EndsWith("metadata.db", StringComparison.OrdinalIgnoreCase))
            remote = remote.Length == 0 ? "metadata.db" : remote + "/metadata.db";
        string root = remote.Contains('/') ? remote[..remote.LastIndexOf('/')] : "";
        return new LibraryTarget(new SmbTarget(
            server.Host, share.Name, remote,
            server.User, s.PasswordFor(server.Host) ?? "",
            server.Domain, root));
    }

    /// Fetch the live metadata.db bytes — always fresh, never cached, so newly
    /// added books are visible without Reindex/restart (pure-direct mode B).
    public static async Task<(LibraryTarget Target, byte[] Data)> SnapshotAsync(AppSettings? settings = null)
    {
        LibraryTarget t = ResolveTarget(settings ?? SettingsStore.Load());
        if (t.IsLocal)
        {
            string dbPath = Path.Combine(t.LocalDir, "metadata.db");
            byte[] data;
            try
            {
                data = await File.ReadAllBytesAsync(dbPath);
            }
            catch (Exception ex)
            {
                throw CatalogDbException.NotFound($"file://{dbPath}", Trunc(ex.Message));
            }
            if (data.Length == 0)
                throw CatalogDbException.NotFound($"file://{dbPath}", "empty file");
            AppLog.Shared.Info("SmbCatalogDB", $"read {data.Length} bytes from file://{dbPath}");
            return (t, data);
        }
        SmbTarget s = t.Smb!;
        byte[] bytes;
        try
        {
            await Task.Run(() => WnetEnsure(s));
            bytes = await File.ReadAllBytesAsync(s.UncPath);
        }
        catch (CatalogDbException)
        {
            throw;
        }
        catch (Exception ex)
        {
            // Stale session surfaces here — drop it so the next op reconnects.
            DropSession();
            throw Classify(ex, s);
        }
        if (bytes.Length == 0)
            throw CatalogDbException.NotFound(s.SmbPath, "empty file");
        AppLog.Shared.Info("SmbCatalogDB", $"fetched {bytes.Length} bytes from {s.SmbPath}");
        return (t, bytes);
    }

    public static string SmbBookPath(string rel, SmbTarget t)
    {
        if (rel.Length == 0) return "";
        if (rel.StartsWith("smb://") || rel.StartsWith("http://") || rel.StartsWith("https://")) return rel;
        if (rel.StartsWith('/')) return rel;
        string prefix = t.LibRoot.Length == 0 ? "" : t.LibRoot + "/";
        return $"smb://{t.Host}/{t.Share}/{prefix}{rel}";
    }

    public static string LocalBookPath(string rel, string dir)
    {
        if (rel.Length == 0) return "";
        if (rel.StartsWith("smb://") || rel.StartsWith("http://") || rel.StartsWith("https://")) return rel;
        dir = SettingsStore.NormalizedLocalDir(dir).TrimEnd('\\');
        if (rel.StartsWith('/')) return rel;
        return dir.Length == 0 ? rel : $"{dir}\\{rel.Replace('/', '\\')}";
    }

    public static string BookPath(string rel, LibraryTarget t)
        => t.IsLocal ? LocalBookPath(rel, t.LocalDir) : SmbBookPath(rel, t.Smb!);

    private static CatalogDbException Classify(Exception ex, SmbTarget s)
    {
        string m = (ex.GetType().Name + " " + ex.Message).ToLowerInvariant();
        string short_ = m.Length > 300 ? m[..300] : m;
        // Reading a FOLDER as a file throws UnauthorizedAccessException —
        // report the real problem, not a login failure (2026-09-19: bare
        // "calibre" folder was misreported as bad credentials).
        try
        {
            if (Directory.Exists(s.UncPath))
                return CatalogDbException.NotFound(s.SmbPath,
                    $"'{s.RemotePath}' is a folder — the Calibre path must point at the database file (e.g. calibre/metadata.db). ({short_})");
        }
        catch
        {
        }
        if (m.Contains("sharing"))
            return CatalogDbException.Network(s.SmbPath,
                $"Sharing violation — Calibre likely has the library open and locked it. Close Calibre or wait a moment and retry. ({short_})");
        if (m.Contains("logon") || m.Contains("access_denied") || m.Contains("unauthorized") ||
            (m.Contains("auth") && !m.Contains("author")))
            return CatalogDbException.AuthFailed(s.SmbPath, short_);
        if (m.Contains("not found") || m.Contains("no such") || m.Contains("could not find") ||
            m.Contains("bad_netpath") || m.Contains("not_found") || m.Contains("0x"))
            return CatalogDbException.NotFound(s.SmbPath, short_);
        return CatalogDbException.Network(s.SmbPath, short_);
    }

    private static string Trunc(string m) => m.Length > 300 ? m[..300] : m;
}
