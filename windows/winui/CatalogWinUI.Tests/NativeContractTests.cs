// Facade contract tests: envelope handling, config shape, fresh-start
// mapping. No native calls (no catalog_ffi needed), no UI, no STA.

using System.Text.Json;
using Xunit;

namespace CatalogWinUI.Tests;

public sealed class NativeContractTests
{
    [Fact]
    public void UnwrapOk_ReturnsPayload()
    {
        var ok = CatalogWinUICore.Native.Unwrap("{\"ok\":{\"books\":6755}}");
        Assert.Equal(6755, ok.GetProperty("books").GetInt32());
    }

    [Fact]
    public void UnwrapError_ThrowsWithMessage()
    {
        var ex = Assert.Throws<CatalogWinUICore.CatalogDbException>(
            () => CatalogWinUICore.Native.Unwrap("{\"error\":\"boom\"}"));
        Assert.Equal("boom", ex.Message);
    }

    [Fact]
    public void BuildConfigSmb_CarriesAllFields()
    {
        var settings = new CatalogWinUICore.AppSettings();
        settings.Passwords["dbhost"] = "s3cret";
        var server = new CatalogWinUICore.SmbServer(
            Host: "dbhost", User: "u", Domain: "d",
            Shares: new() { new("ebooks", "calibre/metadata.db") });
        using var doc = JsonDocument.Parse(
            CatalogWinUICore.Native.BuildConfig(settings, server));
        var root = doc.RootElement;
        Assert.Equal("smb", root.GetProperty("source").GetString());
        Assert.Equal("dbhost", root.GetProperty("host").GetString());
        Assert.Equal("ebooks", root.GetProperty("share").GetString());
        Assert.Equal("calibre", root.GetProperty("remote_dir").GetString());
        Assert.Equal("u", root.GetProperty("user").GetString());
        Assert.Equal("s3cret", root.GetProperty("pass").GetString());
        Assert.Equal("d", root.GetProperty("domain").GetString());
    }

    [Fact]
    public void BuildConfigLocal_OnlyDirMatters()
    {
        var settings = new CatalogWinUICore.AppSettings
        {
            LibrarySource = "local",
            LocalLibraryDir = @"D:\books",
        };
        using var doc = JsonDocument.Parse(
            CatalogWinUICore.Native.BuildConfig(settings, new()));
        Assert.Equal("local", doc.RootElement.GetProperty("source").GetString());
        Assert.Equal(@"D:\books", doc.RootElement.GetProperty("local_dir").GetString());
    }

    [Fact]
    public void MapDbError_NotConfigured_StaysSilent()
    {
        Assert.Null(CatalogWinUICore.Native.MapDbError(
            new CatalogWinUICore.CatalogDbException("not configured: no local library folder")));
    }

    [Fact]
    public void MapDbError_RealFailure_KeepsMessage()
    {
        Assert.Equal("unreachable",
            CatalogWinUICore.Native.MapDbError(new Exception("unreachable")));
    }
}
