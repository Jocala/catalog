// P/Invoke facade over catalog_ffi (Rust catalog-core C ABI).
// Design: JSON in, JSON out. Every native call blocks (SMB + SQLite run on
// the Rust-side tokio runtime), so ALL wrappers go through Task.Run — never
// call these on the UI thread.
//
// Correctness notes (this is the first live C# consumer of the ABI):
// - Rust `bool` is 1 byte: every bool param carries I1, never the default
//   4-byte BOOL.
// - Rust strings are UTF-8: every string param is LPUTF8Str (the default
//   ANSI marshal would mangle non-ASCII authors/titles). Returns are read
//   with PtrToStringUTF8.
// - Every returned string is freed with catalog_string_free; cover blobs
//   (by-value struct) with catalog_bytes_free. Null cover ptr = miss.

using System.Runtime.InteropServices;
using System.Text.Json;

namespace CatalogWinUICore;

[StructLayout(LayoutKind.Sequential)]
internal struct NativeBytes
{
    public IntPtr Ptr;
    public UIntPtr Len;
}

public sealed class CatalogDbException(string message) : Exception(message);

public static class Native
{
    private const string Dll = "catalog_ffi";

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_version();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_open_count(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_fetch_books(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string query,
        [MarshalAs(UnmanagedType.I1)] bool sortDescending,
        [MarshalAs(UnmanagedType.I1)] bool sortByAuthor,
        [MarshalAs(UnmanagedType.I1)] bool sortByDate);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_browse(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string mode,
        [MarshalAs(UnmanagedType.I1)] bool sortDescending,
        [MarshalAs(UnmanagedType.I1)] bool sortByAuthor);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern NativeBytes catalog_cover(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string bookPath,
        uint maxW,
        uint maxH);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void catalog_string_free(IntPtr s);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void catalog_bytes_free(NativeBytes b);

    // -- envelope -----------------------------------------------------------

    private static string TakeString(IntPtr p)
    {
        if (p == IntPtr.Zero)
            throw new CatalogDbException("native call returned null (misuse)");
        try
        {
            return Marshal.PtrToStringUTF8(p)
                ?? throw new CatalogDbException("native string was not UTF-8");
        }
        finally
        {
            catalog_string_free(p);
        }
    }

    /// Unwrap {"ok":…} or throw CatalogDbException({"error":…}).
    public static JsonElement Unwrap(string raw)
    {
        using JsonDocument doc = JsonDocument.Parse(raw);
        JsonElement root = doc.RootElement;
        if (root.TryGetProperty("error", out JsonElement err))
            throw new CatalogDbException(err.GetString() ?? "unknown native error");
        return root.GetProperty("ok").Clone();
    }

    /// Fresh start (not configured) is not an error: the gallery's empty
    /// state points at Settings, so these stay silent. Only real failures
    /// surface as exception text.
    public static string? MapDbError(Exception ex)
        => ex.Message.StartsWith("not configured", StringComparison.OrdinalIgnoreCase)
            ? null
            : ex.Message;

    private static JsonElement Call(Func<IntPtr> fn) => Unwrap(TakeString(fn()));

    private static List<T> CallList<T>(Func<IntPtr> fn)
    {
        JsonElement ok = Call(fn);
        return JsonSerializer.Deserialize<List<T>>(ok.GetRawText()) ?? new();
    }

    // -- config -------------------------------------------------------------
    // Mirrors catalog-ffi file_source(): local_dir for local, host/share/
    // remote_dir/user/pass/domain for SMB. Passwords pass per-call; the DLL
    // persists nothing (the shell's settings file remains the only store).

    public static string BuildConfig(AppSettings settings, SmbServer server)
    {
        if (settings.LibrarySource == "local")
        {
            return JsonSerializer.Serialize(new
            {
                source = "local",
                local_dir = settings.LocalLibraryDir,
            });
        }
        SmbShare? share = server.Shares?.FirstOrDefault();
        string remoteDir = (share?.CalibreMetadataPath ?? "").Trim().Trim('/').Replace('\\', '/');
        if (remoteDir.EndsWith("metadata.db", StringComparison.OrdinalIgnoreCase))
            remoteDir = Path.GetDirectoryName(remoteDir)?.Replace('\\', '/') ?? "";
        return JsonSerializer.Serialize(new
        {
            source = "smb",
            local_dir = "",
            host = server.Host,
            share = share?.Name ?? "",
            remote_dir = remoteDir,
            user = server.User,
            pass = settings.PasswordFor(server.Host) ?? "",
            domain = server.Domain,
        });
    }

    // -- async wrappers (all off-UI via Task.Run) ----------------------------

    public static Task<string> VersionAsync() =>
        Task.Run(() => Call(() => catalog_version()).GetProperty("version").GetString() ?? "");

    public static Task<int> GetCountAsync(string config) =>
        Task.Run(() => Call(() => catalog_open_count(config)).GetProperty("books").GetInt32());

    public static Task<List<BookDto>> FetchBooksAsync(
        string config, string query, bool desc, bool byAuthor, bool byDate) =>
        Task.Run(() => CallList<BookDto>(() =>
            catalog_fetch_books(config, query, desc, byAuthor, byDate)));

    public static Task<List<AuthorDto>> BrowseAuthorsAsync(string config, bool desc) =>
        Task.Run(() => CallList<AuthorDto>(() => catalog_browse(config, "authors", desc, false)));

    public static Task<List<SeriesDto>> BrowseSeriesAsync(string config, bool desc, bool byAuthor) =>
        Task.Run(() => CallList<SeriesDto>(() => catalog_browse(config, "series", desc, byAuthor)));

    public static Task<List<TagDto>> BrowseTagsAsync(string config, bool desc) =>
        Task.Run(() => CallList<TagDto>(() => catalog_browse(config, "tags", desc, false)));

    /// Cover JPEG bytes at max_WxH (engine scales), or null on a miss.
    /// Caller owns caching + decode; this only transports bytes.
    public static Task<byte[]?> CoverAsync(string config, string bookPath, uint maxW, uint maxH) =>
        Task.Run(() =>
        {
            NativeBytes b = catalog_cover(config, bookPath, maxW, maxH);
            if (b.Ptr == IntPtr.Zero || b.Len == UIntPtr.Zero)
                return (byte[]?)null;
            try
            {
                byte[] buf = new byte[(int)(uint)b.Len];
                Marshal.Copy(b.Ptr, buf, 0, buf.Length);
                return (byte[]?)buf;
            }
            finally
            {
                catalog_bytes_free(b);
            }
        });
}
