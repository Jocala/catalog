using Microsoft.UI.Xaml;

namespace CatalogWinUI;

public sealed partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);

        RootFrame.Navigate(typeof(MainPage));
    }
}
