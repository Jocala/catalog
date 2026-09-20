import Foundation
import CatalogCore

// Whole-library Kobo index cache — one find, not per-book
// Replaces per-open find 540KB/0.32s with single cached lookup

enum KoboIndex {
    static var indexURL: URL { CatalogPaths.base.appendingPathComponent("kobo_index.txt") }

    static func cachedPaths() -> [String]? {
        guard let data = try? String(contentsOf: indexURL, encoding: .utf8) else { return nil }
        let lines = data.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        return lines.isEmpty ? nil : lines
    }

    static func save(_ paths: [String]) {
        let dir = indexURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let content = paths.joined(separator: "\n") + "\n"
        try? content.write(to: indexURL, atomically: true, encoding: .utf8)
    }

    static func contains(_ path: String, in cache: [String]? = nil) -> Bool {
        if let cache = cache { return cache.contains(path) }
        guard let c = cachedPaths() else { return false }
        return c.contains(path)
    }

    // Refresh via SSH find — file-backed, handles 6742 lines without Pipe deadlock
    static func refresh(ip: String) async -> [String]? {
        let result = await sshFind(ip: ip)
        guard let paths = result, !paths.isEmpty else { return nil }
        save(paths)
        return paths
    }

    private static func sshFind(ip: String) async -> [String]? {
        await Task.detached(priority: .userInitiated) { () -> [String]? in
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
            p.arguments = ["-T", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new", "root@\(ip)"]
            let input = Pipe()
            let tmpOut = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".koboindex")
            FileManager.default.createFile(atPath: tmpOut.path, contents: nil)
            guard let outHandle = try? FileHandle(forWritingTo: tmpOut) else { return nil }
            p.standardOutput = outHandle
            p.standardError = outHandle
            p.standardInput = input
            do {
                try p.run()
                let cmd = "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort\n"
                if let d = cmd.data(using: .utf8) { input.fileHandleForWriting.write(d) }
                input.fileHandleForWriting.closeFile()
                let deadline = Date().addingTimeInterval(35)
                while p.isRunning && Date() < deadline { usleep(50000) }
                if p.isRunning { p.terminate(); usleep(200000); if p.isRunning { p.interrupt() } }
                try? outHandle.close()
                let data = (try? Data(contentsOf: tmpOut)) ?? Data()
                try? FileManager.default.removeItem(at: tmpOut)
                let out = String(data: data, encoding: .utf8) ?? ""
                if p.isRunning { return nil }
                let lines = out.split(separator: "\n").map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }.filter { $0.hasPrefix("/mnt/onboard/") }
                return lines.isEmpty ? nil : lines
            } catch {
                try? outHandle.close()
                try? FileManager.default.removeItem(at: tmpOut)
                return nil
            }
        }.value
    }
}
