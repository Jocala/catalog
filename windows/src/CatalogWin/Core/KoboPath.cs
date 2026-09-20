// Port of macos/Sources/ReaderCatalogGUI/KoboPath.swift (178 lines).
// Replicates calibre/devices/kobo/driver.py create_upload_path.
// Template: "{author_sort}/{title} - {authors}", MAX_PATH_LEN=185, kepubify=.kepub.epub
//
// Length semantics: Swift uses s.utf16.count * 2 (bytes). C# string.Length IS
// the UTF-16 code-unit count, so EncodedLength(s) = s.Length * 2 is identical
// for all BMP text (book paths are diacritic-folded first, so this holds).
// tst_kobopath vectors are the byte-exactness gate — if they fail, Kobo opens break.

using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;

namespace CatalogWin.Core;

public static partial class KoboPath
{
    public const int MaxPathLen = 185;
    public const string Prefix = "/mnt/onboard";

    // calibre's _filename_sanitize_unicode set: control 0x00-0x1F + / : * ? " < > | \ +
    private static readonly HashSet<char> SanitizeSet = BuildSanitizeSet();

    private static HashSet<char> BuildSanitizeSet()
    {
        var s = new HashSet<char>();
        for (int i = 0; i <= 0x1F; i++) s.Add((char)i);
        foreach (char c in new[] { '/', ':', '*', '?', '"', '<', '>', '|', '\\', '+' })
            s.Add(c);
        return s;
    }

