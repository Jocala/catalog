// Placeholder hint text for TextBox/PasswordBox (macOS parity).
// Classic WPF has no built-in prompt/PlaceholderText (unlike SwiftUI
// TextField(prompt:) or WinUI), so this attached behavior overlays a
// muted, hit-test-invisible hint shown only while the field is empty:
//   <TextBox ui:Watermark.Text="hostname or IP" ... />
// Hint copy mirrors the Mac prompts 1:1 (blank where Mac has none).
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;

namespace CatalogWin.UI;

public static class Watermark
{
    public static readonly DependencyProperty TextProperty =
        DependencyProperty.RegisterAttached("Text", typeof(string), typeof(Watermark),
            new PropertyMetadata("", OnTextChanged));

    public static string GetText(DependencyObject d) => (string)d.GetValue(TextProperty);
    public static void SetText(DependencyObject d, string value) => d.SetValue(TextProperty, value);

    private static void OnTextChanged(DependencyObject d, DependencyPropertyChangedEventArgs e)
    {
        if (d is not Control control) return;
        if (control.IsLoaded) Attach(control);
        else control.Loaded += OnControlLoaded;
        switch (control)
        {
            case TextBox tb: tb.TextChanged += (_, _) => Refresh(control); break;
            case PasswordBox pb: pb.PasswordChanged += (_, _) => Refresh(control); break;
        }
        Refresh(control);
    }

    private static void OnControlLoaded(object sender, RoutedEventArgs e)
    {
        if (sender is Control control)
        {
            control.Loaded -= OnControlLoaded;
            Attach(control);
            Refresh(control);
        }
    }

    private static void Attach(Control control)
    {
        if (string.IsNullOrEmpty(GetText(control))) return;
        var layer = AdornerLayer.GetAdornerLayer(control);
        if (layer is null) return;
        foreach (var a in layer.GetAdorners(control) ?? System.Array.Empty<Adorner>())
            if (a is WatermarkAdorner) return;
        layer.Add(new WatermarkAdorner(control));
    }

    private static void Refresh(Control? control)
    {
        if (control is null) return;
        string hint = GetText(control);
        string text = control switch
        {
            TextBox tb => tb.Text,
            PasswordBox pb => pb.Password,
            _ => "",
        };
        var layer = AdornerLayer.GetAdornerLayer(control);
        if (layer is null) return;
        foreach (var a in layer.GetAdorners(control) ?? System.Array.Empty<Adorner>())
        {
            if (a is WatermarkAdorner w)
                w.Visibility = hint.Length > 0 && text.Length == 0
                    ? Visibility.Visible : Visibility.Collapsed;
        }
    }

    private sealed class WatermarkAdorner : Adorner
    {
        private readonly TextBlock _hint;
        public WatermarkAdorner(UIElement adorned) : base(adorned)
        {
            IsHitTestVisible = false;
            _hint = new TextBlock
            {
                Text = GetText(adorned),
                Foreground = SystemColors.GrayTextBrush,
                FontStyle = FontStyles.Italic,
                Margin = new Thickness(4, 1, 0, 0),
                VerticalAlignment = VerticalAlignment.Center,
            };
            AddVisualChild(_hint);
        }
        protected override int VisualChildrenCount => 1;
        protected override Visual GetVisualChild(int index) => _hint;
        protected override Size MeasureOverride(Size constraint)
        {
            _hint.Measure(constraint);
            return base.MeasureOverride(constraint);
        }
        protected override Size ArrangeOverride(Size finalSize)
        {
            _hint.Arrange(new Rect(finalSize));
            return base.ArrangeOverride(finalSize);
        }
    }
}
