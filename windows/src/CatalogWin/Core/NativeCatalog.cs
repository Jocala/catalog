// P/Invoke bridge to catalog_ffi (the Rust catalog-core C ABI).
// Design: JSON in, JSON out. Every native call blocks (SMB + SQLite run on
// the Rust-side tokio runtime), so ALL wrappers go through Task.Run — never
// call these on the WPF UI thread. Errors surface as CatalogDbException
// with the same dialog text as the C# Services backend.
//
// DLL placement (win10): catalog_ffi.dll sits next to CatalogWin.exe
// (cargo build --release on the VM, copy into the publish dir).
// Fallback: Services/CalibreDb+SmbReader+Covers (pure C#, OS UNC) stays
// intact — if the Rust transport ever regresses, flip the call sites back.

using System.Runtime.InteropServices;
using System.Text.Json;
using System.IO;

namespace CatalogWin.Core;

[StructLayout(LayoutKind.Sequential)]
internal struct NativeBytes
{
    public IntPtr Ptr;
    public UIntPtr Len;
}

public sealed class NativeCatalogException : Exception
{
    public NativeCatalogException(string message) : base(message) { }
}

public static class NativeCatalog
{
    private const string Dll = "catalog_ffi";

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_version();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_abi();

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
    private static extern IntPtr catalog_search(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string paramsJson);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_detail(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson, long bookId);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern NativeBytes catalog_cover(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string bookPath,
        uint maxW, uint maxH);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_kobo_predict(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string authorSort,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string authorsNatural);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr catalog_kobo_match(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string candidatesJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string author);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void catalog_string_free(IntPtr s);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void catalog_bytes_free(NativeBytes b);

    // -- envelope -----------------------------------------------------------

    private static string TakeString(IntPtr p)
    {
        if (p == IntPtr.Zero)
            throw new NativeCatalogException("native call returned null (misuse)");
        try
        {
            return Marshal.PtrToStringUTF8(p)
                ?? throw new NativeCatalogException("native string was not UTF-8");
        }
        finally
        {
            catalog_string_free(p);
        }
    }

    /// Unwrap {"ok":…} or throw NativeCatalogException({"error":…}).
    public static JsonElement Unwrap(string raw)
    {
        using JsonDocument doc = JsonDocument.Parse(raw);
        JsonElement root = doc.RootElement;
        if (root.TryGetProperty("error", out JsonElement err))
            throw new NativeCatalogException(err.GetString() ?? "unknown native error");
        return root.GetProperty("ok").Clone();
    }

    private static JsonElement Call(Func<IntPtr> fn) => Unwrap(TakeString(fn()));

    // -- config -------------------------------------------------------------
    // Mirrors catalog-ffi file_source(): local_dir for local, host/share/
    // remote_dir/user/pass/domain for SMB. Passwords pass per-call; the DLL
    // persists nothing (this app's settings.json remains the only store).

    public static string BuildConfig(
        Services.AppSettings settings, Services.SmbServer server, string calibrePath)
    {
        if (settings.LibrarySource == "local")
        {
            return JsonSerializer.Serialize(new
            {
                source = "local",
                local_dir = settings.LocalLibraryDir,
            });
        }
        string remoteDir = calibrePath.Trim().Trim('/').Replace('\\', '/');
        if (remoteDir.EndsWith("metadata.db", StringComparison.OrdinalIgnoreCase))
            remoteDir = Path.GetDirectoryName(remoteDir)?.Replace('\\', '/') ?? "";
        string share = server.Shares?.FirstOrDefault()?.Name ?? "";
        return JsonSerializer.Serialize(new
        {
            source = "smb",
            local_dir = "",
            host = server.Host,
            share,
            remote_dir = remoteDir,
            user = server.User,
            pass = settings.PasswordFor(server.Host) ?? "",
            domain = server.Domain,
        });
    }

    // -- async wrappers (all off-UI via Task.Run) ----------------------------

    public static Task<int> GetCountAsync(string config) =>
        Task.Run(() =>
        {
            JsonElement ok = Call(() => catalog_open_count(config));
            return ok.GetProperty("books").GetInt32();
        });

    public static Task<string> GetBooksJsonAsync(
        string config, string query, bool desc, bool byAuthor, bool byDate) =>
        Task.Run(() => TakeStringRaw(() =>
            catalog_fetch_books(config, query, desc, byAuthor, byDate)));

    public static Task<string> BrowseJsonAsync(
        string config, string mode, bool desc, bool byAuthor) =>
        Task.Run(() => TakeStringRaw(() => catalog_browse(config, mode, desc, byAuthor)));

    public static Task<string> SearchJsonAsync(string config, string paramsJson) =>
        Task.Run(() => TakeStringRaw(() => catalog_search(config, paramsJson)));

    public static Task<string> DetailJsonAsync(string config, long bookId) =>
        Task.Run(() => TakeStringRaw(() => catalog_detail(config, bookId)));

    /// Raw envelope JSON (caller parses with System.Text.Json into the
    /// CatalogModels DTOs). Kept as string: shapes match core exactly.
    private static string TakeStringRaw(Func<IntPtr> fn) => TakeString(fn());

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

    public static Task<string> KoboPredictAsync(string title, string authorSort, string natural) =>
        Task.Run(() =>
        {
            JsonElement ok = Call(() => catalog_kobo_predict(title, authorSort, natural));
            return ok.GetProperty("path").GetString() ?? "";
        });

    public static Task<(string[] Strict, string[] TitleOnly)> KoboMatchAsync(
        string[] candidates, string title, string author) =>
        Task.Run(() =>
        {
            string cands = JsonSerializer.Serialize(candidates);
            JsonElement ok = Call(() => catalog_kobo_match(cands, title, author));
            string[] strict = ok.GetProperty("strict").EnumerateArray()
                .Select(e => e.GetString() ?? "").ToArray();
            string[] titleOnly = ok.GetProperty("title_only").EnumerateArray()
                .Select(e => e.GetString() ?? "").ToArray();
            return (strict, titleOnly);
        });

    public static Task<(string Version, int PtrSize)> ProbeAsync() =>
        Task.Run(() =>
        {
            // DllNotFoundException propagates when catalog_ffi.dll is missing —
            // call sites treat that as "native backend unavailable".
            JsonElement v = Call(() => catalog_version());
            JsonElement a = Call(() => catalog_abi());
            return (v.GetProperty("version").GetString() ?? "?",
                    a.GetProperty("ptr_size").GetInt32());
        });
}
