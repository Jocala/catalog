// Port of macos ReaderCatalogApp KoboLauncher (internal SSH, no script).
// Uses Settings Kobo IP. Test = ping; open = SSH heredoc to KOReader.
//
// SSH via SSH.NET ShellStream — fully internal to the app (no ssh.exe
// dependency, out-of-the-box). Two hard-won facts shape this code:
//   1. The Kobo's sshd swallows EXEC-channel requests (even OpenSSH one-shot
//      `ssh host cmd` returns empty; RunCommand hangs forever) — so this
//      uses a SHELL channel with heredoc content, like macOS sshSync.
//   2. Reads are deadline-polled, never blocking: ShellStream.Expect proved
//      fragile and RunCommand's default timeout is infinite. Every wait here
//      ends at timeoutSec+margin with exit 124, mirroring the old ssh.exe
//      transport it replaces (2026-09-19).

using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using Renci.SshNet;
using CatalogWin.Core;

namespace CatalogWin.Services;

public static class KoboLauncher
{
    public static async Task<string> OpenAsync(
        Core.CatalogBook book, Action<string>? onStatus = null, Action<string>? onError = null)
    {
        string query = $"{book.Author} {book.Title}";
        return await RunKoboAsync(query, book.Title, book.Author, onStatus, onError);
    }

    public static async Task<string> RunKoboAsync(
        string query, string title, string author,
        Action<string>? onStatus = null, Action<string>? onError = null)
    {
        onStatus?.Invoke($"Opening “{query}” on Kobo…");
        AppSettings settings = SettingsStore.Load();
        string koboIp = (settings.KoboIp ?? "").Trim();
        if (koboIp.Length == 0)
        {
            string msg = "Kobo IP not set — enter it in Settings → Kobo";
            onStatus?.Invoke(msg); onError?.Invoke(msg);
            return msg;
        }
        var (output, code) = await Task.Run(() => RunKoboSsh(query, title, author, koboIp));
        // Log FULL output (capped) — the 400-char dialog prefix hid the real
        // error last time (2026-09-19 SSH.NET failure). Dialog stays short.
        string logged = output.Trim();
        if (logged.Length > 2000) logged = logged[..2000];
        string logLine = logged;
        AppLog.Shared.Info("Kobo", $"open query='{query}' ip={koboIp} exit={code} out='{logLine}'");
        if (code == 0)
        {
            onStatus?.Invoke($"Opened on Kobo: {query}");
        }
        else
        {
            string err = output.Trim();
            string short_ = err.Length == 0 ? $"exit {code}" : err.Length > 400 ? err[..400] : err;
            onError?.Invoke(short_);
            onStatus?.Invoke($"Kobo failed ({code}): {logLine}");
        }
        return output;
    }

