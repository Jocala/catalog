// Minimal file logger for headless verification (no debugger attached
// to the VM sessions this runs under). Appends; the shell sets FilePath
// before first use. Never throws.

namespace CatalogWinUICore;

public static class Log
{
    private static readonly object Gate = new();
    public static string FilePath { get; set; } =
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Jocala", "Catalog", "app_log.txt");

    public static void Info(string tag, string msg) => Write("I", tag, msg);
    public static void Error(string tag, string msg) => Write("E", tag, msg);

    private static void Write(string level, string tag, string msg)
    {
        try
        {
            lock (Gate)
            {
                string? dir = Path.GetDirectoryName(FilePath);
                if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);
                File.AppendAllText(FilePath,
                    $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} [{level}] {tag}: {msg}\n");
            }
        }
        catch
        {
        }
    }
}
