// Kobo per-IP SSH password parity (macOS KoboDevice): AppSettings holds
// ip->password pairs next to the SMB passwords; empty means key-only.

using System.Text.Json;
using CatalogWin.Services;
using Xunit;

namespace CatalogWin.Tests;

public sealed class KoboPasswordTests
{
    [Fact]
    public void UnknownIp_ReturnsNull()
    {
        var s = new AppSettings();
        Assert.Null(s.KoboPasswordFor("192.168.1.74"));
    }

    [Fact]
    public void SetAndGet_RoundTrips()
    {
        var s = new AppSettings();
        s.SetKoboPassword("192.168.1.74", "1234");
        Assert.Equal("1234", s.KoboPasswordFor("192.168.1.74"));
    }

    [Fact]
    public void EmptyPassword_RemovesEntry_KeyOnly()
    {
        var s = new AppSettings();
        s.SetKoboPassword("192.168.1.74", "1234");
        s.SetKoboPassword("192.168.1.74", "");
        Assert.Null(s.KoboPasswordFor("192.168.1.74"));
        Assert.Empty(s.KoboPasswords);
    }

    [Fact]
    public void Ip_IsTrimmed()
    {
        var s = new AppSettings();
        s.SetKoboPassword("  192.168.1.74  ", "1234");
        Assert.Equal("1234", s.KoboPasswordFor("192.168.1.74"));
        s.SetKoboPassword("", "x"); // no-op, no empty-key entry
        Assert.Single(s.KoboPasswords);
    }

    [Fact]
    public void Json_RoundTripsKoboPasswords()
    {
        var s = new AppSettings();
        s.KoboIps.Add("192.168.1.74");
        s.SetKoboPassword("192.168.1.74", "1234");
        string json = JsonSerializer.Serialize(s);
        var back = JsonSerializer.Deserialize<AppSettings>(json);
        Assert.NotNull(back);
        Assert.Equal("1234", back!.KoboPasswordFor("192.168.1.74"));
        Assert.Equal(new List<string> { "192.168.1.74" }, back.KoboIps);
    }

    [Fact]
    public void Rename_CarriesPasswordAndDefault()
    {
        var s = new AppSettings();
        s.KoboIps.Add("192.168.1.74");
        s.KoboIp = "192.168.1.74";
        s.SetKoboPassword("192.168.1.74", "1234");
        s.RenameKobo("192.168.1.74", "192.168.1.75");
        Assert.Equal(new List<string> { "192.168.1.75" }, s.KoboIps);
        Assert.Equal("192.168.1.75", s.KoboIp);
        Assert.Equal("1234", s.KoboPasswordFor("192.168.1.75"));
        Assert.Null(s.KoboPasswordFor("192.168.1.74"));
    }

    [Fact]
    public void Rename_NonDefault_KeepsDefault()
    {
        var s = new AppSettings();
        s.KoboIps.Add("192.168.1.74");
        s.KoboIps.Add("192.168.1.75");
        s.KoboIp = "192.168.1.74";
        s.RenameKobo("192.168.1.75", "192.168.1.76");
        Assert.Equal("192.168.1.74", s.KoboIp);
    }

    [Fact]
    public void Rename_UnknownOrEmpty_IsNoOp()
    {
        var s = new AppSettings();
        s.KoboIps.Add("192.168.1.74");
        s.RenameKobo("10.0.0.9", "10.0.0.10");
        s.RenameKobo("192.168.1.74", "");
        s.RenameKobo("192.168.1.74", "192.168.1.74");
        Assert.Equal(new List<string> { "192.168.1.74" }, s.KoboIps);
    }
}