    private static (string Output, int Code) RunKoboSsh(string query, string title, string author, string ip)
    {
        // 1. Quick reachability: ssh echo ok with 5s timeout.
        var probe = SshSync(ip, "echo ok", 5);
        if (probe.Code != 0 || !probe.Output.ToLowerInvariant().Contains("ok"))
        {
            string hint = probe.Output.Length == 0
                ? $"Kobo is sleeping or unreachable at {ip} — press power button to wake, then try again."
                : probe.Output;
            string msg = hint.Contains("sleeping")
                ? hint
                : $"Kobo is sleeping or unreachable at {ip} — press power button to wake.\n{hint}";
            return (msg, probe.Code != 0 ? probe.Code : 7);
        }

        string titleTrim = title.Trim();
        string authorTrim = author.Trim();
        string lastName;
        if (authorTrim.Contains(','))
            lastName = authorTrim.Split(',', 2)[0].Trim();
        else
        {
            string[] toks = authorTrim.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            lastName = toks.Length > 0 ? toks[^1] : authorTrim;
        }
        string lastNameNorm = Norm(lastName);

        // 2. Primary: Calibre-predicted Kobo path.
        string predicted = Core.KoboPath.PredictedPath(
            title, authorTrim, Core.KoboPath.NaturalName(authorTrim));
        string escPred = predicted.Replace("'", "'\\''");
        var check = SshSync(ip, $"if [ -f '{escPred}' ]; then echo \"exists:{escPred}\"; else echo \"missing\"; fi", 6);
        List<string> filtered = new();
        // Response line starts with "exists:" — the echoed command line
        // contains `echo "exists:..."` mid-line, so Contains would false-positive.
        if (Regex.IsMatch(check.Output, "^exists:", RegexOptions.Multiline))
        {
            filtered.Add(predicted);
        }
        else
        {
            // 3. Fallback: strict title+author match via cached index or live find.
            List<string>? candidates = KoboIndex.CachedPaths();
            if (candidates is null)
            {
                var list = SshSync(ip,
                    "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort", 30);
                if (list.Code != 0)
                {
                    if (!(list.Output.Contains("/mnt/onboard/") && list.Output.Contains(".epub")))
                        return ($"Failed to list books on Kobo: {list.Output}", list.Code);
                }
                candidates = list.Output.Split('\n').Select(s => s.Trim())
                    // Absolute paths only — the shell echoes our command +
                    // prompt lines into the buffer; those never start with /.
                    .Where(s => s.StartsWith('/')).ToList();
                if (candidates.Count > 0) KoboIndex.Save(candidates);
            }
            if (candidates is null || candidates.Count == 0)
                return ("No books found on Kobo (find returned empty) — is /mnt/onboard mounted?", 1);

            string[] rawTokens = titleTrim.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            List<string> titleTokens = rawTokens.Select(Norm)
                .Where(t => t.Length > 0 && t != "a" && t != "an" && t != "the").ToList();
            List<string> required = titleTokens.Count > 0 ? titleTokens
                : new List<string> { Norm(titleTrim) }.Where(s => s.Length > 0).ToList();

            List<string> Strict(IEnumerable<string> list) => list.Where(p =>
            {
                string n = Norm(p);
                return required.All(n.Contains) &&
                       (lastNameNorm.Length == 0 || n.Contains(lastNameNorm));
            }).ToList();
            List<string> TitleOnly(IEnumerable<string> list) => list.Where(p =>
            {
                string n = Norm(p);
                return required.All(n.Contains);
            }).ToList();

            List<string> strict = Strict(candidates);
            List<string> titleOnly = strict.Count == 0 ? TitleOnly(candidates) : new();
            if (strict.Count == 0 && titleOnly.Count == 0)
            {
                // Refresh-on-miss: cached index may predate synced books. One
                // live find, overwrite cache, retry once.
                AppLog.Shared.Info("Kobo", $"0 matches on cached index ({candidates.Count} paths) — refreshing index and retrying once");
                var fresh = SshSync(ip,
                    "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort", 30);
                var freshPaths = fresh.Output.Split('\n').Select(s => s.Trim())
                    .Where(s => s.StartsWith('/')).ToList();
                if (freshPaths.Count > 0)
                {
                    KoboIndex.Save(freshPaths);
                    candidates = freshPaths;
                    AppLog.Shared.Info("Kobo", $"index refreshed ({candidates.Count} paths), retrying match");
                    strict = Strict(candidates);
                    titleOnly = strict.Count == 0 ? TitleOnly(candidates) : new();
                }
            }
            if (strict.Count == 0)
            {
                if (titleOnly.Count == 1)
                {
                    AppLog.Shared.Info("Kobo", $"strict title+author 0, title-only fallback 1 → {titleOnly[0]}");
                    filtered = titleOnly;
                }
                else if (titleOnly.Count == 0)
                {
                    return ($"No match for: \"{titleTrim}\" by {(lastName.Length == 0 ? authorTrim : lastName)} " +
                            $"(predicted: {predicted.Replace("/mnt/onboard/", "")} not found → strict {candidates.Count} books, 0 matched)\n" +
                            "Check Calibre Kobo template or re-sync Kobo.", 1);
                }
                else
                {
                    string preview = string.Join("\n", titleOnly.Take(10).Select(p => "  " + p.Replace("/mnt/onboard/", "")));
                    return ($"{titleOnly.Count} matches for \"{titleTrim}\" (title-only, author {lastName} not found) — ambiguous, not opening:\n" +
                            $"{preview}{(titleOnly.Count > 10 ? $"\n  ... +{titleOnly.Count - 10} more" : "")}\nRefine title or check series prefix.", 1);
                }
            }
            else if (strict.Count == 1)
            {
                filtered = strict;
            }
            else
            {
                string preview = string.Join("\n", strict.Take(10).Select(p => "  " + p.Replace("/mnt/onboard/", "")));
                return ($"{strict.Count} matches for \"{titleTrim}\" by {lastName} (strict) — ambiguous, not opening:\n" +
                        $"{preview}{(strict.Count > 10 ? $"\n  ... +{strict.Count - 10} more" : "")}", 1);
            }
        }

        if (filtered.Count != 1)
            return ($"Internal error: ambiguous match for \"{titleTrim}\"", 1);
        string chosen = filtered[0];
        // 4. Open via KOReader (OCP path first, fallback legacy).
        string esc = chosen.Replace("'", "'\\''").Replace("\"", "\\\"");
        string openCmd =
            "if [ -x /mnt/onboard/.adds/koreader/koreader.sh ]; then K=/mnt/onboard/.adds/koreader/koreader.sh; " +
            "else K=/mnt/onboard/koreader/koreader.sh; fi; " +
            $"if [ ! -f '{esc}' ]; then echo \"not found: {esc}\"; exit 1; fi; " +
            $"nohup \"$K\" '{esc}' >/tmp/koreader-open.log 2>&1 & sleep 1; " +
            "ps | grep -E \"koreader|reader.lua\" | head -n 5; " +
            $"echo \"launched: {esc}\"";
        var open = SshSync(ip, openCmd, 10);
        if (open.Code != 0) return ($"Failed to open on Kobo: {open.Output}", open.Code);
        return (open.Output, 0);
    }

