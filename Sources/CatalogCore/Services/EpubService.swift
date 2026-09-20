import Foundation

struct EpubMetadata {
    let title: String
    let author: String?
    let series: String?
    let seriesIndex: Float?
    let isbn: String?
    let publisher: String?
    let tags: [String]
    let comment: String?
}

struct SpineItem {
    let idref: String
    let index: Int
}

struct EpubService {

    static func parseMetadata(from url: URL) -> EpubMetadata? {
        guard let archive = Archive(url: url, accessMode: .read) else { return nil }
        guard let opfPath = findOpfPath(in: archive) else { return nil }
        let opfData = readEntry(opfPath, from: archive) ?? Data()
        return parseOpfMetadata(opfData)
    }

    static func parseChapters(from url: URL) -> [Chapter] {
        guard let archive = Archive(url: url, accessMode: .read) else { return [] }
        guard let opfPath = findOpfPath(in: archive) else { return [] }
        let opfBase = (opfPath as NSString).deletingLastPathComponent
        guard let opfData = readEntry(opfPath, from: archive) else { return [] }

        let spine = parseSpine(opfData)
        let manifest = parseManifest(opfData)

        let tocId = findTocId(opfData)
        if let tocId = tocId, let tocHref = manifest[tocId] {
            let tocPath = opfBase.isEmpty ? tocHref : "\(opfBase)/\(tocHref)"
            if let tocData = readEntry(tocPath, from: archive) {
                let ncxChapters = parseNcx(String(data: tocData, encoding: .utf8) ?? "")
                if !ncxChapters.isEmpty {
                    return ncxChapters
                }
            }
        }

        if let navHref = findNavHref(opfData), let navPath = manifest[navHref] {
            let fullNavPath = opfBase.isEmpty ? navPath : "\(opfBase)/\(navPath)"
            if let navData = readEntry(fullNavPath, from: archive) {
                let navChapters = parseNavXhtml(String(data: navData, encoding: .utf8) ?? "")
                if !navChapters.isEmpty {
                    return navChapters
                }
            }
        }

        return spine.enumerated().map { i, item in
            let href = manifest[item.idref] ?? "\(item.idref).xhtml"
            let title = (href as NSString).deletingPathExtension
                .replacingOccurrences(of: "_", with: " ")
                .capitalized
            return Chapter(index: i, title: title, anchor: item.idref)
        }
    }

    static func extractChapters(from url: URL) -> [URL] {
        guard let archive = Archive(url: url, accessMode: .read) else { return [] }
        guard let opfPath = findOpfPath(in: archive) else { return [] }
        let opfBase = (opfPath as NSString).deletingLastPathComponent
        guard let opfData = readEntry(opfPath, from: archive) else { return [] }

        let spine = parseSpine(opfData)
        let manifest = parseManifest(opfData)

        let cacheDir = cacheDirectory(for: url)
        try? FileManager.default.removeItem(at: cacheDir)
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)

        let htmlExts: Set<String> = ["xhtml", "html", "htm"]
        for (_, href) in manifest {
            let fullPath = opfBase.isEmpty ? href : "\(opfBase)/\(href)"
            guard let entry = archive[fullPath] else { continue }
            let dest = cacheDir.appendingPathComponent(fullPath)
            let dir = dest.deletingLastPathComponent()
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            if !FileManager.default.fileExists(atPath: dest.path) {
                var data = Data()
                try? archive.extract(entry, consumer: { data.append($0) })
                let ext = (href as NSString).pathExtension.lowercased()
                if htmlExts.contains(ext) {
                    if var str = String(data: data, encoding: .utf8) {
                        let cspPattern = "<meta[^>]*http-equiv\\s*=\\s*\"[Cc]ontent-[Ss]ecurity-[Pp]olicy\"[^>]*>"
                        if let regex = try? NSRegularExpression(pattern: cspPattern, options: [.caseInsensitive]) {
                            str = regex.stringByReplacingMatches(in: str, range: NSRange(str.startIndex..., in: str), withTemplate: "")
                        }
                        data = Data(str.utf8)
                    }
                }
                try? data.write(to: dest)
            }
        }

