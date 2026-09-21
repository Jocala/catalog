import Foundation
import CatalogCore

// WiFi sync of a Calibre EPUB to the Kobo predicted path (raw bytes).
// Transport is the same `ssh root@<kobo-ip>` channel KoboLauncher uses;
// stock Kobo has no sftp-server/scp, so pushes stream as a base64 heredoc
// (raw binary on shared stdin races the shell — never do `cat >` + bytes).
// Writes are new-file-only (guarded re-check) and atomic (.part + mv).

enum KoboSync {
    /// Fetch a book's EPUB bytes from the Calibre library (SMB or local).
    /// Returns bytes + calibre's uncompressed_size (dialog ~size).
    static func fetchEpubBytes(bookId: Int64) async throws -> (data: Data, size: Int64) {
        guard let src = try await SmbCatalogDB.shared.epubSource(bookId: bookId) else {
            throw KoboSyncError.noEpubFormat
        }
        if src.isSMB {
            let data = try await SmbService.downloadData(
                host: src.host, user: src.user, password: src.password,
                share: src.share, domain: src.domain, remotePath: src.filePath)
            guard !data.isEmpty else { throw KoboSyncError.emptyFile }
            return (data, src.size)
        }
        let data = try Data(contentsOf: URL(fileURLWithPath: src.filePath), options: .mappedIfSafe)
        guard !data.isEmpty else { throw KoboSyncError.emptyFile }
        return (data, src.size)
    }

    /// Push bytes to the exact predicted Kobo path, then verify.
    /// Sequence: df guard → mkdir -p → re-check absent → base64-heredoc
    /// to .part → size verify → mv. Throws KoboSyncError on any failure.
    static func pushToKobo(ip: String, predicted: String, data: Data) async throws {
        let esc = escShell(predicted)
        let dir = (predicted as NSString).deletingLastPathComponent
        let escDir = escShell(dir)
        let part = predicted + ".part"
        let escPart = escShell(part)
        let need = Int64(data.count)

        // 1. Free-space guard (+1MB margin).
        let df = sshText(ip: ip, remoteCmd: "df -k /mnt/onboard | tail -n 1", timeout: 10)
        guard df.code == 0 else { throw KoboSyncError.transport(df.output) }
        let fields = df.output.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        guard fields.count >= 4, let availKB = Int64(fields[3]),
              availKB * 1024 >= need + 1_048_576 else {
            throw KoboSyncError.storageFull
        }
        // 2. Author dir + don't clobber a file that appeared since.
        // Also clear any .part left by an earlier failed push.
        let prep = sshText(ip: ip, remoteCmd: "mkdir -p '\(escDir)' && rm -f '\(escPart)' && if [ -f '\(esc)' ]; then echo present; else echo absent; fi", timeout: 10)
        guard prep.code == 0 else { throw KoboSyncError.transport(prep.output) }
        if prep.output.contains("present") { throw KoboSyncError.alreadyThere }

        // 3. Stream bytes as a base64 heredoc (all stdin stays plain text),
        // then size-verify and atomically move into place.
        let push = sshPushB64(ip: ip, partPath: part, finalPath: predicted, data: data, timeout: 180)
        guard push.code == 0, push.output.contains("moved:") else {
            throw KoboSyncError.transport(push.output)
        }
        ReaderLog.shared.i("KoboSync", "pushed \(need) bytes to \(predicted)")
        if var paths = KoboIndex.cachedPaths() {
            if !paths.contains(predicted) { paths.append(predicted); KoboIndex.save(paths) }
        } else {
            KoboIndex.save([predicted])
        }
    }

    // MARK: - SSH primitives (mirrors KoboLauncher.sshSync conventions)

    /// Password configured in Settings → Kobo for this IP, if any.
    /// Passwords live in the app password store under `kobo-passwords`
    /// (KeychainHelper UserDefaults mirror — no login-keychain prompts).
    static func koboSSHPassword(forIP ip: String) -> String? {
        guard let s = KeychainHelper.read(account: "kobo-passwords"),
              let d = s.data(using: .utf8),
              let dict = try? JSONDecoder().decode([String: String].self, from: d) else { return nil }
        let pw = dict[ip.trimmingCharacters(in: .whitespacesAndNewlines)] ?? ""
        return pw.isEmpty ? nil : pw
    }

