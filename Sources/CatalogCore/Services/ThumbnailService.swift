import Foundation
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif
import PDFKit
import CryptoKit

#if os(macOS)
public typealias PlatformImage = NSImage
#else
public typealias PlatformImage = UIImage
#endif


public enum ThumbnailService {
    static let thumbnailSize = CGSize(width: 120, height: 180)

    private static var cacheDir: URL {
        #if os(macOS)
        return CatalogPaths.thumbnailsDir
        #else
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("thumbnails", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
        #endif
    }

    static func cachedURL(for comicURL: URL) -> URL {
        let key = stableKey(comicURL.path)
        return cacheDir.appendingPathComponent("\(key).png")
    }

    private static func stableKey(_ path: String) -> String {
        var hash: UInt = 5381
        for byte in path.utf8 {
            hash = ((hash << 5) &+ hash) &+ UInt(byte)
        }
        let attrs = try? FileManager.default.attributesOfItem(atPath: path)
        if let size = attrs?[.size] as? UInt64 {
            hash = ((hash << 5) &+ hash) &+ UInt(size)
        }
        return "thumb_\(hash)"
    }

    /// Async thumbnail loader that handles all path types using content-addressed cover storage.
    public static func loadCoverJpg(from path: String) async -> PlatformImage? {
        return await Task.detached(priority: .utility) {
            let coverURL = URL(fileURLWithPath: path)
                .deletingLastPathComponent()
                .appendingPathComponent("cover.jpg")
            let bookExists = FileManager.default.fileExists(atPath: path)
            let coverExists = FileManager.default.fileExists(atPath: coverURL.path)
            ReaderLog.shared.d("CoverDiag", "loadCoverJpg bookPath='\(path)' bookExists=\(bookExists) coverPath='\(coverURL.path)' coverExists=\(coverExists)")
            guard let data = try? Data(contentsOf: coverURL) else {
                ReaderLog.shared.d("CoverDiag", "loadCoverJpg data=FAILED path='\(coverURL.path)'")
                return nil
            }
            #if os(macOS)
            guard let image = NSImage(data: data) else {
                ReaderLog.shared.d("CoverDiag", "loadCoverJpg image=DECODE_FAILED dataSize=\(data.count)")
                return nil
            }
            #else
            guard let image = UIImage(data: data) else {
                ReaderLog.shared.d("CoverDiag", "loadCoverJpg image=DECODE_FAILED dataSize=\(data.count)")
                return nil
            }
            #endif
            ReaderLog.shared.d("CoverDiag", "loadCoverJpg SUCCESS imageSize=\(image.size)")
            return image
        }.value
    }

    /// Async thumbnail loader that handles all path types using content-addressed cover storage.
    /// Expects a unified `covers/` cache directory. If `coverHash` is provided, it is used for
    /// cache lookup directly. Otherwise the cover is downloaded, hashed, and cached in `covers/`.
    public static func thumbnail(for path: String, host: String = "", share: String = "", coverHash: String? = nil) async -> PlatformImage? {
        #if os(macOS)
        let unifiedCovers = CatalogPaths.coversDir
        #else
        let unifiedCovers = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first!
            .appendingPathComponent("covers", isDirectory: true)
        try? FileManager.default.createDirectory(at: unifiedCovers, withIntermediateDirectories: true)
        #endif

        if let hash = coverHash {
            let hashURL = unifiedCovers.appendingPathComponent("\(hash).jpg")
            if FileManager.default.fileExists(atPath: hashURL.path) {
                #if os(macOS)
                if let image = NSImage(contentsOf: hashURL) {
                    let thumb = centerCropToThumbnail(image) ?? image
                    return thumb
                }
                #else
                if let image = UIImage(contentsOfFile: hashURL.path) {
                    let thumb = centerCropToThumbnail(image) ?? image
                    return thumb
                }
                #endif
            }
        }

        // tolerate "/https://..." from fileURL artifact or stored "/https://"
        let effectivePath: String = path.hasPrefix("/") && (path.hasPrefix("/http://") || path.hasPrefix("/https://") || path.hasPrefix("/smb://")) ? String(path.dropFirst()) : path
        if effectivePath.hasPrefix("smb://") || path.hasPrefix("smb://") {
            var lookupHost = host
            var lookupShare = share
            var remotePath = String(path.dropFirst(6))
            if lookupHost.isEmpty {
                let parts = remotePath.split(separator: "/")
                if parts.count >= 2 {
                    lookupHost = String(parts[0])
                    lookupShare = String(parts[1])
                    remotePath = parts.dropFirst(2).joined(separator: "/")
                }
            } else {
                let prefix = "\(lookupHost)/\(lookupShare)/"
                if remotePath.hasPrefix(prefix) {
                    remotePath = String(remotePath.dropFirst(prefix.count))
                }
            }
            let coverRemotePath: String
            // Calibre books.path is a FOLDER ("Author/Title (id)"); only a full
            // file path (ebook extension suffix) takes sibling cover.jpg.
            // Dot-presence is NOT a file test: folders like
            // "01-03.winter.black (9329)" or "St. Peter's Fair (699)" contain dots.
            let lower = remotePath.lowercased()
            if lower.hasSuffix(".epub") || lower.hasSuffix(".pdf") || lower.hasSuffix(".kepub") {
                if let range = remotePath.range(of: "/[^/]+$", options: .regularExpression) {
                    coverRemotePath = remotePath.replacingCharacters(in: range, with: "/cover.jpg")
                } else {
                    coverRemotePath = "\(remotePath)/cover.jpg"
                }
            } else {
                coverRemotePath = "\(remotePath)/cover.jpg"
            }
            let server = SmbServer.saved.first(where: { $0.host == lookupHost })
            let user = server?.user ?? ""
            let domain = server?.domain ?? ""
            let password = KeychainHelper.read(account: lookupHost) ?? ""
            do {
                let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).jpg")
                try await SmbService.downloadFile(
                    host: lookupHost, user: user, password: password,
                    share: lookupShare, domain: domain,
                    remotePath: coverRemotePath, to: tempURL,
                    progress: { _ in }
                )
                let data = try Data(contentsOf: tempURL)
                let hash = SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
                let cachedURL = unifiedCovers.appendingPathComponent("\(hash).jpg")
                if !FileManager.default.fileExists(atPath: cachedURL.path) {
                    try data.write(to: cachedURL)
                }
                try? FileManager.default.removeItem(at: tempURL)
                #if os(macOS)
                if let image = NSImage(data: data) {
                    let thumb = centerCropToThumbnail(image) ?? image
                    try? data.write(to: cachedURL)
                    return thumb
                }
                #else
                if let image = UIImage(data: data) {
                    let thumb = centerCropToThumbnail(image) ?? image
                    if let pngData = thumb.pngData() {
                        try? pngData.write(to: cachedURL)
                    }
                    return thumb
                }
                #endif
            } catch {
                ReaderLog.shared.e("ThumbnailService", "smb cover fetch failed input='\(path)' remote='\(lookupShare)/\(coverRemotePath)' err=\(error.localizedDescription)")
            }
            return nil
        }
        let httpPath = effectivePath.hasPrefix("http://") || effectivePath.hasPrefix("https://") ? effectivePath : path
        if httpPath.hasPrefix("http://") || httpPath.hasPrefix("https://") {
            // Calibre paths contain raw spaces/quotes e.g. "Author/ 'Title' (123)/Book.epub" — URL(string:) fails without encoding; ' not in urlFragmentAllowed
            let baseEnc = httpPath.addingPercentEncoding(withAllowedCharacters: .urlFragmentAllowed) ?? httpPath
            let encoded = baseEnc.replacingOccurrences(of: "'", with: "%27").replacingOccurrences(of: "\"", with: "%22")
            guard let bookURL = URL(string: encoded) else {
                ReaderLog.shared.e("ThumbnailService", "thumbnail http URL(string:) nil for path='\(path)' encoded='\(encoded)'")
                return nil
            }
            let coverURL = bookURL.deletingLastPathComponent().appendingPathComponent("cover.jpg")
            ReaderLog.shared.i("ThumbnailService", "http cover fetch coverURL='\(coverURL)' from path='\(path)'")
            do {
                let request = URLRequest(url: coverURL, timeoutInterval: 15)
                let (data, response) = try await URLSession.shared.data(for: request)
                let code = (response as? HTTPURLResponse)?.statusCode ?? -1
                let ct = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Type") ?? "?"
                ReaderLog.shared.i("ThumbnailService", "http cover HTTP \(code) ct=\(ct) bytes=\(data.count) url='\(coverURL)'")
                guard code == 200 else { return nil }
                guard !data.isEmpty else { return nil }
                let hash = SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
                let cachedURL = unifiedCovers.appendingPathComponent("\(hash).jpg")
                if !FileManager.default.fileExists(atPath: cachedURL.path) {
                    try data.write(to: cachedURL)
                }
                #if os(macOS)
                guard let image = NSImage(data: data) else {
                    ReaderLog.shared.e("ThumbnailService", "http cover NSImage nil bytes=\(data.count) url='\(coverURL)'")
                    return nil
                }
                let thumb = centerCropToThumbnail(image) ?? image
                try? data.write(to: cachedURL)
                #else
                guard let image = UIImage(data: data) else {
                    ReaderLog.shared.e("ThumbnailService", "http cover UIImage nil bytes=\(data.count) url='\(coverURL)'")
                    return nil
                }
                let thumb = centerCropToThumbnail(image) ?? image
                if let pngData = thumb.pngData() {
                    try? pngData.write(to: cachedURL)
                }
                #endif
                ReaderLog.shared.i("ThumbnailService", "http cover SUCCESS bytes=\(data.count) url='\(coverURL)'")
                return thumb
            } catch {
                ReaderLog.shared.e("ThumbnailService", "http cover fetch error \(error.localizedDescription) url='\(coverURL)'")
                return nil
            }
        }
        return await Task.detached(priority: .utility) {
            return await thumbnail(for: URL(fileURLWithPath: path))
        }.value
    }

