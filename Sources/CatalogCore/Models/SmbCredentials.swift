import Foundation

public struct SmbServer: Codable, Identifiable, Equatable, Hashable {
    public var id: String { host }
    public var label: String = ""
    public var host: String = ""
    public var port: Int = 445
    public var user: String = ""
    public var domain: String = ""
    public var shares: [SmbShare] = []

    public init(label: String = "", host: String = "", port: Int = 445, user: String = "", domain: String = "", shares: [SmbShare] = []) {
        self.label = label
        self.host = host
        self.port = port
        self.user = user
        self.domain = domain
        self.shares = shares
    }

    public var displayName: String {
        label.isEmpty ? host : label
    }

    public var primaryShareName: String {
        get { shares.first?.name ?? "" }
        set {
            if shares.isEmpty {
                shares = [SmbShare(name: newValue)]
            } else if !newValue.isEmpty {
                shares[0] = SmbShare(name: newValue, calibreMetadataPath: shares[0].calibreMetadataPath)
            }
        }
    }
}

public struct SmbShare: Codable, Identifiable, Equatable, Hashable {
    public var id: String { name }
    public var name: String = ""
    public var calibreMetadataPath: String = ""

    public init(name: String = "", calibreMetadataPath: String = "") {
        self.name = name
        self.calibreMetadataPath = calibreMetadataPath
    }
}

extension SmbServer {
    public static var saved: [SmbServer] {
        get {
            guard let data = UserDefaults.standard.data(forKey: "smb_servers") else {
                return []
            }
            return (try? JSONDecoder().decode([SmbServer].self, from: data)) ?? []
        }
        set {
            guard let data = try? JSONEncoder().encode(newValue) else { return }
            UserDefaults.standard.set(data, forKey: "smb_servers")
        }
    }

    public static var primary: SmbServer? {
        saved.first
    }

    public static func save(_ servers: [SmbServer]) {
        Self.saved = servers
    }
}
