import Foundation
import OSLog

public struct LogSession {
    public let title: String
    public let content: String
    public init(title: String, content: String) {
        self.title = title
        self.content = content
    }
}

public class ReaderLog {
    public static let shared = ReaderLog()

    private let logFile: URL
    private let oldLogFile: URL
    private let maxSize = 512 * 1024
    private let queue = DispatchQueue(label: "com.jocala.catalog.log", qos: .utility)
    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    private init() {
        #if os(macOS)
        let docs = CatalogPaths.base
        #else
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first!
        #endif
        logFile = docs.appendingPathComponent("app_log.txt")
        oldLogFile = docs.appendingPathComponent("app_log_old.txt")

        if FileManager.default.fileExists(atPath: logFile.path) {
            let tmp = docs.appendingPathComponent("app_log.tmp")
            try? FileManager.default.moveItem(at: logFile, to: tmp)
            try? FileManager.default.removeItem(at: oldLogFile)
            try? FileManager.default.moveItem(at: tmp, to: oldLogFile)
        }
        write("I", "ReaderLog", "=== App Start ===")
    }

    public func i(_ tag: String, _ msg: String) { write("I", tag, msg) }
    public func e(_ tag: String, _ msg: String) { write("E", tag, msg) }
    public func d(_ tag: String, _ msg: String) {
        guard UserDefaults.standard.bool(forKey: "diagnostic_logging") else { return }
        write("D", tag, msg)
    }

    public func clear() {
        queue.sync {
            try? FileManager.default.removeItem(at: logFile)
            try? FileManager.default.removeItem(at: oldLogFile)
        }
    }

    public var current: LogSession {
        let content = (try? String(contentsOf: logFile)).map { $0.trimmingCharacters(in: .newlines) } ?? "(empty)"
        return LogSession(title: "Current Session", content: content)
    }

    public var previous: LogSession? {
        guard FileManager.default.fileExists(atPath: oldLogFile.path) else { return nil }
        let content = (try? String(contentsOf: oldLogFile)).map { $0.trimmingCharacters(in: .newlines) } ?? ""
        return LogSession(title: "Previous Session", content: content)
    }

    private func write(_ level: String, _ tag: String, _ msg: String) {
        os_log("[%{public}@] %{public}@/%{public}@: %{public}@", log: OSLog(subsystem: "com.jocala.catalog", category: tag), type: .info, Self.timeFormatter.string(from: Date()), level, tag, msg)
        let line = "[\(Self.timeFormatter.string(from: Date()))] \(level)/\(tag): \(msg)\n"
        queue.sync {
            trimIfNeeded()
            guard let handle = FileHandle(forWritingAtPath: logFile.path) else {
                try? line.write(to: logFile, atomically: false, encoding: .utf8)
                return
            }
            defer { try? handle.close() }
            handle.seekToEndOfFile()
            if let data = line.data(using: .utf8) {
                handle.write(data)
            }
            handle.synchronizeFile()
        }
    }

    private func trimIfNeeded() {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: logFile.path),
              let size = attrs[.size] as? UInt64,
              size > maxSize,
              let content = try? String(contentsOf: logFile) else { return }
        let lines = content.components(separatedBy: .newlines)
        let trimmed = lines.dropFirst(lines.count / 2).joined(separator: "\n")
        try? trimmed.write(to: logFile, atomically: true, encoding: .utf8)
    }
}
