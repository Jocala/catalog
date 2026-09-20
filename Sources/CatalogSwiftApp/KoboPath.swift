import Foundation

// Replicates calibre/devices/kobo/driver.py create_upload_path for Kobo Clara Colour
// Template: "{author_sort}/{title} - {authors}"  MAX_PATH_LEN=185  kepubify=.kepub.epub
// See AGENTS.md and explore ses_f5491fda3ffe0

enum KoboPath {
    static let maxPathLen = 185
    static let prefix = "/mnt/onboard"
    // calibre's _filename_sanitize_unicode set: control 0x00-0x1F + / : * ? " < > | \ +
    private static let sanitizeSet: Set<Character> = {
        var s = Set<Character>()
        // 0x00-0x1F
        for i in 0...0x1F { if let scalar = UnicodeScalar(i) { s.insert(Character(scalar)) } }
        for c in ["/", ":", "*", "?", "\"", "<", ">", "|", "\\", "+"] { s.insert(Character(c)) }
        return s
    }()

    // calibre title_sort with library_order: move leading A/An/The to end with comma
    static func titleSort(_ title: String) -> String {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        // calibre strips surrounding quotes then checks ^(A|The|An)\s+ IGNORECASE;
        // quote-stripping omitted — no quoted leading articles in this library.
        let prefixes = ["A ", "An ", "The "]
        for prefix in prefixes {
            if t.lowercased().hasPrefix(prefix.lowercased()) {
                let rest = String(t.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces)
                return rest + ", " + prefix.trimmingCharacters(in: .whitespaces)
            }
        }
        return t
    }

    // calibre author_to_author_sort with method='comma' (default)
    // For our use, books.author_sort is already stored, but keep for completeness
    static func authorSort(from natural: String) -> String {
        let a = natural.trimmingCharacters(in: .whitespaces)
        if a.isEmpty { return "" }
        if a.contains(",") { return a } // already "Last, First"
        let tokens = a.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        if tokens.count < 2 { return a }
        // Simplified: last token is surname, rest is given
        // calibre handles von, Jr., etc. — not needed for 6742 (covers Bruen, Ken etc.)
        let last = tokens.last!
        let first = tokens.dropLast().joined(separator: " ")
        return "\(last), \(first)"
    }

    static func naturalName(fromSort sort: String) -> String {
        // Inverse of authorSort: "Bruen, Ken" -> "Ken Bruen"
        let parts = sort.split(separator: ",", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        if parts.count == 2 { return "\(parts[1]) \(parts[0])" }
        return sort
    }

    // calibre sanitize_file_name(name, substitute='_') + Kobo ascii handling (Carre, Smiley_s)
    static func sanitize(_ name: String) -> String {
        // Kobo filesystem is FAT, calibre uses ascii for Kobo: fold diacritics (é->e) and '_' for "'"
        // Mirror observed: "Carré, John le" -> "Carre, John le", "Smiley's" -> "Smiley_s", "Hell's" -> "Hell_s"
        var ascii = name.folding(options: .diacriticInsensitive, locale: .current)
        ascii = ascii.replacingOccurrences(of: "'", with: "_") // calibre replaces ' with _ for Kobo FAT
        var one = String(ascii.map { sanitizeSet.contains($0) ? "_" : $0 })
        // re.sub(r'\s',' ', one).strip() — collapse whitespace to space
        one = one.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression).trimmingCharacters(in: .whitespaces)
        // split ext
        let ns = one as NSString
        let ext = ns.pathExtension
        var bname = ns.deletingPathExtension
        // re.sub(r'^\.+$','_',bname)
        if !bname.isEmpty && bname.allSatisfy({ $0 == "." }) { bname = "_" }
        var result = bname.replacingOccurrences(of: "..", with: "_")
        if !ext.isEmpty { result += "." + ext }
        // if last char in '. ' -> '_' and leading '.' -> '_'
        if let last = result.last, last == "." || last == " " {
            result = String(result.dropLast()) + "_"
        }
        if result.hasPrefix(".") {
            result = "_" + result.dropFirst()
        }
        return result
    }