    /// argv + environment for a Kobo SSH connection. Password present →
    /// askpass auth (stdin stays free for the heredoc); absent → BatchMode,
    /// byte-identical to the legacy key-only invocation. No new dependencies
    /// (no sshpass): `Process` has no tty, so with SSH_ASKPASS_REQUIRE=force
    /// ssh uses the helper script. The script is chmod 700 and removed via
    /// the returned cleanup (call it on every exit path, e.g. `defer`).
    static func koboSSHInvocation(ip: String, timeout: Int, password: String?) -> (args: [String], env: [String: String], cleanup: () -> Void) {
        var args = ["-T", "-o", "ConnectTimeout=\(timeout)", "-o", "StrictHostKeyChecking=accept-new"]
        var env: [String: String] = [:]
        var cleanup: () -> Void = {}
        if let pw = password, !pw.isEmpty {
            let script = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".koboask")
            let body = "#!/bin/sh\nexec printf '%s' \"$KOBO_SSH_PASS\"\n"
            if (try? body.write(to: script, atomically: true, encoding: .utf8)) != nil {
                try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
                args += ["-o", "NumberOfPasswordPrompts=1"]
                env["SSH_ASKPASS"] = script.path
                env["SSH_ASKPASS_REQUIRE"] = "force"
                env["KOBO_SSH_PASS"] = pw
                cleanup = { try? FileManager.default.removeItem(at: script) }
            } else {
                args += ["-o", "BatchMode=yes"]
            }
        } else {
            args += ["-o", "BatchMode=yes"]
        }
        args.append("root@\(ip)")
        return (args, env, cleanup)
    }

    /// True when ssh output shows an auth rejection (wrong password or key).
    static func isAuthFailure(output: String, code: Int32) -> Bool {
        code == 255 && output.localizedCaseInsensitiveContains("Permission denied")
    }

    static func escShell(_ s: String) -> String {
        s.replacingOccurrences(of: "'", with: "'\\''")
    }

    /// Text command via stdin heredoc (Dropbear user-rc requires this).
    @discardableResult
    static func sshText(ip: String, remoteCmd: String, timeout: Int) -> (output: String, code: Int32) {
        sshCommon(ip: ip, timeout: timeout) { input in
            if let d = (remoteCmd + "\n").data(using: .utf8) { input.fileHandleForWriting.write(d) }
            input.fileHandleForWriting.closeFile()
        }
    }

    /// Base64-heredoc push: the payload travels as plain text, so the
    /// remote shell never parses binary (raw bytes on shared stdin race
    /// `cat` and die with e.g. "EOF in backquote substitution").
    /// Decodes to .part, size-verifies against the original byte count,
    /// and atomically moves into place — all in one SSH invocation.
    /// Delimiter uses underscores (outside the base64 alphabet), so payload
    /// lines can never collide with it.
    static func sshPushB64(ip: String, partPath: String, finalPath: String, data: Data, timeout: Int) -> (output: String, code: Int32) {
        let escPart = escShell(partPath)
        let escFinal = escShell(finalPath)
        let b64 = data.base64EncodedString(options: .lineLength76Characters)
        let script = """
        base64 -d > '\(escPart)' <<'KOBO_EOF_DONE'
        \(b64)
        KOBO_EOF_DONE
        n=$(wc -c < '\(escPart)'); if [ "$n" -eq \(data.count) ]; then mv '\(escPart)' '\(escFinal)' && echo "moved:$n"; else rm -f '\(escPart)'; echo "size-mismatch:$n"; exit 1; fi
        """
        return sshText(ip: ip, remoteCmd: script, timeout: timeout)
    }

    private static func sshCommon(ip: String, timeout: Int, feed: (Pipe) -> Void) -> (output: String, code: Int32) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
        let (sshArgs, sshEnv, askpassCleanup) = koboSSHInvocation(ip: ip, timeout: timeout, password: koboSSHPassword(forIP: ip))
        defer { askpassCleanup() }
        p.arguments = sshArgs
        if !sshEnv.isEmpty {
            var e = ProcessInfo.processInfo.environment
            e.merge(sshEnv) { _, new in new }
            p.environment = e
        }
        let input = Pipe()
        let tmpOut = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".kobosync")
        FileManager.default.createFile(atPath: tmpOut.path, contents: nil)
        guard let outHandle = try? FileHandle(forWritingTo: tmpOut) else {
            return ("ssh temp file failed", -1)
        }
        p.standardOutput = outHandle; p.standardError = outHandle; p.standardInput = input
        do {
            try p.run()
            feed(input)
            let deadline = Date().addingTimeInterval(Double(timeout + 5))
            while p.isRunning && Date() < deadline { usleep(50000) }
            if p.isRunning { p.terminate(); usleep(200000); if p.isRunning { p.interrupt() } }
            try? outHandle.close()
            let data = (try? Data(contentsOf: tmpOut)) ?? Data()
            try? FileManager.default.removeItem(at: tmpOut)
            let out = String(data: data, encoding: .utf8) ?? ""
            if p.isRunning { return (out + "\n(timed out)", 124) }
            return (out, p.terminationStatus)
        } catch {
            try? outHandle.close()
            try? FileManager.default.removeItem(at: tmpOut)
            return ("ssh launch failed: \(error.localizedDescription)", -1)
        }
    }
}

enum KoboSyncError: LocalizedError {
    case noEpubFormat
    case emptyFile
    case storageFull
    case alreadyThere
    case transport(String)

    var errorDescription: String? {
        switch self {
        case .noEpubFormat:
            return "No EPUB format in Calibre for this book — sync needs an EPUB."
        case .emptyFile:
            return "Calibre's EPUB file is empty — not syncing."
        case .storageFull:
            return "Kobo storage is full — free space and try again."
        case .alreadyThere:
            return "The book appeared on the Kobo since — just tap Read on Kobo again."
        case .transport(let m):
            return "Sync failed: \(m.prefix(300))"
        }
    }
}
