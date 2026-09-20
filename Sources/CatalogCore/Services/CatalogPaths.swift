import Foundation

/// Catalog data root — ~/Library/Application Support/com.jocala.Catalog/
/// (forked from macos JReaderPaths ~/.jreader/; catalog product identity
/// keeps its own platform dir, never shares the Reader app's store).
public enum CatalogPaths {
    public static var base: URL {
        let fm = FileManager.default
        let root = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
            .appendingPathComponent("com.jocala.Catalog", isDirectory: true)
        try? fm.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }
    public static var coversDir: URL {
        let u = base.appendingPathComponent("covers", isDirectory: true)
        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
        return u
    }
    public static var thumbnailsDir: URL {
        let u = base.appendingPathComponent("thumbnails", isDirectory: true)
        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
        return u
    }
    public static var logFile: URL { base.appendingPathComponent("app_log.txt") }
    public static var oldLogFile: URL { base.appendingPathComponent("app_log_old.txt") }
}