    /// Resolve an smb:// path to a local cached cover, downloading from SMB if needed.
    static func smbCoverURL(for smbPath: String, host: String, share: String) async -> URL? {
        guard smbPath.hasPrefix("smb://") else { return nil }
        let remotePath = String(smbPath.dropFirst(6))
        let coverRemotePath = remotePath
            .replacingOccurrences(of: "/[^/]+$", with: "/cover.jpg", options: .regularExpression)
        #if os(macOS)
        let cacheDir = CatalogPaths.coversDir.appendingPathComponent("smb_covers", isDirectory: true)
        #else
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first!
            .appendingPathComponent("smb_covers", isDirectory: true)
        #endif
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        let localURL = cacheDir.appendingPathComponent("\(abs(smbPath.hashValue)).jpg")
        if FileManager.default.fileExists(atPath: localURL.path) {
            return localURL
        }
        let server = SmbServer.saved.first(where: { $0.host == host })
        let user = server?.user ?? ""
        let domain = server?.domain ?? ""
        let password = KeychainHelper.read(account: host) ?? ""
        do {
            try await SmbService.downloadFile(
                host: host, user: user, password: password,
                share: share, domain: domain,
                remotePath: coverRemotePath, to: localURL,
                progress: { _ in }
            )
            return localURL
        } catch {
            return nil
        }
    }