    private static string Norm(string s)
    {
        // folding(.diacriticInsensitive).lowercased, keep letter|number.
        string decomposed = s.Normalize(NormalizationForm.FormD);
        var sb = new StringBuilder(decomposed.Length);
        foreach (char c in decomposed)
        {
            var cat = CharUnicodeInfo.GetUnicodeCategory(c);
            if (cat == UnicodeCategory.NonSpacingMark) continue;
            if (char.IsLetterOrDigit(c)) sb.Append(char.ToLowerInvariant(c));
        }
        return sb.ToString().Normalize(NormalizationForm.FormC);
    }

    /// One remote command over an SSH.NET shell channel with heredoc content.
    /// Deadline-polled reads: waits for the `MARK:$?` trailer or the
    /// timeout+margin deadline (exit 124 on overrun). Never blocks forever —
    /// no Expect, no exec channel, no pipes, no temp files.
    public static (string Output, int Code) SshSync(string ip, string remoteCmd, int timeoutSec)
    {
        string keyFile = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            ".ssh", "id_ed25519");
        if (!File.Exists(keyFile))
            return ($"ssh key not found: {keyFile}", -1);
        SshClient? client = null;
        Renci.SshNet.ShellStream? shell = null;
        try
        {
            client = new SshClient(new ConnectionInfo(ip, "root",
                new PrivateKeyAuthenticationMethod("root", new PrivateKeyFile(keyFile)))
            {
                Timeout = TimeSpan.FromSeconds(timeoutSec + 5),
            });
            // Trust-LAN host key policy (mirrors accept-new; Kobos live on LAN).
            client.HostKeyReceived += (_, e) => { e.CanTrust = true; };
            client.Connect();
            if (!client.IsConnected)
                return ("ssh connect failed (no exception)", -1);
            shell = client.CreateShellStream("kobo", 80, 24, 800, 600, 4096);
            string marker = $"KOBO_EXIT_{Guid.NewGuid():N}";
            shell.WriteLine(remoteCmd);
            shell.WriteLine($"echo {marker}:$?");

            // The Kobo shell ECHOES input (prompt + command lines), so the
            // marker text appears first inside our own echoed
            // `echo MARK:$?` line (followed by "$?", not digits). Only an
            // occurrence followed by digits is the real trailer — anything
            // else means keep polling.
            var sb = new StringBuilder();
            var buf = new byte[8192];
            DateTime deadline = DateTime.UtcNow.AddSeconds(timeoutSec + 10);
            while (DateTime.UtcNow < deadline)
            {
                while (shell.DataAvailable)
                {
                    int n = shell.Read(buf, 0, buf.Length);
                    if (n > 0)
                        sb.Append(Encoding.UTF8.GetString(buf, 0, n));
                    else
                        break;
                }
                string cur = sb.ToString().Replace("\r\n", "\n").Replace('\r', '\n');
                int searchFrom = 0;
                while (true)
                {
                    int mi = cur.IndexOf(marker + ":", searchFrom, StringComparison.Ordinal);
                    if (mi < 0) break;
                    string tail = cur[(mi + marker.Length + 1)..];
                    string digits = new string(tail.TakeWhile(c => c == '-' || char.IsDigit(c)).ToArray());
                    if (digits.Length > 0 && int.TryParse(digits, out int c))
                        return (cur[..mi].Trim(), c);
                    searchFrom = mi + 1; // echo occurrence — keep looking/polling
                }
                Thread.Sleep(100);
            }
            return (sb.ToString().Replace("\r\n", "\n").Replace('\r', '\n').Trim() + "\n(timed out)", 124);
        }
        catch (Exception ex)
        {
            return ($"ssh failed: {ex.GetType().Name}: {ex.Message}", -1);
        }
        finally
        {
            try { shell?.Dispose(); } catch { }
            try { client?.Disconnect(); } catch { }
            client?.Dispose();
        }
    }
}
