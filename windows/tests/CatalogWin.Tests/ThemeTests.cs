// Pure-mapping tests for UI/Theme.MapPreference (no WPF, no STA needed).

using System.Windows;
using CatalogWin.UI;
using Xunit;

namespace CatalogWin.Tests;

public sealed class ThemeTests
{
    [Fact]
    public void Light_MapsToLight()
    {
        Assert.Equal(ThemeMode.Light, Theme.MapPreference(1));
    }

    [Fact]
    public void Dark_MapsToDark()
    {
        Assert.Equal(ThemeMode.Dark, Theme.MapPreference(2));
    }

    [Fact]
    public void System_MapsToSystem()
    {
        Assert.Equal(ThemeMode.System, Theme.MapPreference(0));
    }

    [Fact]
    public void UnknownPreference_FallsBackToSystem()
    {
        Assert.Equal(ThemeMode.System, Theme.MapPreference(99));
    }
}
