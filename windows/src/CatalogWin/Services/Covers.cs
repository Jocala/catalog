// Port of macos CatalogCore/Services/ThumbnailService.swift (cover paths only).
// Flow per book path:
//   covers/<coverHash>.jpg fast path (local SSD, no SMB)
//   smb:// -> cover.jpg next to the book (folder → folder/cover.jpg;
//           file → sibling cover.jpg) via WNet UNC -> SHA256 -> cache in covers/
//   http(s):// -> sibling cover.jpg via HttpClient (15s) -> cache
//   local -> folder/cover.jpg or sibling cover.jpg (EPUB-embedded fallback
//           is NOT ported — cover.jpg covers SMB + synced libraries)
// Scale-to-fit INSIDE 120x180, preserving aspect ratio — never crop (mirrors
// centerCropToThumbnail; already-fitting images return as-is). Frozen
// BitmapSource so background threads can hand it to the UI.

using System.IO;
using System.Net.Http;
using System.Text;
using System.Security.Cryptography;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using CatalogWin.Core;

namespace CatalogWin.Services;

public static class Covers
{
    public const int ThumbWidth = 120;
    public const int ThumbHeight = 180;

    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(15) };

    public static async Task<BitmapSource?> ThumbnailAsync(
        string path, string? coverHash, AppSettings? settings = null)
    {
        settings ??= SettingsStore.Load();
        string coversDir = AppPaths.CoversDir;

        if (!string.IsNullOrEmpty(coverHash))
        {
            string hashFile = Path.Combine(coversDir, coverHash + ".jpg");
            if (File.Exists(hashFile) && Decode(await File.ReadAllBytesAsync(hashFile)) is BitmapSource hit)
                return hit;
        }

        // tolerate "/https://..." fileURL artifact
        string effective = path;
        if (effective.StartsWith('/') &&
            (effective.StartsWith("/http://") || effective.StartsWith("/https://") || effective.StartsWith("/smb://")))
            effective = effective[1..];

        if (effective.StartsWith("smb://", StringComparison.OrdinalIgnoreCase) ||
            path.StartsWith("smb://", StringComparison.OrdinalIgnoreCase))
        {
            byte[]? data = await FetchSmbCoverAsync(path, settings);
            if (data is null) return null;
            CacheBytes(coversDir, data);
            return Decode(data);
        }

        string httpPath = effective.StartsWith("http://") || effective.StartsWith("https://") ? effective : path;
        if (httpPath.StartsWith("http://") || httpPath.StartsWith("https://"))
        {
            byte[]? data = await FetchHttpCoverAsync(httpPath);
            if (data is null) return null;
            CacheBytes(coversDir, data);
            return Decode(data);
        }

        string coverFile = CoverSiblingLocal(path);
        if (coverFile.Length > 0 && File.Exists(coverFile))
            return Decode(await File.ReadAllBytesAsync(coverFile));
        return null;
    }

    /// smb://host/share/a/b[/file] -> cover.jpg bytes via WNet UNC.
    public static async Task<byte[]?> FetchSmbCoverAsync(string smbPath, AppSettings? settings = null)
    {
        settings ??= SettingsStore.Load();
        try
        {
            string rest = smbPath.StartsWith("smb://", StringComparison.OrdinalIgnoreCase)
                ? smbPath[6..] : smbPath;
            string[] parts = rest.Split('/', 3);
            if (parts.Length < 2) return null;
            string host = parts[0], share = parts[1];
            string remote = parts.Length > 2 ? parts[2] : "";
            string coverRemote = CoverSiblingRemote(remote);

            SmbServer? server = settings.SmbServers.FirstOrDefault(s => s.Host == host);
            var target = new SmbTarget(host, share, coverRemote,
                server?.User ?? "", settings.PasswordFor(host) ?? "", server?.Domain ?? "", "");
            return await Task.Run(() =>
            {
                SmbReader.WnetEnsure(target);
                return File.ReadAllBytes(target.UncPath);
            });
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("ThumbnailService", $"smb cover fetch failed input='{smbPath}' err={ex.Message}");
            return null;
        }
    }

    private static async Task<byte[]?> FetchHttpCoverAsync(string bookUrl)
    {
        try
        {
            // Calibre paths contain raw spaces/quotes — encode like Swift
            // (urlFragmentAllowed, then ' -> %27 and " -> %22).
            string encoded = EncodeUrl(bookUrl);
            if (!Uri.TryCreate(encoded, UriKind.Absolute, out Uri? book)) return null;
            var cover = new Uri(book, "cover.jpg");
            using var resp = await Http.GetAsync(cover);
            AppLog.Shared.Info("ThumbnailService", $"http cover HTTP {(int)resp.StatusCode} url='{cover}'");
            if (!resp.IsSuccessStatusCode) return null;
            byte[] data = await resp.Content.ReadAsByteArrayAsync();
            return data.Length == 0 ? null : data;
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("ThumbnailService", $"http cover fetch error {ex.Message} url='{bookUrl}'");
            return null;
        }
    }

    private static string EncodeUrl(string url)
    {
        // urlFragmentAllowed equivalent: leave unreserved + fragment chars
        // (incl. '%') alone so existing escapes survive; UTF-8 encode the rest.
        const string allowed = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~!$&'()*+,;=:@/?#[]%";
        var sb = new StringBuilder(url.Length);
        foreach (char c in url)
        {
            if (c < 128 && allowed.Contains(c))
                sb.Append(c);
            else
                foreach (byte b in Encoding.UTF8.GetBytes(c.ToString()))
                {
                    sb.Append('%');
                    sb.Append(b.ToString("X2"));
                }
        }
        return sb.ToString().Replace("'", "%27").Replace("\"", "%22");
    }

    /// Calibre books.path is a FOLDER ("Author/Title (id)"); only a full file
    /// path (ebook suffix) takes a sibling cover.jpg. Dot-presence is NOT a
    /// file test: folders like "01-03.winter.black (9329)" contain dots.
    public static string CoverSiblingRemote(string remotePath)
    {
        string lower = remotePath.ToLowerInvariant();
        if (lower.EndsWith(".epub") || lower.EndsWith(".pdf") || lower.EndsWith(".kepub"))
        {
            int slash = remotePath.LastIndexOf('/');
            return slash < 0 ? remotePath + "/cover.jpg"
                             : remotePath[..slash] + "/cover.jpg";
        }
        return remotePath.TrimEnd('/') + "/cover.jpg";
    }

    private static string CoverSiblingLocal(string path)
    {
        string lower = path.ToLowerInvariant();
        bool isFile = lower.EndsWith(".epub") || lower.EndsWith(".pdf") || lower.EndsWith(".kepub");
        if (!isFile && Directory.Exists(path))
            return Path.Combine(path, "cover.jpg");
        try
        {
            string? dir = Path.GetDirectoryName(path);
            if (dir is null) return "";
            if (!isFile && File.Exists(path)) return Path.Combine(dir, "cover.jpg");
            return isFile ? Path.Combine(dir, "cover.jpg") : Path.Combine(path, "cover.jpg");
        }
        catch
        {
            return "";
        }
    }

    private static void CacheBytes(string coversDir, byte[] data)
    {
        try
        {
            string hash = Convert.ToHexString(SHA256.HashData(data)).ToLowerInvariant();
            string cached = Path.Combine(coversDir, hash + ".jpg");
            if (!File.Exists(cached))
                File.WriteAllBytes(cached, data);
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("ThumbnailService", $"cache write failed: {ex.Message}");
        }
    }

    private static BitmapSource? Decode(byte[] data)
    {
        try
        {
            using var ms = new MemoryStream(data);
            var bmp = new BitmapImage();
            bmp.BeginInit();
            bmp.CacheOption = BitmapCacheOption.OnLoad;
            bmp.StreamSource = ms;
            bmp.EndInit();
            bmp.Freeze();
            return ScaleToFit(bmp);
        }
        catch
        {
            return null;
        }
    }

    private static BitmapSource ScaleToFit(BitmapSource img)
    {
        if (img.PixelWidth <= 0 || img.PixelHeight <= 0) return img;
        double scale = Math.Min((double)ThumbWidth / img.PixelWidth, (double)ThumbHeight / img.PixelHeight);
        if (scale >= 1) return img;
        var scaled = new TransformedBitmap(img, new ScaleTransform(scale, scale));
        scaled.Freeze();
        return scaled;
    }
}
