// Pure-math tests for UI/ItemWindow (no WPF, no STA needed).

using CatalogWin.UI;
using Xunit;

namespace CatalogWin.Tests;

public sealed class WindowTests
{
    private static List<TileItem> Books(int n)
    {
        var list = new List<TileItem>(n);
        for (int i = 0; i < n; i++)
            list.Add(new TileItem { Id = i, Title = $"t{i}", Path = $"p{i}" });
        return list;
    }

    [Fact]
    public void EmptySource_EmptyWindow()
    {
        var w = new ItemWindow();
        w.SetSource(Books(0), 4);
        Assert.Empty(w.Window);
        Assert.Equal(0, w.TopPad);
        Assert.Equal(0, w.BottomPad);
    }

    [Fact]
    public void TopWindow_StartsAtZero()
    {
        var books = Books(6755);
        var w = new ItemWindow();
        w.SetSource(books, 4);
        w.OnScroll(0, 600);
        Assert.Same(books[0], w.Window[0]);
        Assert.Equal(0, w.TopPad);
        // 1689 rows total; window rows 0..7 -> bottom covers the rest.
        Assert.Equal((1689 - 1 - 7) * 300, w.BottomPad);
        Assert.True(w.Window.Count <= 14 * 4, $"count={w.Window.Count}");
    }

    [Fact]
    public void DeepScroll_WindowFollows()
    {
        var books = Books(6755);
        var w = new ItemWindow();
        w.SetSource(books, 4);
        w.OnScroll(30000, 600); // row 100
        Assert.Same(books[376], w.Window[0]); // (100-6)*4
        Assert.Equal(94 * 300, w.TopPad);
        Assert.True(w.Window.Count <= 14 * 4, $"count={w.Window.Count}");
    }

    [Fact]
    public void SameWindow_NoRebuild()
    {
        var books = Books(6755);
        var w = new ItemWindow();
        w.SetSource(books, 4);
        w.OnScroll(0, 600);
        var first = w.Window[0];
        int count = w.Window.Count;
        w.OnScroll(0, 600); // identical viewport -> no rebuild
        Assert.Same(first, w.Window[0]);
        Assert.Equal(count, w.Window.Count);
    }

    [Fact]
    public void HugeViewport_Capped()
    {
        var books = Books(6755);
        var w = new ItemWindow();
        w.SetSource(books, 4);
        w.OnScroll(0, 10000);
        Assert.True(w.Window.Count <= 14 * 4, $"count={w.Window.Count}");
        // Pads still account for the full extent.
        Assert.Equal(0, w.TopPad);
        Assert.True(w.BottomPad > 0, $"bottom={w.BottomPad}");
    }

    [Fact]
    public void Gallery_FixedFourColumns()
    {
        // macOS parity: the gallery is always 4 across, never width-derived.
        Assert.Equal(4, ItemWindow.FixedColumns);
        var books = Books(6755);
        var w = new ItemWindow();
        w.SetSource(books);
        w.OnScroll(0, 600);
        Assert.Same(books[0], w.Window[0]);
        Assert.True(w.Window.Count <= 14 * 4, $"count={w.Window.Count}");
    }
}
