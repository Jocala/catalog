import Foundation
import CryptoKit

// Shipping: Calibre import machinery removed 2026-09-19 (macOS reads the
// live metadata.db read-only via SmbCatalogDB — nothing is ever imported).
// This file keeps the library registry (Settings → saved Calibre library
// list) only.

public struct CalibreLibraryConfig: Codable, Identifiable, Equatable {
    public var id: String { name }
    public var name: String
    public var type: ConnectionType
    public var path: String
    public var isPrimary = false

    public init(name: String, type: ConnectionType, path: String, isPrimary: Bool = false) {
        self.name = name
        self.type = type
        self.path = path
        self.isPrimary = isPrimary
    }

    public enum ConnectionType: String, Codable {
        case local
        case smb
        case remote
    }

    public var displayPath: String {
        switch type {
        case .local: return path
        case .smb: return "smb://\(path)"
        case .remote: return path
        }
    }

    public var smbHost: String? {
        guard type == .smb else { return nil }
        return path.split(separator: "/").first.map(String.init)
    }

    public var smbShare: String? {
        guard type == .smb else { return nil }
        let parts = path.split(separator: "/")
        guard parts.count >= 2 else { return nil }
        return String(parts[1])
    }

    public var smbMetadataPath: String {
        let parts = path.split(separator: "/")
        guard parts.count >= 3 else { return "" }
        return parts.dropFirst(2).joined(separator: "/")
    }

    public var smbLibRoot: String {
        let meta = smbMetadataPath
        guard let slash = meta.lastIndex(of: "/") else { return "" }
        return String(meta[..<slash])
    }

    public var smbPassword: String {
        guard let host = smbHost else { return "" }
        return KeychainHelper.read(account: host) ?? ""
    }
}

@MainActor
public class CalibreManager: ObservableObject {
    public static let shared = CalibreManager()

    @Published public var libraries: [CalibreLibraryConfig] = []

    private let storageKey = "calibre_libraries"

    private init() {
        load()
    }

    func load() {
        if let data = UserDefaults.standard.data(forKey: storageKey),
           let decoded = try? JSONDecoder().decode([CalibreLibraryConfig].self, from: data) {
            libraries = decoded
            if libraries.count == 1, let sole = libraries.first, !sole.isPrimary {
                libraries[0].isPrimary = true
                save()
            }
        }
    }

    public func save() {
        if let data = try? JSONEncoder().encode(libraries) {
            UserDefaults.standard.set(data, forKey: storageKey)
        }
    }

    func add(_ library: CalibreLibraryConfig) {
        if !libraries.contains(where: { $0.id == library.id }) {
            var toAdd = library
            if libraries.isEmpty && !toAdd.isPrimary { toAdd.isPrimary = true }
            libraries.append(toAdd)
            if libraries.count == 1, !libraries[0].isPrimary {
                libraries[0].isPrimary = true
            }
            save()
        }
    }

    func remove(at index: Int) {
        libraries.remove(at: index)
        if libraries.count == 1, !libraries[0].isPrimary {
            libraries[0].isPrimary = true
        }
        save()
    }

    func remove(_ library: CalibreLibraryConfig) {
        libraries.removeAll { $0.id == library.id }
        if libraries.count == 1, let first = libraries.first, !first.isPrimary {
            libraries[0].isPrimary = true
        }
        save()
    }

    func setPrimary(_ library: CalibreLibraryConfig) {
        for i in libraries.indices {
            libraries[i].isPrimary = libraries[i].id == library.id
        }
        save()
    }

    func update(_ library: CalibreLibraryConfig) {
        if let idx = libraries.firstIndex(where: { $0.id == library.id }) {
            libraries[idx] = library
            save()
        }
    }
}
