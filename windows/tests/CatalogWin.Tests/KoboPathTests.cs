// Byte-exactness gate for Core/KoboPath.cs.
// Vectors mirror C:/source/reader/qt/tests/tst_kobopath.cpp and the Swift original.
// If these fail, Kobo opens break. Do not weaken them.

using CatalogWin.Core;
using Xunit;

namespace CatalogWin.Tests;

public sealed class KoboPathTests
{
    [Fact]
    public void TitleSort_MovesLeadingArticles()
    {
        Assert.Equal("Emma, The", KoboPath.TitleSort("The Emma"));
        Assert.Equal("Corpse, A", KoboPath.TitleSort("A Corpse"));
        Assert.Equal("Atlas, An", KoboPath.TitleSort("An Atlas"));
        Assert.Equal("Emma", KoboPath.TitleSort("Emma"));
    }

    [Fact]
    public void AuthorSort_CommaMethod()
    {
        Assert.Equal("Bruen, Ken", KoboPath.AuthorSort("Ken Bruen"));
        Assert.Equal("Bruen, Ken", KoboPath.AuthorSort("Bruen, Ken"));
    }

    [Fact]
    public void NaturalName_InvertsSort()
    {
        Assert.Equal("Ken Bruen", KoboPath.NaturalName("Bruen, Ken"));
    }

    [Fact]
    public void Sanitize_FatRules()
    {
        // Observed calibre/Kobo FAT behaviour (see KoboPath.swift comments).
        Assert.Equal("Smiley_s", KoboPath.Sanitize("Smiley's"));
        Assert.Equal("Hell_s", KoboPath.Sanitize("Hell's"));
        Assert.Equal("a_b_c", KoboPath.Sanitize("a/b:c"));
    }

    [Fact]
    public void Sanitize_Diacritics()
    {
        Assert.Equal("Carre, John le", KoboPath.Sanitize("Carré, John le"));
    }

    [Fact]
    public void PredictedPath_Emma()
    {
        string p = KoboPath.PredictedPath("Emma", "Austen, Jane", "Jane Austen");
        Assert.Equal("/mnt/onboard/Austen, Jane/Emma - Jane Austen.kepub.epub", p);
    }

    [Fact]
    public void PredictedPath_TheTitleFolds()
    {
        string p2 = KoboPath.PredictedPath("The Hobbit", "Tolkien, J. R. R.", "J. R. R. Tolkien");
        Assert.Contains("Hobbit, The - ", p2);
    }

    [Fact]
    public void PredictedPath_Within185()
    {
        string p = KoboPath.PredictedPath("Emma", "Austen, Jane", "Jane Austen");
        // UTF-16 bytes, mirrors Qt gate (toStdU16String().size() * 2 <= 185 * 2).
        Assert.True(p.Length * 2 <= 185 * 2);
    }
}
