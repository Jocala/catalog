// Port of CatalogCore ReaderLog errors-only policy.
// Thread-safe; appends to %APPDATA%/com.jocala.Catalog/app_log.txt.
// Rotates once at startup (.log -> .old.log, one backup kept).
// Rule: session headers + summaries + errors only. Per-book progress is banned.

namespace CatalogWin.Core;

using System.IO;

public sealed class AppLog
{
    public static AppLog Shared { get; } = new AppLog();

    private readonly object _gate = new();
    private bool _rotated;

    private AppLog() { }

    public void Info(string tag, string msg) => Write("I", tag, msg);
    public void Error(string tag, string msg) => Write("E", tag, msg);
    public void Debug(string tag, string msg) => Write("D", tag, msg);

    private void Write(string level, string tag, string msg)
    {
        lock (_gate)
        {
            try
            {
                if (!_rotated)
                {
                    _rotated = true;
                    Rotate();
                }
                File.AppendAllText(AppPaths.LogFile,
                    $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} [{level}] {tag}: {msg}{Environment.NewLine}");
            }
            catch
            {
                // Logging must never crash the app.
            }
        }
    }

    private static void Rotate()
    {
        try
        {
            if (File.Exists(AppPaths.OldLogFile))
                File.Delete(AppPaths.OldLogFile);
            if (File.Exists(AppPaths.LogFile))
                File.Move(AppPaths.LogFile, AppPaths.OldLogFile);
        }
        catch
        {
        }
    }
}
