// Data windowing for the catalog grids (replaces VirtualizingWrapPanel,
// which never starts its generator after a collapsed first measure).
// A stock UniformGrid (fixed 4 columns, macOS LazyVGrid count:4 parity)
// renders at most one window (~12 rows) between two spacer
// Borders that preserve the full scroll extent, so the UX is identical to a
// free-scrolling 6755-tile grid while memory stays bounded (~60 tiles).
// All math is pure (no WPF) and headless-tested.

using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace CatalogWin.UI;

public sealed class ItemWindow : INotifyPropertyChanged
{
    public const double CellHeight = 300;
    /// <summary>Gallery column count (macOS parity: fixed 4, not width-derived).</summary>
    public const int FixedColumns = 4;
    public const int OverscanRows = 6;
    public const int MaxWindowRows = 14;

    public ObservableCollection<TileItem> Window { get; } = new();

    private double _topPad;
    public double TopPad
    {
        get => _topPad;
        private set { _topPad = value; OnPropertyChanged(); }
    }

    private double _bottomPad;
    public double BottomPad
    {
        get => _bottomPad;
        private set { _bottomPad = value; OnPropertyChanged(); }
    }

    private IList<TileItem>? _source = new List<TileItem>();
    private int _cols = FixedColumns;

    public event PropertyChangedEventHandler? PropertyChanged;
    private void OnPropertyChanged([CallerMemberName] string? n = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(n));

    public void SetSource(IList<TileItem>? source) => SetSource(source, FixedColumns);

    public void SetSource(IList<TileItem>? source, int cols)
    {
        _source = source;
        _cols = Math.Max(1, cols);
        Recompute(0, 600);
    }

    public void OnScroll(double offsetY, double viewportH)
    {
        Recompute(offsetY, viewportH);
    }

    private void Recompute(double offsetY, double viewportH)
    {
        var src = _source;
        if (src is null || src.Count == 0 || _cols <= 0)
        {
            Window.Clear();
            TopPad = 0;
            BottomPad = 0;
            return;
        }
        int totalRows = (src.Count + _cols - 1) / _cols;
        int firstRow = Math.Max(0, (int)(offsetY / CellHeight) - OverscanRows);
        int lastRow = Math.Min(totalRows - 1,
            (int)Math.Ceiling((offsetY + Math.Max(1, viewportH)) / CellHeight) + OverscanRows - 1);
        // Cap the window so a tall viewport can't realize thousands at once.
        if (lastRow - firstRow + 1 > MaxWindowRows)
            lastRow = firstRow + MaxWindowRows - 1;
        int first = firstRow * _cols;
        int last = Math.Min(src.Count - 1, (lastRow + 1) * _cols - 1);
        if (last < first)
        {
            Window.Clear();
            TopPad = 0;
            BottomPad = totalRows * CellHeight;
            return;
        }
        // Rebuild only when the window actually moved.
        if (Window.Count == last - first + 1 &&
            Window.Count > 0 &&
            ReferenceEquals(Window[0], src[first]) &&
            ReferenceEquals(Window[Window.Count - 1], src[last]))
            return;
        Window.Clear();
        for (int i = first; i <= last; i++)
            Window.Add(src[i]);
        TopPad = firstRow * CellHeight;
        BottomPad = Math.Max(0, (totalRows - 1 - lastRow) * CellHeight);
    }
}
