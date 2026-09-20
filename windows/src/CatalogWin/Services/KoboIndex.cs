// Port of macos/Sources/ReaderCatalogGUI/KoboIndex.swift.
// Whole-library Kobo index cache — one find, not per-book.

using System.IO;
using CatalogWin.Core;

namespace CatalogWin.Services;

public static class KoboIndex
{
    public static List<string>? CachedPaths()
    {
        try
        {
            string file = AppPaths.KoboIndexFile;
            if (!File.Exists(file)) return null;
            var lines = File.ReadAllLines(file)
                .Select(l => l.Trim())
                .Where(l => l.Length > 0)
                .ToList();
            return lines.Count == 0 ? null : lines;
        }
        catch
        {
            return null;
        }
    }

    public static void Save(IEnumerable<string> paths)
    {
        try
        {
            File.WriteAllText(AppPaths.KoboIndexFile, string.Join("\n", paths) + "\n");
        }
        catch (Exception ex)
        {
            AppLog.Shared.Error("Kobo", $"index save failed: {ex.Message}");
        }
    }
}
