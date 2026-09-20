import Foundation

public enum SmbService {
    private static var log: ReaderLog { ReaderLog.shared }

    /// POSIX errors worth one retry: the first SYN after idle can fail with
    /// ENETDOWN/UNREACH on a stale LAN route while a second attempt rides
    /// the live one. Login is idempotent, so a single re-attempt is safe.
    private static let retryablePOSIX: Set<Int32> = [
        50, // ENETDOWN
        51, // ENETUNREACH
        60, // ETIMEDOUT
        64, // EHOSTDOWN
        65, // EHOSTUNREACH
    ]

    private static func isTransientNetworkError(_ error: Error) -> Bool {
        if let posix = error as? POSIXError {
            return retryablePOSIX.contains(posix.code.rawValue)
        }
        let ns = error as NSError
        if ns.domain == NSPOSIXErrorDomain {
            return retryablePOSIX.contains(Int32(ns.code))
        }
        // Also treat textual timeouts / broken pipe as transient (SMBClient wraps POSIX as generic Error).
        // NWError carries e.g. "Network is down (50)" / "No route to host" as text only — match those too
        // so the first-SYN-after-idle retry actually fires for the metadata.db fetch.
        let desc = "\(error)".lowercased()
        if desc.contains("timed out") || desc.contains("timeout") || desc.contains("broken pipe") || desc.contains("connection reset") || desc.contains("connection refused") {
            return true
        }
        if desc.contains("network is down") || desc.contains("network is unreachable") || desc.contains("no route to host") || desc.contains("host is down") || desc.contains("host is unreachable") {
            return true
        }
        return false
    }

    private static func isAuthFailure(_ error: Error) -> Bool {
        let s = "\(type(of: error)): \(error)".lowercased()
        return s.contains("logon") || s.contains("logon_failure") || s.contains("access_denied") || s.contains("auth")
    }

    private static func describe(_ error: Error) -> String {
        "\(type(of: error)): \(error.localizedDescription)"
    }

    /// Runs work with a fresh client; retries once on transient network
    /// errors after a short beat for the route to settle. Auth and protocol
    /// failures throw immediately (no retry). For `test` we also retry once on
    /// *any* non-auth error because the first SYN after idle can stall even
    /// when the error is wrapped as a generic timeout/broken-pipe not matching
    /// POSIX codes — avoids "first test always Fail, second Pass".
    private static func withFreshClient<T>(host: String, op: String, _ work: (SMBClient) async throws -> T) async throws -> T {
        let client = SMBClient(host: host, port: 445)
        do {
            return try await work(client)
        } catch {
            try? client.session.disconnect()
            let isAuth = isAuthFailure(error)
            let shouldRetry = isTransientNetworkError(error) || (op == "test" && !isAuth)
            guard shouldRetry else {
                log.e("SMB", "\(op) failed host=\(host) err=\(describe(error))")
                throw error
            }
            log.i("SMB", "\(op) transient/non-auth failure, retrying host=\(host) err=\(describe(error))")
            try await Task.sleep(nanoseconds: 800_000_000)
            let retryClient = SMBClient(host: host, port: 445)
            do {
                return try await work(retryClient)
            } catch {
                try? retryClient.session.disconnect()
                log.e("SMB", "\(op) retry failed host=\(host) err=\(describe(error))")
                throw error
            }
        }
    }

