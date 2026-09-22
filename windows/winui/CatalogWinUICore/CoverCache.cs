// On-disk cover cache: book path -> JPEG file. Bounded by total bytes
// (default 256 MB); Put evicts oldest-by-mtime first, so a 6700-book
// library can never grow this without limit. Pure file IO, no UI —
// fully unit-testable. Thread-safe for concurrent fills.

using System.Security.Cryptography;
using System.Text;

namespace CatalogWinUICore;

public sealed class CoverCache
{
    private readonly string _root;
    private readonly long _maxBytes;
    private readonly object _gate = new();

    public CoverCache(string rootDir, long maxBytes = 256L * 1024 * 1024)
    {
        _root = rootDir;
        _maxBytes = maxBytes;
        Directory.CreateDirectory(_root);
    }

    public static string KeyFor(string bookPath)
    {
        byte[] hash = SHA256.HashData(Encoding.UTF8.GetBytes(bookPath));
        return Convert.ToHexString(hash).ToLowerInvariant() + ".jpg";
    }

    public bool TryGet(string bookPath, out string filePath)
    {
        filePath = Path.Combine(_root, KeyFor(bookPath));
        return File.Exists(filePath);
    }

    public void Put(string bookPath, byte[] jpeg)
    {
        lock (_gate)
        {
            string file = Path.Combine(_root, KeyFor(bookPath));
            File.WriteAllBytes(file, jpeg);
            EnforceCap();
        }
    }

    private void EnforceCap()
    {
        var files = new DirectoryInfo(_root).GetFiles("*.jpg");
        long total = files.Sum(f => f.Length);
        if (total <= _maxBytes) return;
        foreach (var f in files.OrderBy(f => f.LastWriteTimeUtc))
        {
            total -= f.Length;
            try { f.Delete(); } catch { }
            if (total <= _maxBytes) break;
        }
    }
}