    public static func thumbnail(for comicURL: URL, persist: Bool = true) async -> PlatformImage? {
        // Defensive: this overload is often called with URL(fileURLWithPath: "https://...") which mangles to "/https:/..." — recover to http handler
        let raw = comicURL.absoluteString
        if raw.contains("https://") || raw.contains("http://") || comicURL.path.contains("https://") || comicURL.path.contains("http://") {
            var str = comicURL.path
            // fileURLWithPath adds leading "/" and collapses "//" -> "/"
            if str.hasPrefix("/https://") || str.hasPrefix("/http://") { str = String(str.dropFirst()) }
            else if str.contains("https:/") && !str.contains("https://") { str = str.replacingOccurrences(of: "https:/", with: "https://") }
            else if str.contains("http:/") && !str.contains("http://") { str = str.replacingOccurrences(of: "http:/", with: "http://") }
            // also try absoluteString recovery
            if str.hasPrefix("/") { str = String(str.dropFirst()) }
            if str.hasPrefix("http") {
                ReaderLog.shared.i("ThumbnailService", "thumbnail URL overload recovered http str='\(str)' from comicURL='\(comicURL)'")
                if let img = await thumbnail(for: str, coverHash: nil) { return img }
            }
        }
        return await Task.detached(priority: .utility) {
            let cached = cachedURL(for: comicURL)
            #if os(macOS)
            if FileManager.default.fileExists(atPath: cached.path), let img = NSImage(contentsOf: cached) { return img }
            #else
            if FileManager.default.fileExists(atPath: cached.path) {
                return UIImage(contentsOfFile: cached.path)
            }
            #endif
            // Calibre books.path is a FOLDER ("Author/Title (id)") for local
            // libraries — cover.jpg lives INSIDE it. deletingLastPathComponent
            // is only correct for file paths (would look one level too high).
            var isDir: ObjCBool = false
            let isFolder = FileManager.default.fileExists(atPath: comicURL.path, isDirectory: &isDir) && isDir.boolValue
            let coverJpg = isFolder
                ? comicURL.appendingPathComponent("cover.jpg")
                : comicURL.deletingLastPathComponent().appendingPathComponent("cover.jpg")
            #if os(macOS)
            if FileManager.default.fileExists(atPath: coverJpg.path),
               let data = try? Data(contentsOf: coverJpg),
               let image = NSImage(data: data) {
                let thumb = centerCropToThumbnail(image) ?? image
                if persist, let tiff = thumb.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
                    try? png.write(to: cached)
                }
                return thumb
            }
            #else
            if FileManager.default.fileExists(atPath: coverJpg.path),
               let data = try? Data(contentsOf: coverJpg),
               let image = UIImage(data: data) {
                let thumb = centerCropToThumbnail(image) ?? image
                if persist, let pngData = thumb.pngData() {
                    try? pngData.write(to: cached)
                }
                return thumb
            }
            #endif
            if isFolder {
                // No cover.jpg — extract embedded cover from the first EPUB
                // inside the folder (mirrors Qt local branch). Never run
                // extractFirstPage on the folder itself (format="" noise).
                if let files = try? FileManager.default.contentsOfDirectory(at: comicURL, includingPropertiesForKeys: nil, options: .skipsHiddenFiles),
                   let epub = files.first(where: { $0.pathExtension.lowercased() == "epub" }),
                   let image = extractFirstPage(from: epub) {
                    let thumb = centerCropToThumbnail(image) ?? image
                    #if os(macOS)
                    if persist, let tiff = thumb.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
                        try? png.write(to: cached)
                    }
                    #else
                    if persist, let data = thumb.pngData() {
                        try? data.write(to: cached)
                    }
                    #endif
                    return thumb
                }
                return nil
            }
            guard let image = extractFirstPage(from: comicURL) else {
                ReaderLog.shared.e("Thumbnail", "path=\(comicURL.path) error=ThumbnailService msg=extractFirstPage returned nil format=\(comicURL.pathExtension)")
                return nil
            }
            let thumb = centerCropToThumbnail(image) ?? image
            #if os(macOS)
            if persist, let tiff = thumb.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
                try? png.write(to: cached)
            }
            #else
            if persist, let data = thumb.pngData() {
                try? data.write(to: cached)
            }
            #endif
            return thumb
        }.value
    }

    /// Scale to fit INSIDE thumbnailSize, preserving aspect ratio — never crop.
    /// Views lay out with contentMode .fit (letterbox), so cropping here only
    /// clips cover edges. Images that already fit are returned as-is.
    private static func centerCropToThumbnail(_ image: PlatformImage) -> PlatformImage? {
        #if os(macOS)
        let w = image.size.width
        let h = image.size.height
        guard w > 0, h > 0 else { return image }
        let scale = min(thumbnailSize.width / w, thumbnailSize.height / h)
        guard scale < 1 else { return image }
        let newSize = NSSize(width: w * scale, height: h * scale)
        let thumb = NSImage(size: newSize)
        thumb.lockFocus()
        image.draw(in: NSRect(origin: .zero, size: newSize),
                   from: NSRect(origin: .zero, size: image.size),
                   operation: .copy, fraction: 1.0)
        thumb.unlockFocus()
        return thumb
        #else
        return image.preparingThumbnail(of: thumbnailSize) ?? image
        #endif
    }

    static func extractFirstPage(from url: URL) -> PlatformImage? {
        let fmt = url.pathExtension.lowercased()
        switch fmt {
        case "pdf": return extractFirstFromPDF(url)
        case "epub": return extractFirstFromEPUB(url)
        default:
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=DetectFormat msg=Unknown book format: \(fmt)")
            return nil
        }
    }

    private static func extractFirstFromPDF(_ url: URL) -> PlatformImage? {
        guard let document = PDFDocument(url: url) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=PDFDocument msg=PDFDocument.init returned nil")
            return nil
        }
        guard let page = document.page(at: 0) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=PDFDocument msg=document.page(at: 0) returned nil (pageCount was not checked)")
            return nil
        }
        let bounds = page.bounds(for: .mediaBox)
        let scale = min(thumbnailSize.width / bounds.width, thumbnailSize.height / bounds.height)
        let size = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        return page.thumbnail(of: size, for: .mediaBox)
    }

    /// Returns a full-size cover image for a book file (EPUB or PDF).
    /// Checks `cover.jpg` sibling first, then extracts from EPUB or renders first PDF page.
    static func coverImage(for url: URL) -> PlatformImage? {
        let coverJpg = url.deletingLastPathComponent().appendingPathComponent("cover.jpg")
        #if os(macOS)
        if FileManager.default.fileExists(atPath: coverJpg.path),
           let data = try? Data(contentsOf: coverJpg),
           let image = NSImage(data: data) {
            return image
        }
        #else
        if FileManager.default.fileExists(atPath: coverJpg.path),
           let data = try? Data(contentsOf: coverJpg),
           let image = UIImage(data: data) {
            return image
        }
        #endif
        switch url.pathExtension.lowercased() {
        case "epub":
            #if os(macOS)
            guard let coverURL = EpubService.coverImageURL(for: url),
                  let data = try? Data(contentsOf: coverURL),
                  let image = NSImage(data: data) else {
                return nil
            }
            return image
            #else
            guard let coverURL = EpubService.coverImageURL(for: url),
                  let data = try? Data(contentsOf: coverURL),
                  let image = UIImage(data: data) else {
                return nil
            }
            return image
            #endif
        case "pdf":
            guard let document = PDFDocument(url: url),
                  let page = document.page(at: 0) else {
                return nil
            }
            let bounds = page.bounds(for: .mediaBox)
            #if os(macOS)
            let screenSize = NSScreen.main?.frame.size ?? CGSize(width: 1440, height: 900)
            #else
            let screenSize = UIScreen.main.bounds.size
            #endif
            let scale = min(screenSize.width / bounds.width, screenSize.height / bounds.height)
            let size = CGSize(width: bounds.width * scale, height: bounds.height * scale)
            return page.thumbnail(of: size, for: .mediaBox)
        default:
            return nil
        }
    }

    private static func extractFirstFromEPUB(_ url: URL) -> PlatformImage? {
        guard let coverURL = EpubService.coverImageURL(for: url) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=EPUB msg=coverImageURL returned nil")
            return nil
        }
        guard let data = try? Data(contentsOf: coverURL) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=EPUB msg=could not read cover data from \(coverURL.path)")
            return nil
        }
        #if os(macOS)
        guard let image = NSImage(data: data) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=EPUB msg=NSImage(data:) failed (\(data.count) bytes)")
            return nil
        }
        #else
        guard let image = UIImage(data: data) else {
            ReaderLog.shared.e("Thumbnail", "path=\(url.path) error=EPUB msg=UIImage(data:) failed (\(data.count) bytes)")
            return nil
        }
        #endif
        return image
    }
}
