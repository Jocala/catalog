using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;

namespace CatalogWin;

// GPU-less VMs (QXL, no passthrough): force WPF software rendering.
// Do not add D3D/QML/Chromium dependencies to this project.
public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        RenderOptions.ProcessRenderMode = RenderMode.SoftwareOnly;
        base.OnStartup(e);
    }
}
