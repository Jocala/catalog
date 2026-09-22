// Cover cache + settings tests. Temp dirs only, no UI, no STA.

using Xunit;

namespace CatalogWinUI.Tests;

public sealed class CoverCacheTests
{
    [Fact]
    public void Roundtrip_MissThenHit()
    {
        string root = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
        try
        {
            var cache = new CatalogWinUICore.CoverCache(root);
            Assert.False(cache.TryGet("smb://h/s/Author/Title (1)", out _));
            cache.Put("smb://h/s/Author/Title (1)", new byte[] { 1, 2, 3 });
            Assert.True(cache.TryGet("smb://h/s/Author/Title (1)", out string file));
            Assert.Equal(new byte[] { 1, 2, 3 }, File.ReadAllBytes(file));
        }
        finally
        {
            try { Directory.Delete(root, true); } catch { }
        }
    }

    [Fact]
    public void Cap_EvictsOldestFirst()
    {
        string root = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
        try
        {
            // 10-byte cap: each 6-byte put overflows, oldest goes first.
            var cache = new CatalogWinUICore.CoverCache(root, maxBytes: 10);
            cache.Put("a", new byte[6]);
            Thread.Sleep(20);
            cache.Put("b", new byte[6]);
            Assert.False(cache.TryGet("a", out _));
            Assert.True(cache.TryGet("b", out _));
        }
        finally
        {
            try { Directory.Delete(root, true); } catch { }
        }
    }
}

public sealed class SettingsTests
{
    [Fact]
    public void LoadMissingFile_Defaults()
    {
        var s = CatalogWinUICore.SettingsStore.LoadFrom(
            Path.Combine(Path.GetTempPath(), Path.GetRandomFileName()));
        Assert.Equal("smb", s.LibrarySource);
        Assert.False(s.HasSource);
    }

    [Fact]
    public void HasSource_TrueWhenHostPresent()
    {
        var s = new CatalogWinUICore.AppSettings
        {
            SmbServers = new() { new CatalogWinUICore.SmbServer(Host: "dbhost") },
        };
        Assert.True(s.HasSource);
    }

    [Fact]
    public void ParseMinimalJson_KeepsPasswords()
    {
        string file = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
        try
        {
            File.WriteAllText(file,
                "{\"LibrarySource\":\"smb\",\"Passwords\":{\"dbhost\":\"pw\"}," +
                "\"SmbServers\":[{\"Host\":\"dbhost\"}]}");
            var s = CatalogWinUICore.SettingsStore.LoadFrom(file);
            Assert.True(s.HasSource);
            Assert.Equal("pw", s.PasswordFor("dbhost"));
        }
        finally
        {
            try { File.Delete(file); } catch { }
        }
    }
}
