// Fresh start (NotConfigured) stays silent — the centered No-books empty
// state points at Settings. Real failures still surface as DbError text
// (MainWindow pops up). Pure mapping, no WPF, no STA needed.

using CatalogWin.Services;
using CatalogWin.UI;
using Xunit;

namespace CatalogWin.Tests;

public sealed class DbErrorTests
{
    [Fact]
    public void NotConfigured_MapsToNull()
    {
        Assert.Null(CatalogStore.MapDbError(
            CatalogDbException.NotConfigured("No SMB server configured")));
    }

    [Fact]
    public void RealFailures_KeepMessage()
    {
        var network = CatalogDbException.Network("smb://h/s/", "unreachable");
        Assert.Equal(network.Message, CatalogStore.MapDbError(network));
        var auth = CatalogDbException.AuthFailed("smb://h/s/", "rejected");
        Assert.Equal(auth.Message, CatalogStore.MapDbError(auth));
        var corrupt = CatalogDbException.Corrupt("bad size");
        Assert.Equal(corrupt.Message, CatalogStore.MapDbError(corrupt));
        var plain = new System.Exception("boom");
        Assert.Equal("boom", CatalogStore.MapDbError(plain));
    }
}
