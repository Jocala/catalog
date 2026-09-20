// Port of SmbCatalogDB CatalogDBError (LocalizedError -> exception with Kind).
// Message text mirrors the Swift errorDescription so dialogs read the same.

namespace CatalogWin.Services;

public enum CatalogDbKind
{
    NotConfigured,
    NotFound,
    AuthFailed,
    Network,
    Corrupt,
}

public sealed class CatalogDbException : Exception
{
    public CatalogDbKind Kind { get; }

    public CatalogDbException(CatalogDbKind kind, string message) : base(message)
    {
        Kind = kind;
    }

    public static CatalogDbException NotConfigured(string m) => new(CatalogDbKind.NotConfigured, m);
    public static CatalogDbException NotFound(string path, string detail)
        => new(CatalogDbKind.NotFound,
            $"Calibre database not found at {path}.\nCheck Settings → SMB server and Calibre path.\n{detail}");
    public static CatalogDbException AuthFailed(string path, string detail)
        => new(CatalogDbKind.AuthFailed,
            $"SMB login failed for {path}.\nCheck Settings → SMB user and password.\n{detail}");
    public static CatalogDbException Network(string path, string detail)
        => new(CatalogDbKind.Network, $"Could not reach the Calibre database at {path}.\n{detail}");
    public static CatalogDbException Corrupt(string m) => new(CatalogDbKind.Corrupt, $"Calibre database unreadable: {m}");
}