    // calibre shorten_component(s, by_what): keep start+end
    private static func shortenComponent(_ s: String, by byWhat: Int) -> String {
        let l = s.count
        if l <= byWhat { return s }
        let keep = (l - byWhat) / 2
        if keep <= 0 { return String(s.prefix(max(1, l - byWhat))) }
        return String(s.prefix(keep)) + String(s.suffix(keep))
    }

    // calibre limit_component / shorten_components_to
    // filename_encoding_for_length = 'utf-16' on macOS
    private static func encodedLength(_ s: String) -> Int {
        // macOS uses utf-16 for MAX_PATH_LEN check
        return s.utf16.count * 2 // utf-16 2 bytes per code unit
    }

    static func shortenComponentsTo(length: Int, components: [String]) -> [String] {
        var comps = components
        // First limit each component individually
        for i in comps.indices {
            var c = comps[i]
            while encodedLength(c) > length && c.count > 2 {
                let delta = encodedLength(c) - length
                // delta in bytes, convert to chars approx delta/2
                let byChars = max(2, (delta + 1) / 2)
                c = shortenComponent(c, by: byChars)
            }
            comps[i] = c
        }
        // Then distribute extra proportionally if joined still too long
        let joined = comps.joined(separator: "/")
        let extra = encodedLength(joined) - length
        if extra <= 0 { return comps }
        let totalLen = comps.map { encodedLength($0) }.reduce(0, +)
        var result: [String] = []
        var remainingExtra = extra
        for (idx, comp) in comps.enumerated() {
            if idx == comps.count - 1 && remainingExtra > 0 {
                // last component: shorten more
                let pct = Double(encodedLength(comp)) / Double(totalLen)
                let delta = Int(ceil(pct * Double(extra)))
                let byChars = max(2, (delta + 1) / 2)
                var c = comp
                // handle extension separately
                let ns = c as NSString
                let ext = ns.pathExtension
                var bname = ns.deletingPathExtension
                if !ext.isEmpty {
                    bname = shortenComponent(bname, by: byChars)
                    c = bname + "." + ext
                } else {
                    c = shortenComponent(c, by: byChars)
                }
                result.append(c)
                remainingExtra = 0
            } else {
                result.append(comp)
            }
        }
        return result
    }

    // Predicted Kobo path for a CatalogBook (mirrors calibre send)
    static func predictedPath(title: String, authorSort: String, authorsNatural: String? = nil) -> String {
        let aSort = sanitize(authorSort)
        let tSort = sanitize(titleSort(title))
        let authors = sanitize(authorsNatural ?? naturalName(fromSort: authorSort))
        let file = "\(tSort) - \(authors).kepub.epub"
        let sanitizedFile = sanitize(file) // already sanitized parts, but ensure
        var comps = [aSort, sanitizedFile]
        // Shorten to MAX_PATH_LEN - prefix -1
        let prefixLen = encodedLength(prefix) // "/mnt/onboard" utf-16
        let maxCompsLen = maxPathLen * 2 - prefixLen - 2 // bytes, account for "/"
        // Convert maxCompsLen bytes to char approx for our limit check (use encodedLength)
        // Use shortenComponentsTo with length in bytes
        let compsBytesLen = encodedLength(comps.joined(separator: "/"))
        if compsBytesLen > maxCompsLen {
            comps = shortenComponentsTo(length: maxCompsLen / 2, components: comps) // /2 because we compare char counts
            // Fallback simple: if still too long, truncate titleSort
            let full = prefix + "/" + comps.joined(separator: "/")
            if encodedLength(full) > maxPathLen * 2 {
                let over = encodedLength(full) - maxPathLen * 2
                let byChars = max(10, over / 2)
                let shortTitle = shortenComponent(tSort, by: byChars)
                let shortFile = sanitize("\(shortTitle) - \(authors).kepub.epub")
                comps = [aSort, shortFile]
            }
        }
        return prefix + "/" + comps.joined(separator: "/")
    }

    static func predictedPath(for book: CatalogBook) -> String {
        let authorsNatural = naturalName(fromSort: book.author)
        return predictedPath(title: book.title, authorSort: book.author, authorsNatural: authorsNatural)
    }
}