        return spine.compactMap { item in
            guard let href = manifest[item.idref] else { return nil }
            let fullPath = opfBase.isEmpty ? href : "\(opfBase)/\(href)"
            let dest = cacheDir.appendingPathComponent(fullPath)
            return FileManager.default.fileExists(atPath: dest.path) ? dest : nil
        }
    }

    static func coverImageURL(for bookURL: URL) -> URL? {
        guard let archive = Archive(url: bookURL, accessMode: .read) else { return nil }
        guard let opfPath = findOpfPath(in: archive) else { return nil }
        let opfBase = (opfPath as NSString).deletingLastPathComponent
        guard let opfData = readEntry(opfPath, from: archive) else { return nil }
        let opfXml = String(data: opfData, encoding: .utf8) ?? ""
        let manifest = parseManifest(opfData)

        var coverId: String?
        if let match = try? NSRegularExpression(pattern: "<meta[^>]*name=\"cover\"[^>]*content=\"([^\"]+)\"").firstMatch(
            in: opfXml, range: NSRange(opfXml.startIndex..., in: opfXml)),
           let range = Range(match.range(at: 1), in: opfXml) {
            coverId = String(opfXml[range])
        }
        if coverId == nil {
            if let match = try? NSRegularExpression(pattern: "<item[^>]*id=\"([^\"]+)\"[^>]*properties=\"cover-image\"").firstMatch(
                in: opfXml, range: NSRange(opfXml.startIndex..., in: opfXml)),
               let range = Range(match.range(at: 1), in: opfXml) {
                coverId = String(opfXml[range])
            }
        }
        if coverId == nil {
            // Fallback: find first image in manifest whose id or href contains "cover"
            for (id, href) in manifest {
                if id.lowercased().contains("cover") || href.lowercased().contains("cover") {
                    coverId = id
                    break
                }
            }
        }
        guard let cid = coverId, let href = manifest[cid] else { return nil }
        let fullPath = opfBase.isEmpty ? href : "\(opfBase)/\(href)"
        let imageURL = cacheDirectory(for: bookURL).appendingPathComponent(fullPath)
        if FileManager.default.fileExists(atPath: imageURL.path) {
            return imageURL
        }
        guard let entry = archive[fullPath] else { return nil }
        try? FileManager.default.createDirectory(at: imageURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        var data = Data(capacity: Int(entry.uncompressedSize))
        try? archive.extract(entry, consumer: { data.append($0) })
        try? data.write(to: imageURL)
        return imageURL
    }

    static func clearPageCache() {
        guard let cacheDir = try? FileManager.default.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: false) else { return }
        guard let contents = try? FileManager.default.contentsOfDirectory(at: cacheDir, includingPropertiesForKeys: nil) else { return }
        for url in contents where url.lastPathComponent.hasPrefix("epub_") {
            try? FileManager.default.removeItem(at: url)
        }
    }

    private static func findOpfPath(in archive: Archive) -> String? {
        guard let entry = archive["META-INF/container.xml"] else { return nil }
        var data = Data(capacity: Int(entry.uncompressedSize))
        try? archive.extract(entry, consumer: { data.append($0) })
        guard let xml = String(data: data, encoding: .utf8) else { return nil }
        guard let match = try? NSRegularExpression(pattern: "full-path=\"([^\"]+)\"").firstMatch(
            in: xml, range: NSRange(xml.startIndex..., in: xml)),
              let range = Range(match.range(at: 1), in: xml)
        else { return nil }
        return String(xml[range])
    }

    private static func readEntry(_ path: String, from archive: Archive) -> Data? {
        guard let entry = archive[path] else { return nil }
        var data = Data(capacity: Int(entry.uncompressedSize))
        try? archive.extract(entry, consumer: { data.append($0) })
        return data
    }

    private static func parseOpfMetadata(_ data: Data) -> EpubMetadata? {
        let xml = String(data: data, encoding: .utf8) ?? ""
        let title = extractTag(xml, tag: "dc:title") ?? extractTag(xml, tag: "title") ?? "Unknown"
        let author = extractTag(xml, tag: "dc:creator") ?? extractTag(xml, tag: "creator")
        let publisher = extractTag(xml, tag: "dc:publisher") ?? extractTag(xml, tag: "publisher")

        var series: String?
        var seriesIndex: Float?
        if let match = try? NSRegularExpression(pattern: "<meta[^>]*name=\"calibre:series\"[^>]*content=\"([^\"]+)\"").firstMatch(
            in: xml, range: NSRange(xml.startIndex..., in: xml)),
           let range = Range(match.range(at: 1), in: xml) {
            series = String(xml[range])
        }
        if let match = try? NSRegularExpression(pattern: "<meta[^>]*name=\"calibre:series_index\"[^>]*content=\"([^\"]+)\"").firstMatch(
            in: xml, range: NSRange(xml.startIndex..., in: xml)),
           let range = Range(match.range(at: 1), in: xml) {
            seriesIndex = Float(String(xml[range]))
        }

        // Tags (dc:subject)
        var tags: [String] = []
        if let matches = try? NSRegularExpression(pattern: "<dc:subject[^>]*>([^<]+)</dc:subject>", options: [.caseInsensitive]).matches(
            in: xml, range: NSRange(xml.startIndex..., in: xml)) {
            for match in matches {
                if let range = Range(match.range(at: 1), in: xml) {
                    let subject = String(xml[range]).trimmingCharacters(in: .whitespaces)
                    if !subject.isEmpty {
                        // Some EPUBs separate tags with semicolons or commas
                        for part in subject.components(separatedBy: CharacterSet(charactersIn: ";,|")) {
                            let trimmed = part.trimmingCharacters(in: .whitespaces)
                            if !trimmed.isEmpty { tags.append(trimmed) }
                        }
                    }
                }
            }
        }

        // Description (dc:description) — strip HTML tags
        let comment: String? = (extractTag(xml, tag: "dc:description") ?? extractTag(xml, tag: "description"))
            .map { stripHTML($0) }

        var isbn: String?
        let isbnPatterns = [
            "<dc:identifier[^>]*>[^<]*(?:urn:)?isbn:([^<]+)</dc:identifier>",
            "<dc:identifier[^>]*id=\"[^\"]*isbn[^\"]*\"[^>]*>([^<]+)</dc:identifier>",
        ]
        for pattern in isbnPatterns {
            if let match = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]).firstMatch(
                in: xml, range: NSRange(xml.startIndex..., in: xml)),
               let range = Range(match.range(at: 1), in: xml) {
                isbn = String(xml[range]).trimmingCharacters(in: .whitespaces)
                break
            }
        }

        return EpubMetadata(title: title, author: author, series: series, seriesIndex: seriesIndex, isbn: isbn, publisher: publisher, tags: tags, comment: comment)
    }

    private static func extractTag(_ xml: String, tag: String) -> String? {
        guard let match = try? NSRegularExpression(pattern: "<\(tag)[^>]*>([^<]+)</\(tag)>", options: [.caseInsensitive]).firstMatch(
            in: xml, range: NSRange(xml.startIndex..., in: xml)),
              let range = Range(match.range(at: 1), in: xml)
        else { return nil }
        return String(xml[range]).trimmingCharacters(in: .whitespaces)
    }

    private static func stripHTML(_ html: String) -> String {
        guard let data = html.data(using: .utf8),
              let plain = try? NSAttributedString(
                data: data,
                options: [.documentType: NSAttributedString.DocumentType.html],
                documentAttributes: nil
              ).string else { return html }
        return plain.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func parseSpine(_ data: Data) -> [SpineItem] {
        let xml = String(data: data, encoding: .utf8) ?? ""
        var items: [SpineItem] = []
        guard let matches = try? NSRegularExpression(pattern: "<itemref[^>]*idref=\"([^\"]+)\"").matches(
            in: xml, range: NSRange(xml.startIndex..., in: xml)) else { return [] }
        for (i, match) in matches.enumerated() {
            guard let range = Range(match.range(at: 1), in: xml) else { continue }
            items.append(SpineItem(idref: String(xml[range]), index: i))
        }
        return items
    }

    private static func parseManifest(_ data: Data) -> [String: String] {
        let xml = String(data: data, encoding: .utf8) ?? ""
        var map: [String: String] = [:]
        let patterns = [
            "<item[^>]*id=\"([^\"]+)\"[^>]*href=\"([^\"]+)\"",
            "<item[^>]*href=\"([^\"]+)\"[^>]*id=\"([^\"]+)\""
        ]
        for (i, pattern) in patterns.enumerated() {
            guard let matches = try? NSRegularExpression(pattern: pattern).matches(
                in: xml, range: NSRange(xml.startIndex..., in: xml)) else { continue }
            for match in matches {
                guard let r1 = Range(match.range(at: 1), in: xml),
                      let r2 = Range(match.range(at: 2), in: xml) else { continue }
                let first = String(xml[r1])
                let second = String(xml[r2])
                if i == 0 {
                    // id then href
                    if map[first] == nil { map[first] = second }
                } else {
                    // href then id — swap
                    if map[second] == nil { map[second] = first }
                }
            }
        }
        return map
    }

    private static func findTocId(_ data: Data) -> String? {
        let xml = String(data: data, encoding: .utf8) ?? ""
        if let match = try? NSRegularExpression(pattern: "<spine[^>]*toc=\"([^\"]+)\"").firstMatch(
            in: xml, range: NSRange(xml.startIndex..., in: xml)),
           let range = Range(match.range(at: 1), in: xml) {
            return String(xml[range])
        }
        return nil
    }

    private static func findNavHref(_ data: Data) -> String? {
        let xml = String(data: data, encoding: .utf8) ?? ""
        let patterns = [
            "<item[^>]*id=\"toc\"[^>]*href=\"([^\"]+)\"",
            "<item[^>]*id=\"nav\"[^>]*href=\"([^\"]+)\"",
            "<item[^>]*properties=\"nav\"[^>]*href=\"([^\"]+)\"",
        ]
        for pattern in patterns {
            if let match = try? NSRegularExpression(pattern: pattern).firstMatch(
                in: xml, range: NSRange(xml.startIndex..., in: xml)),
               let range = Range(match.range(at: 1), in: xml) {
                return String(xml[range])
            }
        }
        return nil
    }

    private static func parseNcx(_ xml: String) -> [Chapter] {
        var chapters: [Chapter] = []
        guard let matches = try? NSRegularExpression(
            pattern: "<navPoint[^>]*>.*?<navLabel>.*?<text>([^<]+)</text>.*?</navLabel>.*?<content[^>]*src=\"([^\"]+)\"",
            options: [.dotMatchesLineSeparators]
        ).matches(in: xml, range: NSRange(xml.startIndex..., in: xml)) else { return [] }
        for (i, match) in matches.enumerated() {
            guard let titleRange = Range(match.range(at: 1), in: xml),
                  let srcRange = Range(match.range(at: 2), in: xml) else { continue }
            let title = String(xml[titleRange])
            let anchor = String(xml[srcRange])
            chapters.append(Chapter(index: i, title: title, anchor: anchor))
        }
        return chapters
    }

    private static func parseNavXhtml(_ xml: String) -> [Chapter] {
        var chapters: [Chapter] = []
        guard let matches = try? NSRegularExpression(
            pattern: "<a[^>]*href=\"([^\"]+)\"[^>]*>([^<]+)</a>",
            options: [.dotMatchesLineSeparators]
        ).matches(in: xml, range: NSRange(xml.startIndex..., in: xml)) else { return [] }
        for (i, match) in matches.enumerated() {
            guard let hrefRange = Range(match.range(at: 1), in: xml),
                  let textRange = Range(match.range(at: 2), in: xml) else { continue }
            let anchor = String(xml[hrefRange])
            let text = String(xml[textRange]).trimmingCharacters(in: .whitespacesAndNewlines)
            chapters.append(Chapter(index: i, title: text, anchor: anchor))
        }
        return chapters
    }

    static func cacheDirectory(for url: URL) -> URL {
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        let hash = abs(url.path.hashValue)
        return cacheDir.appendingPathComponent("epub_\(hash)")
    }
}