    // calibre title_sort with library_order: move leading A/An/The to end with comma.
    public static string TitleSort(string title)
    {
        string t = title.Trim();
        foreach (string prefix in new[] { "A ", "An ", "The " })
        {
            if (t.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
            {
                string rest = t[prefix.Length..].Trim();
                return rest + ", " + prefix.Trim();
            }
        }
        return t;
    }

    // calibre author_to_author_sort with method='comma' (default).
    public static string AuthorSort(string natural)
    {
        string a = natural.Trim();
        if (a.Length == 0) return "";
        if (a.Contains(',')) return a; // already "Last, First"
        string[] tokens = a.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
        if (tokens.Length < 2) return a;
        string last = tokens[^1];
        string first = string.Join(" ", tokens[..^1]);
        return $"{last}, {first}";
    }

    // Inverse of AuthorSort: "Bruen, Ken" -> "Ken Bruen".
    public static string NaturalName(string sort)
    {
        string[] parts = sort.Split(',', 2).Select(p => p.Trim()).ToArray();
        if (parts.Length == 2) return $"{parts[1]} {parts[0]}";
        return sort;
    }

    // calibre sanitize_file_name(name, substitute='_') + Kobo ascii handling.
    // Mirror observed: "Carré, John le" -> "Carre, John le", "Smiley's" -> "Smiley_s".
    public static string Sanitize(string name)
    {
        // Kobo filesystem is FAT, calibre uses ascii for Kobo: fold diacritics, '_' for "'".
        string ascii = StripDiacritics(name);
        ascii = ascii.Replace("'", "_");
        string one = new string(ascii.Select(c => SanitizeSet.Contains(c) ? '_' : c).ToArray());
        // re.sub(r'\s',' ', one).strip() — collapse whitespace to space.
        one = WhitespaceRegex().Replace(one, " ").Trim();
        // split ext (name has no directory separators at this point).
        string ext = Path.GetExtension(one);
        string bname = ext.Length > 0 ? one[..^ext.Length] : one;
        // re.sub(r'^\.+$','_',bname).
        if (bname.Length > 0 && bname.All(c => c == '.')) bname = "_";
        string result = bname.Replace("..", "_");
        if (ext.Length > 0) result += ext;
        // if last char in '. ' -> '_' and leading '.' -> '_'.
        if (result.Length > 0 && (result[^1] == '.' || result[^1] == ' '))
            result = result[..^1] + "_";
        if (result.StartsWith('.'))
            result = "_" + result[1..];
        return result;
    }

    // calibre shorten_component(s, by_what): keep start+end.
    private static string ShortenComponent(string s, int byWhat)
    {
        int l = s.Length;
        if (l <= byWhat) return s;
        int keep = (l - byWhat) / 2;
        if (keep <= 0) return s[..Math.Max(1, l - byWhat)];
        return s[..keep] + s[^keep..];
    }

    // filename_encoding_for_length = 'utf-16' on macOS.
    private static int EncodedLength(string s) => s.Length * 2;

    public static string[] ShortenComponentsTo(int length, string[] components)
    {
        string[] comps = (string[])components.Clone();
        // First limit each component individually.
        for (int i = 0; i < comps.Length; i++)
        {
            string c = comps[i];
            while (EncodedLength(c) > length && c.Length > 2)
            {
                int delta = EncodedLength(c) - length;
                int byChars = Math.Max(2, (delta + 1) / 2);
                c = ShortenComponent(c, byChars);
            }
            comps[i] = c;
        }
        // Then distribute extra proportionally if joined still too long.
        string joined = string.Join("/", comps);
        int extra = EncodedLength(joined) - length;
        if (extra <= 0) return comps;
        int totalLen = comps.Sum(c => EncodedLength(c));
        var result = new List<string>();
        for (int idx = 0; idx < comps.Length; idx++)
        {
            string comp = comps[idx];
            if (idx == comps.Length - 1 && extra > 0)
            {
                double pct = (double)EncodedLength(comp) / totalLen;
                int delta = (int)Math.Ceiling(pct * extra);
                int byChars = Math.Max(2, (delta + 1) / 2);
                string c = comp;
                string ext = Path.GetExtension(c);
                string bname = ext.Length > 0 ? c[..^ext.Length] : c;
                if (ext.Length > 0)
                {
                    bname = ShortenComponent(bname, byChars);
                    c = bname + ext;
                }
                else
                {
                    c = ShortenComponent(c, byChars);
                }
                result.Add(c);
                extra = 0;
            }
            else
            {
                result.Add(comp);
            }
        }
        return result.ToArray();
    }

    // Predicted Kobo path for a book (mirrors calibre send).
    public static string PredictedPath(string title, string authorSort, string? authorsNatural = null)
    {
        string aSort = Sanitize(authorSort);
        string tSort = Sanitize(TitleSort(title));
        string authors = Sanitize(authorsNatural ?? NaturalName(authorSort));
        string file = $"{tSort} - {authors}.kepub.epub";
        string sanitizedFile = Sanitize(file); // already sanitized parts, but ensure
        string[] comps = new[] { aSort, sanitizedFile };
        // Shorten to MAX_PATH_LEN - prefix - 1.
        int prefixLen = EncodedLength(Prefix);
        int maxCompsLen = MaxPathLen * 2 - prefixLen - 2; // bytes, account for "/"
        int compsBytesLen = EncodedLength(string.Join("/", comps));
        if (compsBytesLen > maxCompsLen)
        {
            comps = ShortenComponentsTo(maxCompsLen / 2, comps);
            // Fallback simple: if still too long, truncate titleSort.
            string full = Prefix + "/" + string.Join("/", comps);
            if (EncodedLength(full) > MaxPathLen * 2)
            {
                int over = EncodedLength(full) - MaxPathLen * 2;
                int byChars = Math.Max(10, over / 2);
                string shortTitle = ShortenComponent(tSort, byChars);
                string shortFile = Sanitize($"{shortTitle} - {authors}.kepub.epub");
                comps = new[] { aSort, shortFile };
            }
        }
        return Prefix + "/" + string.Join("/", comps);
    }

    // Diacritic-insensitive fold: String.folding(.diacriticInsensitive) equivalent.
    private static string StripDiacritics(string s)
    {
        string decomposed = s.Normalize(NormalizationForm.FormD);
        var sb = new StringBuilder(decomposed.Length);
        foreach (char c in decomposed)
        {
            if (CharUnicodeInfo.GetUnicodeCategory(c) != UnicodeCategory.NonSpacingMark)
                sb.Append(c);
        }
        return sb.ToString().Normalize(NormalizationForm.FormC);
    }

    [GeneratedRegex(@"\s+")]
    private static partial Regex WhitespaceRegex();
}
