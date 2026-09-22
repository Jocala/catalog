// Guards the enclosed-help requirement: Assets/help.html ships as a WPF
// Resource inside the exe — the single-file release has no loose help file.
// No WPF instance, no STA: reads CatalogWin.g.resources directly.

using System.IO;
using System.Resources;
using System.Text;
using Xunit;

namespace CatalogWin.Tests;

public sealed class HelpResourceTests
{
    [Fact]
    public void HelpHtml_IsEmbeddedAsResource()
    {
        var asm = typeof(CatalogWin.HelpWindow).Assembly;
        // Manifest name follows AssemblyName (JocalaCatalog), not the folder.
        using var g = asm.GetManifestResourceStream(asm.GetName().Name + ".g.resources");
        Assert.NotNull(g);
        using var reader = new ResourceReader(g!);
        string? content = null;
        foreach (System.Collections.DictionaryEntry entry in reader)
        {
            if (!string.Equals((string)entry.Key, "assets/help.html", StringComparison.OrdinalIgnoreCase))
                continue;
            content = entry.Value switch
            {
                Stream s => new StreamReader(s).ReadToEnd(),
                string str => str,
                byte[] bytes => Encoding.UTF8.GetString(bytes),
                _ => null
            };
        }
        Assert.False(string.IsNullOrWhiteSpace(content));
        Assert.Contains("catalog:close", content!);
    }
}
