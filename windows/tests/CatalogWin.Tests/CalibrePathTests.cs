// Regression gate for the 2026-09-19 folder-path bug: a bare Calibre
// folder ("calibre") used to be read as a file -> UnauthorizedAccessException
// misreported as login failure, leaving the grid (and covers) empty.
// ResolveTarget must normalize folder input to calibre/metadata.db.

using CatalogWin.Services;
using Xunit;

namespace CatalogWin.Tests;

public sealed class CalibrePathTests
{
    private static AppSettings SettingsWith(string calibrePath) => new()
    {
        LibrarySource = "smb",
        SmbServers = new()
        {
            new SmbServer(
                Host: "dummy",
                Shares: new() { new SmbShare(Name: "ebooks", CalibreMetadataPath: calibrePath) }),
        },
    };

    [Fact]
    public void FolderInput_ResolvesToMetadataDb()
    {
        LibraryTarget t = SmbReader.ResolveTarget(SettingsWith("calibre"));
        Assert.NotNull(t.Smb);
        Assert.Equal("calibre/metadata.db", t.Smb!.RemotePath);
        Assert.Equal("calibre", t.Smb.LibRoot);
    }

    [Fact]
    public void FileInput_Unchanged()
    {
        LibraryTarget t = SmbReader.ResolveTarget(SettingsWith("calibre/metadata.db"));
        Assert.NotNull(t.Smb);
        Assert.Equal("calibre/metadata.db", t.Smb!.RemotePath);
        Assert.Equal("calibre", t.Smb.LibRoot);
    }

    [Fact]
    public void BackslashAndCaseVariants_Normalized()
    {
        LibraryTarget t = SmbReader.ResolveTarget(SettingsWith(@"calibre\METADATA.DB"));
        Assert.NotNull(t.Smb);
        Assert.Equal("calibre/METADATA.DB", t.Smb!.RemotePath);
    }
}