    public static func testConnection(host: String, user: String, password: String, share: String, domain: String) async throws {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        try await withFreshClient(host: host, op: "test") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            try await client.connectShare(share)
            try await client.logoff()
        }
    }

    public static func connect(host: String, user: String, password: String, domain: String, share: String) async throws -> SMBClient {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        return try await withFreshClient(host: host, op: "connect") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            try await client.connectShare(share)
            return client
        }
    }

    public static func downloadFile(client: SMBClient, remotePath: String, to localURL: URL, progress: @escaping (Double) -> Void) async throws {
        try await client.download(path: remotePath, localPath: localURL, overwrite: true, progressHandler: progress)
    }

    public static func listShares(host: String, user: String, password: String, domain: String) async throws -> [String] {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        let shares = try await withFreshClient(host: host, op: "shares") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            let shares = try await client.listShares()
            try await client.logoff()
            return shares
        }
        return shares
            .map { $0.name }
            .filter { !$0.hasSuffix("$") }
            .sorted()
    }

    public static func listDirectory(host: String, user: String, password: String, share: String, domain: String, path: String) async throws -> [SmbEntry] {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        let files = try await withFreshClient(host: host, op: "list") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            try await client.connectShare(share)
            let files = try await client.listDirectory(path: path)
            try await client.logoff()
            return files
        }
        return files
            .filter { $0.name != "." && $0.name != ".." }
            .map { SmbEntry(name: $0.name, isDirectory: $0.isDirectory, size: Int($0.size)) }
    }

    public static func downloadFile(host: String, user: String, password: String, share: String, domain: String, remotePath: String, to localURL: URL, progress: @escaping (Double) -> Void) async throws {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        try await withFreshClient(host: host, op: "download") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            try await client.connectShare(share)
            try await client.download(path: remotePath, localPath: localURL, overwrite: true, progressHandler: progress)
            try await client.logoff()
        }
    }

    /// Memory-only fetch of a remote file — no local copy is written.
    /// Single-database rule: metadata.db is read live from the Settings SMB
    /// share straight into Data, then deserialized into :memory: SQLite.
    ///
    /// Share-tolerant open: Calibre (or another client) may hold metadata.db
    /// open for writing (SQLite locks). The vendored FileReader opens with
    /// share [.read], which the server rejects with STATUS_SHARING_VIOLATION
    /// in that case — so this opens read-only but sharing read+write+delete
    /// (same as Session.fileStat/queryDirectory already do), and retries a
    /// few times when the file is momentarily locked by a Calibre write.
    public static func downloadData(host: String, user: String, password: String, share: String, domain: String, remotePath: String) async throws -> Data {
        let userParam: String? = user.isEmpty ? nil : user
        let domainParam: String? = domain.isEmpty ? nil : domain
        let passwordParam: String? = password.isEmpty ? nil : password
        return try await withFreshClient(host: host, op: "download") { client in
            try await client.login(username: userParam, password: passwordParam, domain: domainParam)
            try await client.connectShare(share)
            let path = Pathname.normalize(remotePath)
            var lastError: Error?
            for attempt in 1...3 {
                do {
                    let data = try await downloadShared(client: client, path: path)
                    try await client.logoff()
                    return data
                } catch let e as ErrorResponse where NTStatus(e.header.status) == .sharingViolation {
                    lastError = e
                    log.i("SMB", "download sharing violation attempt=\(attempt) path=\(remotePath), retrying")
                    try await Task.sleep(nanoseconds: 1_000_000_000)
                }
            }
            throw lastError!
        }
    }

    /// Read-only download that tolerates other openers holding write/delete
    /// access (e.g. Calibre with the library open). Vendored FileReader is
    /// not used here because its share mode ([.read]) cannot be changed
    /// without modifying vendored code.
    private static func downloadShared(client: SMBClient, path: String) async throws -> Data {
        let response = try await client.session.create(
            desiredAccess: [.genericRead],
            fileAttributes: [],
            shareAccess: [.read, .write, .delete],
            createDisposition: .open,
            createOptions: [],
            name: path
        )
        let fileId = response.fileId
        do {
            var buffer = Data()
            var readResponse: Read.Response
            repeat {
                readResponse = try await client.session.read(fileId: fileId, offset: UInt64(buffer.count))
                buffer.append(readResponse.buffer)
            } while NTStatus(readResponse.header.status) != .endOfFile
            try await client.session.close(fileId: fileId)
            guard !buffer.isEmpty else {
                throw CocoaError(.fileReadUnknown)
            }
            return buffer
        } catch {
            try? await client.session.close(fileId: fileId)
            throw error
        }
    }

    public static func listDirectoryRecursive(host: String, user: String, password: String, share: String, domain: String, path: String) async throws -> [(name: String, path: String)] {
        var result: [(name: String, path: String)] = []
        let items = try await listDirectory(host: host, user: user, password: password, share: share, domain: domain, path: path)
        for item in items {
            let fullPath = path.isEmpty ? item.name : "\(path)/\(item.name)"
            if item.isDirectory {
                let subItems = try await listDirectoryRecursive(host: host, user: user, password: password, share: share, domain: domain, path: fullPath)
                result.append(contentsOf: subItems)
            } else {
                let ext = (item.name as NSString).pathExtension.lowercased()
                if supportedBookExts.contains(ext) {
                    result.append((name: item.name, path: fullPath))
                }
            }
        }
        return result
    }
}

public struct SmbEntry: Identifiable {
    public let id = UUID()
    public let name: String
    public let isDirectory: Bool
    public let size: Int

    public init(name: String, isDirectory: Bool, size: Int) {
        self.name = name
        self.isDirectory = isDirectory
        self.size = size
    }
}
