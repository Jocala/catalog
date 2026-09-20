// Fork of ReaderWin AppPaths for Jocala Catalog (com.jocala.catalog).
// Windows data root — %APPDATA%/com.jocala.Catalog/ (covers/,
// thumbnails/, kobo_index.txt, app_log.txt, settings.json). Isolated from
// the Reader app's %APPDATA%/.jreader/ — never share the store.

namespace CatalogWin.Core;

using System.IO;

public static class AppPaths
{
    public static string Base
    {
        get
        {
            string dir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "com.jocala.Catalog");
            Directory.CreateDirectory(dir);
            return dir;
        }
    }

    public static string CoversDir => Ensure(Path.Combine(Base, "covers"));
    public static string ThumbnailsDir => Ensure(Path.Combine(Base, "thumbnails"));
    public static string LogFile => Path.Combine(Base, "app_log.txt");
    public static string OldLogFile => Path.Combine(Base, "app_log_old.txt");
    public static string KoboIndexFile => Path.Combine(Base, "kobo_index.txt");

    private static string Ensure(string dir)
    {
        Directory.CreateDirectory(dir);
        return dir;
    }
}
