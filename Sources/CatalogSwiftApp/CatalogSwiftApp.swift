import SwiftUI
import SQLite3
import AppKit
import CryptoKit
import CatalogCore

// Throttler to prevent 800 concurrent thumbnail SMB stats (was thundering herd).
// Cancellation-safe: a .task cancelled while parked removes its waiter instead
// of leaking a slot (which wedged the gate at 6/6 and stalled all later covers).
actor ThumbnailThrottler {
    private var running = 0
    private let maxConcurrent = 6
    private var waiters: [(UUID, CheckedContinuation<Bool, Never>)] = []
    /// Returns false if cancelled while waiting (caller must not proceed).
    func wait() async -> Bool {
        if running < maxConcurrent {
            running += 1
            return true
        }
        let id = UUID()
        let granted: Bool = await withTaskCancellationHandler {
            await withCheckedContinuation { c in waiters.append((id, c)) }
        } onCancel: {
            Task { await self.cancelWaiter(id: id) }
        }
        if granted { running += 1 }
        return granted
    }
    func signal() {
        running -= 1
        if !waiters.isEmpty {
            let next = waiters.removeFirst()
            next.1.resume(returning: true)
        }
    }
    private func cancelWaiter(id: UUID) {
        if let i = waiters.firstIndex(where: { $0.0 == id }) {
            let w = waiters.remove(at: i)
            w.1.resume(returning: false)
        }
    }
}

// MARK: - Kobo launcher (internal SSH, no script)
// Uses Settings Kobo IP (user adds their own). Test uses ping, open uses SSH heredoc to KOReader.

/// Outcome of a Read-on-Kobo attempt. The two missing cases (clean no-match
/// and stale-cache ghost) offer Sync & Open instead of a dead-end alert.
enum KoboResult: Sendable {
    case opened(String)
    case failed(String)
    case missingOnKobo(predicted: String, message: String)
}

/// A book confirmed absent from the Kobo, awaiting the user's sync choice.
struct KoboMissing: Identifiable {
    let id = UUID()
    let book: CatalogBook
    let predicted: String
    let message: String
    let approxSize: Int64?
    var mbText: String {
        guard let s = approxSize, s > 0 else { return "this" }
        return String(format: "%.1f MB", Double(s) / 1_048_576)
    }
}

enum KoboLauncher {
    /// Launch a book on Kobo by Title via KoboReader.sqlite (primary) with author+title strict fallback (fail-closed).
    @MainActor
    static func open(book: CatalogBook, store: CatalogStore? = nil) async -> String {
        let query = "\(book.author) \(book.title)"
        // One-time stacking-fix prompt (global per installation). Verbs: Install / Not Now.
        if !UserDefaults.standard.bool(forKey: "kobo_handoff_prompt_done") {
            let koboIP = UserDefaults.standard.string(forKey: "kobo_ip")?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !koboIP.isEmpty {
                // Check KOReader presence and handoff installed.
                let present = await Task.detached(priority: .userInitiated) {
                    KoboLauncher.sshSync(ip: koboIP, remoteCmd: "test -d /mnt/onboard/.adds/koreader && echo present || echo absent", timeout: 5)
                }.value
                if present.output.contains("present") {
                    let check = await Task.detached(priority: .userInitiated) {
                        KoboLauncher.sshSync(ip: koboIP, remoteCmd: "grep -q koreader-handoff /mnt/onboard/.adds/koreader/koreader.sh 2>/dev/null && test -f /mnt/onboard/.adds/koreader/patches/2-handoff.lua && echo ok || echo missing", timeout: 6)
                    }.value
                    if check.output.contains("missing") {
                        // Need prompt — store pending book and signal UI.
                        store?.koboHandoffPendingBook = book
                        store?.showHandoffPrompt = true
                        store?.koboStatus = "Stacking fix available"
                        ReaderLog.shared.i("Kobo", "handoff prompt shown for \(koboIP)")
                        return "handoff-prompt"
                    } else if check.output.contains("ok") {
                        UserDefaults.standard.set(true, forKey: "kobo_handoff_prompt_done")
                    }
                } else {
                    // No KOReader — no stacking issue, mark done.
                    UserDefaults.standard.set(true, forKey: "kobo_handoff_prompt_done")
                }
            }
        }
        return await runKobo(query: query, title: book.title, author: book.author, store: store, book: book)
    }

    /// Install the stacking fix (2 files) silently — called from the prompt’s Install.
    @MainActor
    static func installHandoff(store: CatalogStore? = nil) async {
        let koboIP = UserDefaults.standard.string(forKey: "kobo_ip")?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !koboIP.isEmpty else { return }
        store?.koboStatus = "Installing stacking fix…"
        let lua = """
-- 2-handoff.lua — stacking fix drop-in
local lfs=require("libs/libkoreader-lfs")
local logger=require("logger")
local Handoff={path="/tmp/koreader-handoff",interval=1.0,armed=false}
function Handoff:init() if self.armed then return end; self.armed=true; require("ui/uimanager"):scheduleIn(self.interval,function() self:tick() end) end
function Handoff:tick() if self.armed then require("ui/uimanager"):scheduleIn(self.interval,function() self:tick() end) end; local ok,err=pcall(function() self:check() end); if not ok then logger.warn("Handoff:",err) end end
function Handoff:check()
  if lfs.attributes(self.path,"mode")~="file" then return end
  local f=io.open(self.path,"r"); if not f then return end; local t=f:read("*l"); f:close()
  if not t or t=="" then os.remove(self.path); return end
  if lfs.attributes(t,"mode")~="file" then logger.warn("Handoff target missing:",t); os.remove(self.path); return end
  local ok,ReaderUI=pcall(require,"apps/reader/readerui")
  if ok and ReaderUI.instance then logger.info("Handoff: switching to",t); os.remove(self.path); ReaderUI.instance:switchDocument(t); return end
  local ok2,FM=pcall(require,"apps/filemanager/filemanager")
  if ok2 and FM.instance then local fu=require("apps/filemanager/filemanagerutil"); logger.info("Handoff from filemanager",t); os.remove(self.path); fu.openFile(FM.instance,t); return end
end
function Handoff:quit() self.armed=false end
pcall(function() Handoff:init() end)
"""
        // 1. patches dir + lua
        let cmds = [
            "mkdir -p /mnt/onboard/.adds/koreader/patches",
            "cat > /tmp/2-handoff.lua.tmp <<'KOBO_HO_EOF'\n\(lua)\nKOBO_HO_EOF\nmv -f /tmp/2-handoff.lua.tmp /mnt/onboard/.adds/koreader/patches/2-handoff.lua",
            "if ! grep -q koreader-handoff /mnt/onboard/.adds/koreader/koreader.sh; then cp -p /mnt/onboard/.adds/koreader/koreader.sh /mnt/onboard/.adds/koreader/koreader.sh.orig-handoff 2>/dev/null; awk 'NR==1,0 {print} /exec \"\\/tmp\\/koreader\\.sh\"/ {print \"\"; print \"if pidof reader.lua >/dev/null 2>&1; then\"; print \"  if [ -n \\\"${1}\\\" ]; then\"; print \"    printf '\\''%s\\\\n'\\'' \\\"${1}\\\" >\\\"/tmp/koreader-handoff.tmp.$$\\\"\"; print \"    mv -f \\\"/tmp/koreader-handoff.tmp.$$\\\" \\\"/tmp/koreader-handoff\\\"\"; print \"  fi\"; print \"  exit 0\"; print \"fi\"} 1' /mnt/onboard/.adds/koreader/koreader.sh > /tmp/koreader.sh.new && mv /tmp/koreader.sh.new /mnt/onboard/.adds/koreader/koreader.sh && chmod +x /mnt/onboard/.adds/koreader/koreader.sh; fi; sync"
        ]
        for c in cmds {
            let r = await Task.detached(priority: .userInitiated) { KoboLauncher.sshSync(ip: koboIP, remoteCmd: c, timeout: 12) }.value
            if r.code != 0 {
                ReaderLog.shared.e("Kobo", "handoff install step failed: \(r.output.prefix(300))")
                break
            }
        }
        UserDefaults.standard.set(true, forKey: "kobo_handoff_prompt_done")
        store?.koboStatus = "Stacking fix installed"
        ReaderLog.shared.i("Kobo", "handoff installed for \(koboIP)")
    }

    @MainActor
    static func runKobo(query: String, title: String, author: String, store: CatalogStore? = nil, book: CatalogBook? = nil) async -> String {
        store?.koboStatus = "Opening “\(query)” on Kobo…"
        let koboIP = UserDefaults.standard.string(forKey: "kobo_ip")?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !koboIP.isEmpty else {
            let msg = "Kobo IP not set — enter it in Settings → Kobo"
            store?.koboStatus = msg; store?.koboError = msg; return msg
        }
        let result = await runKoboSSH(query: query, title: title, author: author, ip: koboIP)
        switch result {
        case .opened(let out):
            await handleKoboSuccess(out: out, query: query, store: store)
            return out
        case .failed(let out):
            await handleKoboFailure(out: out, code: 1, query: query, store: store)
            return out
        case .missingOnKobo(let predicted, let msg):
            var approx: Int64? = nil
            if let b = book { approx = try? await SmbCatalogDB.shared.epubSource(bookId: b.id)?.size }
            if let b = book {
                store?.koboMissing = KoboMissing(book: b, predicted: predicted, message: msg, approxSize: approx)
                store?.koboStatus = "Not on Kobo: \(b.title)"
            } else {
                await handleKoboFailure(out: msg, code: 1, query: query, store: store)
            }
            return msg
        }
    }

    @MainActor
    private static func handleKoboSuccess(out: String, query: String, store: CatalogStore?) async {
        let logLine = out.trimmingCharacters(in: .whitespacesAndNewlines).prefix(400)
        let koboIP = UserDefaults.standard.string(forKey: "kobo_ip") ?? ""
        ReaderLog.shared.i("Kobo", "open query='\(query)' ip=\(koboIP) exit=0 out='\(logLine)'")
        store?.koboStatus = "Opened on Kobo: \(query)"
        scheduleKoboStatusClear(query: query, store: store)
    }

    @MainActor
    private static func handleKoboFailure(out: String, code: Int32, query: String, store: CatalogStore?) async {
        let logLine = out.trimmingCharacters(in: .whitespacesAndNewlines).prefix(400)
        let koboIP = UserDefaults.standard.string(forKey: "kobo_ip") ?? ""
        ReaderLog.shared.i("Kobo", "open query='\(query)' ip=\(koboIP) exit=\(code) out='\(logLine)'")
        // Wrong-password note: auth rejection while a password is configured
        // for this IP means the stored password is bad — say so explicitly
        // instead of the generic failure text. Without a configured password
        // the same rejection means key auth failed (different problem).
        if KoboSync.isAuthFailure(output: out, code: code), !koboIP.isEmpty,
           KoboSync.koboSSHPassword(forIP: koboIP) != nil {
            let msg = "SSH password rejected for \(koboIP) — check Settings → Kobo."
            ReaderLog.shared.i("Kobo", "auth failure ip=\(koboIP)")
            store?.koboError = msg
            store?.koboStatus = msg
            return
        }
        let err = out.trimmingCharacters(in: .whitespacesAndNewlines)
        let short = err.isEmpty ? "exit \(code)" : String(err.prefix(400))
        // Popup for any communication failure (sleeping, timeout, etc.)
        store?.koboError = short.contains("sleeping") || short.contains("unreachable") || short.contains("timed out") || code != 0
            ? short : "Kobo failed (\(code)): \(short)"
        // If store didn't set specific sleeping text, ensure dialog appears
        if store?.koboError == nil { store?.koboError = short }
        store?.koboStatus = "Kobo failed (\(code)): \(logLine)"
        scheduleKoboStatusClear(query: query, store: store)
    }

    @MainActor
    private static func scheduleKoboStatusClear(query: String, store: CatalogStore?) {
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            if store?.koboStatus == "Opened on Kobo: \(query)" || (store?.koboStatus.hasPrefix("Kobo failed") ?? false) {
                store?.koboStatus = ""
            }
        }
    }

    /// Sync a missing book from Calibre to the Kobo predicted path, then open.
    @MainActor
    static func syncAndOpen(missing: KoboMissing, store: CatalogStore?) async {
        let koboIP = UserDefaults.standard.string(forKey: "kobo_ip")?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !koboIP.isEmpty else {
            store?.koboError = "Kobo IP not set — enter it in Settings → Kobo"
            return
        }
        do {
            store?.koboStatus = "Fetching “\(missing.book.title)” from Calibre…"
            let (bytes, _) = try await KoboSync.fetchEpubBytes(bookId: missing.book.id)
            let mb = String(format: "%.1f", Double(bytes.count) / 1_048_576)
            store?.koboStatus = "Syncing \(mb) MB to Kobo…"
            try await KoboSync.pushToKobo(ip: koboIP, predicted: missing.predicted, data: bytes)
            store?.koboMissing = nil
            _ = await open(book: missing.book, store: store)
        } catch {
            store?.koboMissing = nil
            await handleKoboFailure(out: error.localizedDescription, code: 1, query: "\(missing.book.author) \(missing.book.title)", store: store)
        }
    }

    // Internal SSH executor: Calibre-predicted Kobo path (primary) → strict title+author fallback → nohup koreader.sh (fail-closed)
    private static func runKoboSSH(query: String, title: String, author: String, ip: String) async -> KoboResult {
        await Task.detached(priority: .userInitiated) { () -> KoboResult in
            // 1. Quick reachability: ssh echo ok with 5s timeout
            let probe = sshSync(ip: ip, remoteCmd: "echo ok", timeout: 5)
            if probe.code != 0 || !probe.output.lowercased().contains("ok") {
                if KoboSync.isAuthFailure(output: probe.output, code: probe.code),
                   KoboSync.koboSSHPassword(forIP: ip) != nil {
                    return .failed("SSH password rejected for \(ip) — check Settings → Kobo.")
                }
                let hint = probe.output.isEmpty ? "Kobo is sleeping or unreachable at \(ip) — press power button to wake, then try again." : probe.output
                let msg = hint.contains("sleeping") ? hint : "Kobo is sleeping or unreachable at \(ip) — press power button to wake.\n\(hint)"
                return .failed(msg)
            }
            func norm(_ s: String) -> String { s.folding(options: .diacriticInsensitive, locale: .current).lowercased().filter { $0.isLetter || $0.isNumber } }
            let titleTrim = title.trimmingCharacters(in: .whitespacesAndNewlines)
            let authorTrim = author.trimmingCharacters(in: .whitespacesAndNewlines)
            var lastName = ""
            if authorTrim.contains(",") {
                lastName = authorTrim.split(separator: ",").first?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            } else {
                lastName = authorTrim.split(whereSeparator: { $0.isWhitespace }).last.map(String.init) ?? authorTrim
            }
            let lastNameNorm = norm(lastName)
            // 2. Primary: Calibre-predicted Kobo path (replicates calibre/devices/kobo/driver.py)
            // Uses stored author_sort/title_sort via KoboPath, handles First<->Last, ", The", .kepub.epub, 185, sanitize
            let predicted = KoboPath.predictedPath(title: title, authorSort: author, authorsNatural: KoboPath.naturalName(fromSort: author))
            let escPred = predicted.replacingOccurrences(of: "'", with: "'\\''")
            let check = sshSync(ip: ip, remoteCmd: "if [ -f '\(escPred)' ]; then echo \"exists:\(escPred)\"; else echo \"missing\"; fi", timeout: 6)
            var filtered: [String] = []
            if check.output.contains("exists:") {
                filtered = [predicted]
            } else {
                // 3. Fallback: strict title+author match via cached index or live find — no best-guess
                // Try cached index first (whole-library, 6742, avoids per-open find)
                var candidates: [String]? = KoboIndex.cachedPaths()
                if candidates == nil {
                    // No cache — do live find (file-backed, handles 540KB without Pipe deadlock)
                    let list = sshSync(ip: ip, remoteCmd: "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort", timeout: 30)
                    if list.code != 0 {
                        if list.output.contains("/mnt/onboard/") && list.output.contains(".epub") {
                            // continue with partial
                        } else {
                            return .failed("Failed to list books on Kobo: \(list.output)")
                        }
                    }
                    candidates = list.output.split(separator: "\n").map(String.init).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
                    if let c = candidates, !c.isEmpty { KoboIndex.save(c) } // cache for next opens
                }
                guard var cand = candidates, !cand.isEmpty else { return .failed("No books found on Kobo (find returned empty) — is /mnt/onboard mounted?") }
                // Strict: require ALL title tokens + author lastName in path (diacritic-insensitive)
                let rawTokens: [String] = titleTrim.split(whereSeparator: { $0.isWhitespace }).map(String.init).filter { !$0.isEmpty }
                let normedTokens: [String] = rawTokens.map { norm($0) }
                let titleTokens: [String] = normedTokens.filter { !$0.isEmpty && $0 != "a" && $0 != "an" && $0 != "the" }
                let requiredTokens: [String] = titleTokens.isEmpty ? [norm(titleTrim)].filter { !$0.isEmpty } : titleTokens
                func strictMatches(in list: [String]) -> [String] {
                    list.filter { p in
                        let n = norm(p)
                        return requiredTokens.allSatisfy { n.contains($0) }
                            && (lastNameNorm.isEmpty || n.contains(lastNameNorm))
                    }
                }
                func titleOnlyMatches(in list: [String]) -> [String] {
                    list.filter { p in
                        let n = norm(p)
                        return requiredTokens.allSatisfy { n.contains($0) }
                    }
                }
                var strict = strictMatches(in: cand)
                var titleOnly = strict.isEmpty ? titleOnlyMatches(in: cand) : []
                if strict.isEmpty && titleOnly.isEmpty {
                    // Refresh-on-miss: the cached index may predate books synced
                    // since. One live find, overwrite the cache, retry once.
                    ReaderLog.shared.i("Kobo", "0 matches on cached index (\(cand.count) paths) — refreshing index and retrying once")
                    let fresh = sshSync(ip: ip, remoteCmd: "find /mnt/onboard -type f \\( -name \"*.kepub.epub\" -o -name \"*.epub\" \\) -not -path \"*/.kobo/*\" | sort", timeout: 30)
                    let freshPaths = fresh.output.split(separator: "\n").map(String.init).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
                    if !freshPaths.isEmpty {
                        KoboIndex.save(freshPaths)
                        cand = freshPaths
                        ReaderLog.shared.i("Kobo", "index refreshed (\(cand.count) paths), retrying match")
                        strict = strictMatches(in: cand)
                        titleOnly = strict.isEmpty ? titleOnlyMatches(in: cand) : []
                    }
                }
                if strict.isEmpty {
                    // Phase 2: title tokens alone (covers series-prefix filename like "Devices & Desires - Dalgleish 08 - P. D. James" and minor spelling variants like Dalgliesh vs Dalgleish)
                    if titleOnly.count == 1 {
                        ReaderLog.shared.i("Kobo", "strict title+author 0, title-only fallback 1 → \(titleOnly[0])")
                        filtered = titleOnly
                    } else if titleOnly.isEmpty {
                        return .missingOnKobo(predicted: predicted, message: "“\(titleTrim)” was not found on this device.")
                    } else {
                        let preview = titleOnly.prefix(10).map { "  \($0.replacingOccurrences(of: "/mnt/onboard/", with: ""))" }.joined(separator: "\n")
                        return .failed("\(titleOnly.count) matches for \"\(titleTrim)\" (title-only, author \(lastName) not found) — ambiguous, not opening:\n\(preview)\(titleOnly.count > 10 ? "\n  ... +\(titleOnly.count - 10) more" : "")\nRefine title or check series prefix.")
                    }
                } else if strict.count == 1 {
                    filtered = strict
                } else {
                    let preview = strict.prefix(10).map { "  \($0.replacingOccurrences(of: "/mnt/onboard/", with: ""))" }.joined(separator: "\n")
                    return .failed("\(strict.count) matches for \"\(titleTrim)\" by \(lastName) (strict) — ambiguous, not opening:\n\(preview)\(strict.count > 10 ? "\n  ... +\(strict.count - 10) more" : "")")
                }
            }
            let chosen: String
            if filtered.count == 1 {
                chosen = filtered[0]
            } else {
                return .failed("Internal error: ambiguous match for \"\(titleTrim)\"")
            }
            // 4. Open via KOReader (OCP path first, fallback legacy)
            let esc = chosen.replacingOccurrences(of: "'", with: "'\\''").replacingOccurrences(of: "\"", with: "\\\"")
            // Use single ssh invocation: test .adds path then nohup
            let openCmd = "if [ -x /mnt/onboard/.adds/koreader/koreader.sh ]; then K=/mnt/onboard/.adds/koreader/koreader.sh; else K=/mnt/onboard/koreader/koreader.sh; fi; if [ ! -f '\(esc)' ]; then echo \"not found: \(esc)\"; exit 1; fi; nohup \"$K\" '\(esc)' >/tmp/koreader-open.log 2>&1 & sleep 1; ps | grep -E \"koreader|reader.lua\" | head -n 5; echo \"launched: \(esc)\""
            let open = sshSync(ip: ip, remoteCmd: openCmd, timeout: 10)
            if open.code != 0 {
                if open.output.contains("not found:") {
                    // Stale-cache ghost: the index matched a path that is no
                    // longer on the device. Offer sync instead of dead-ending.
                    return .missingOnKobo(predicted: chosen, message: "“\(titleTrim)” was not found on this device.")
                }
                return .failed("Failed to open on Kobo: \(open.output)")
            }
            return .opened(open.output)
        }.value
    }

    private static func sshSync(ip: String, remoteCmd: String, timeout: Int) -> (output: String, code: Int32) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
        let (sshArgs, sshEnv, askpassCleanup) = KoboSync.koboSSHInvocation(ip: ip, timeout: timeout, password: KoboSync.koboSSHPassword(forIP: ip))
        defer { askpassCleanup() }
        p.arguments = sshArgs
        if !sshEnv.isEmpty {
            var e = ProcessInfo.processInfo.environment
            e.merge(sshEnv) { _, new in new }
            p.environment = e
        }
        let input = Pipe()
        let tmpOut = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".koboout")
        FileManager.default.createFile(atPath: tmpOut.path, contents: nil)
        guard let outHandle = try? FileHandle(forWritingTo: tmpOut) else {
            return ("ssh temp file failed", -1)
        }
        p.standardOutput = outHandle; p.standardError = outHandle; p.standardInput = input
        do {
            try p.run()
            // Heredoc via stdin (required for Dropbear user-rc; one-shot "ssh host cmd" returns empty)
            if let d = (remoteCmd + "\n").data(using: .utf8) {
                input.fileHandleForWriting.write(d)
            }
            input.fileHandleForWriting.closeFile()
            // Enforce timeout locally as well — file backend avoids pipe deadlock on 500KB find output
            let deadline = Date().addingTimeInterval(Double(timeout + 2))
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

// Jocala Catalog — native SwiftUI catalog, SMB/local Calibre link, data in ~/Library/Application Support/com.jocala.Catalog
// Mirrors iOS Browse + UnifiedConfigure (SMB + Calibre) without reading

struct CatalogBook: Identifiable {
    let id: Int64
    let title: String
    let author: String
    let path: String
    let hasCover: Bool
    let coverHash: String?
}

@MainActor
class CatalogStore: ObservableObject {
    @Published var books: [CatalogBook] = []
    @Published var thumbnails: [String: NSImage] = [:]
    @Published var query = ""
    @Published var isLoading = true
    @Published var koboStatus: String = ""
    @Published var koboError: String? = nil
    @Published var koboMissing: KoboMissing? = nil
    @Published var totalCount: Int = 0
    // iOS BrowseView parity: gallery root + ordering + drill-in state.
    // sortOrder defaults to .author, which reproduces the historical
    // fetchBooks order (author_sort, title).
    @Published var browseMode: BrowseMode = .books
    @Published var sortOrder: CatalogSortOrder = .author
    @Published var authors: [AuthorSummary] = []
    @Published var series: [SeriesSummary] = []
    @Published var tags: [TagSummary] = []
    @Published var drilledKind: BrowseMode? = nil
    @Published var drilledTitle: String? = nil
    @Published var drilledBooks: [AuthorBook] = []
    @Published var isDrilling = false
    /// Sort options valid for the current mode/drill (mirrors iOS
    /// availableSortOrders, minus Current which needs reading_progress).
    func availableSortOrders() -> [CatalogSortOrder] {
        if let kind = drilledKind {
            switch kind {
            case .author, .series:
                return [.az, .za, .date, .oldest]
            case .tags, .books:
                return [.author, .az, .za, .date, .oldest]
            }
        }
        switch browseMode {
        case .books: return [.author, .az, .za, .date, .oldest]
        case .author: return [.az, .za]
        case .series: return [.author, .az, .za]
        case .tags: return [.az, .za]
        }
    }
    func clampSortOrder() {
        let avail = availableSortOrders()
        if !avail.contains(sortOrder) { sortOrder = avail.first ?? .az }
    }
    func modeCountText() -> String {
        if drilledKind != nil { return "\(drilledBooks.count) books" }
        switch browseMode {
        case .books: return "\(totalCount) books"
        case .author: return "\(authors.count) authors"
        case .series: return "\(series.count) series"
        case .tags: return "\(tags.count) tags"
        }
    }
    /// Drilled lists arrive in catalogue order; apply the toolbar sort here
    /// (mirrors iOS sortedGalleryBooks, minus Current). Calibre timestamps
    /// sort lexicographically; books without one (tag drill) fall back to title.
    func drilledBooksSorted() -> [AuthorBook] {
        switch sortOrder {
        case .author:
            return drilledBooks.sorted {
                ($0.authorSort.isEmpty ? $0.author : $0.authorSort).localizedCaseInsensitiveCompare(
                    $1.authorSort.isEmpty ? $1.author : $1.authorSort) == .orderedAscending
            }
        case .az:
            return drilledBooks.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
        case .za:
            return drilledBooks.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedDescending }
        case .date:
            return drilledBooks.sorted {
                if $0.timestamp != $1.timestamp {
                    if $0.timestamp.isEmpty { return false }
                    if $1.timestamp.isEmpty { return true }
                    return $0.timestamp > $1.timestamp
                }
                return $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
            }
        case .oldest:
            return drilledBooks.sorted {
                if $0.timestamp != $1.timestamp {
                    if $0.timestamp.isEmpty { return false }
                    if $1.timestamp.isEmpty { return true }
                    return $0.timestamp < $1.timestamp
                }
                return $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
            }
        }
    }
    private let thumbnailThrottler = ThumbnailThrottler()
    private var inFlight: Set<String> = []
    var baseURL: URL { CatalogPaths.base }
    /// Set when the single SMB Calibre database cannot be read — drives the error dialog.
    @Published var dbError: String? = nil
    @Published var showHandoffPrompt = false
    @Published var koboHandoffPendingBook: CatalogBook? = nil
    func load() {
        isLoading = true
        // A fresh mode load exits any drill-in (mirrors iOS Back-on-switch).
        drilledKind = nil; drilledTitle = nil; drilledBooks = []
        let mode = browseMode
        let order = sortOrder
        let desc = order == .za || order == .oldest
        let byAuthor = order == .author
        let byDate = order == .date || order == .oldest
        Task.detached(priority: .userInitiated) { [query] in
            do {
                switch mode {
                case .books:
                    let books = try await SmbCatalogDB.shared.fetchBooks(search: query, sortDescending: desc, sortByAuthor: byAuthor, sortByDate: byDate)
                    let total = try await SmbCatalogDB.shared.fetchCount(search: query)
                    ReaderLog.shared.i("CatalogStore", "load done mode=books query='\(query)' order=\(order.rawValue) books=\(books.count) total=\(total)")
                    await MainActor.run {
                        self.books = books; self.totalCount = total
                        self.authors = []; self.series = []; self.tags = []
                        self.isLoading = false; self.dbError = nil
                    }
                case .author:
                    let authors = try await SmbCatalogDB.shared.allAuthors(sortDescending: desc)
                    ReaderLog.shared.i("CatalogStore", "load done mode=author order=\(order.rawValue) authors=\(authors.count)")
                    await MainActor.run {
                        self.authors = authors
                        self.totalCount = authors.reduce(0) { $0 + $1.bookCount }
                        self.books = []; self.series = []; self.tags = []
                        self.isLoading = false; self.dbError = nil
                    }
                case .series:
                    let series = try await SmbCatalogDB.shared.allSeries(sortDescending: desc, sortByAuthor: byAuthor)
                    ReaderLog.shared.i("CatalogStore", "load done mode=series order=\(order.rawValue) series=\(series.count)")
                    await MainActor.run {
                        self.series = series
                        self.totalCount = series.reduce(0) { $0 + $1.bookCount }
                        self.books = []; self.authors = []; self.tags = []
                        self.isLoading = false; self.dbError = nil
                    }
                case .tags:
                    let tags = try await SmbCatalogDB.shared.allTags(sortDescending: desc)
                    ReaderLog.shared.i("CatalogStore", "load done mode=tags order=\(order.rawValue) tags=\(tags.count)")
                    await MainActor.run {
                        self.tags = tags
                        self.totalCount = tags.reduce(0) { $0 + $1.bookCount }
                        self.books = []; self.authors = []; self.series = []
                        self.isLoading = false; self.dbError = nil
                    }
                }
            } catch {
                ReaderLog.shared.e("CatalogStore", "load failed mode=\(mode.rawValue) \(error.localizedDescription)")
                await MainActor.run {
                    self.books = []; self.totalCount = 0; self.isLoading = false
                    self.dbError = error.localizedDescription
                }
            }
        }
    }
    func exitDrill() {
        drilledKind = nil; drilledTitle = nil; drilledBooks = []
    }
    func drillAuthor(_ author: AuthorSummary) {
        drilledKind = .author; drilledTitle = author.name; drilledBooks = []
        clampSortOrder(); isDrilling = true
        Task.detached(priority: .userInitiated) {
            do {
                let books = try await SmbCatalogDB.shared.booksByAuthor(id: author.id)
                await MainActor.run { self.drilledBooks = books; self.isDrilling = false }
            } catch {
                ReaderLog.shared.e("CatalogStore", "drillAuthor failed \(error.localizedDescription)")
                await MainActor.run { self.isDrilling = false; self.dbError = error.localizedDescription }
            }
        }
    }
    func drillSeries(_ s: SeriesSummary) {
        drilledKind = .series; drilledTitle = s.name; drilledBooks = []
        clampSortOrder(); isDrilling = true
        Task.detached(priority: .userInitiated) {
            do {
                let books = try await SmbCatalogDB.shared.booksBySeries(id: s.id)
                await MainActor.run { self.drilledBooks = books; self.isDrilling = false }
            } catch {
                ReaderLog.shared.e("CatalogStore", "drillSeries failed \(error.localizedDescription)")
                await MainActor.run { self.isDrilling = false; self.dbError = error.localizedDescription }
            }
        }
    }
    func drillTag(_ tag: TagSummary) {
        drilledKind = .tags; drilledTitle = tag.name; drilledBooks = []
        clampSortOrder(); isDrilling = true
        Task.detached(priority: .userInitiated) {
            do {
                let found = try await SmbCatalogDB.shared.searchBooks(tag: tag.name)
                let mapped = found.map {
                    AuthorBook(id: $0.id, title: $0.title, author: $0.author, path: $0.path,
                               coverHash: "", timestamp: "", root_folder: "", authorSort: $0.authorSort)
                }
                await MainActor.run { self.drilledBooks = mapped; self.isDrilling = false }
            } catch {
                ReaderLog.shared.e("CatalogStore", "drillTag failed \(error.localizedDescription)")
                await MainActor.run { self.isDrilling = false; self.dbError = error.localizedDescription }
            }
        }
    }

    func fetchTotalCount(search q: String) async -> Int {
        do { return try await SmbCatalogDB.shared.fetchCount(search: q) }
        catch {
            await MainActor.run { self.dbError = error.localizedDescription }
            return 0
        }
    }
    func fetchBooks(search q: String) async -> [CatalogBook] {
        do { return try await SmbCatalogDB.shared.fetchBooks(search: q) }
        catch {
            await MainActor.run { self.dbError = error.localizedDescription }
            return []
        }
    }
    // Legacy sync file-DB fetchers removed — single SMB database only.
    func loadThumbnail(for book: CatalogBook) async {
        if thumbnails[book.path] != nil { return }
        if !book.hasCover { return }
        if inFlight.contains(book.path) { return }
        inFlight.insert(book.path)
        guard await thumbnailThrottler.wait() else {
            inFlight.remove(book.path)
            return
        }
        if Task.isCancelled {
            inFlight.remove(book.path)
            Task { await self.thumbnailThrottler.signal() }
            return
        }
        defer {
            inFlight.remove(book.path)
            Task { await self.thumbnailThrottler.signal() }
        }
        // Fast path: covers/<hash>.jpg (local SSD, no SMB) — throttled to 6 concurrent
        if let hash = book.coverHash, !hash.isEmpty {
            let coversDir = CatalogPaths.coversDir
            let hashPath = coversDir.appendingPathComponent("\(hash).jpg").path
            if let data = try? Data(contentsOf: URL(fileURLWithPath: hashPath)), let img = NSImage(data: data) {
                await MainActor.run { self.thumbnails[book.path] = img }
                return
            }
        }
        // Covers: local covers/ hash cache, else live SMB cover.jpg download
        // (ThumbnailService parses host/share/remotePath from the smb:// URL).
        // No /Volumes mount reads, no local Books tree.
        if let img = await ThumbnailService.thumbnail(for: book.path, coverHash: book.coverHash) {
            await MainActor.run { self.thumbnails[book.path] = img }
        }
    }
    nonisolated func coverImageSync(for book: CatalogBook) -> NSImage? {
        // Check covers cache first (no per-book log — was 6742 file appends causing refresh storm)
        let baseURL = CatalogPaths.base
        let coversDir = baseURL.appendingPathComponent("covers")
        if let hash = book.coverHash, !hash.isEmpty {
            let hashPath = coversDir.appendingPathComponent("\(hash).jpg").path
            if FileManager.default.fileExists(atPath: hashPath),
               let data = try? Data(contentsOf: URL(fileURLWithPath: hashPath)),
               let img = NSImage(data: data) {
                return img
            }
        }
        // Sync cover lookup: covers/ hash cache + loaded thumbnails only.
        // Live SMB fetch happens in loadThumbnail (async). No mount reads.
        return nil
    }
    func coverImage(for book: CatalogBook) -> NSImage? {
        // Legacy sync path — now delegates to coverImageSync + thumbnails cache
        if let cached = thumbnails[book.path] { return cached }
        return coverImageSync(for: book)
    }

    func fetchDetail(for book: CatalogBook) async -> BookDetail? {
        do { return try await SmbCatalogDB.shared.bookDetail(for: book.id) }
        catch {
            ReaderLog.shared.e("CatalogStore", "detail failed id=\(book.id) \(error.localizedDescription)")
            return nil
        }
    }
}

enum LibraryViewMode: String, CaseIterable { case grid, list }

// iOS BrowseView parity: the <Books> dropdown switches the gallery root,
// the <Sort> dropdown reorders it. No Current on macOS — a live Calibre
// metadata.db has no reading_progress table, so Current has no meaning here.
enum BrowseMode: String, CaseIterable {
    case books = "Books"
    case author = "Author"
    case series = "Series"
    case tags = "Tags"
}

enum CatalogSortOrder: String, CaseIterable {
    case author = "Author"
    case az = "A–Z"
    case za = "Z–A"
    case date = "Newest"
    case oldest = "Oldest"
}

struct ContentView: View {
    @StateObject var store = CatalogStore()
    @State var showSettings = false
    @State var showSearchForm = false
    @State var isSearchMode = false
    @State var searchResults: [SearchResult] = []
    @State var seriesResults: [SeriesSummary] = []
    @State var seriesResultsTagName: String = ""
    @State var searchThumbnails: [String: NSImage] = [:]
    @State var koboLaunching: String? = nil
    @State var selectedBook: CatalogBook? = nil
    @State var selectedDetail: BookDetail? = nil
    @State var isLoadingDetail = false
    // Search selection for detail sheet (SearchResult -> CatalogBook conversion)
    @State var searchSelectedBook: CatalogBook? = nil
    @State var viewMode: LibraryViewMode = .grid
    var body: some View {
        NavigationStack {
            VStack(spacing:0) {
                HStack(spacing:8) {
                    Button {
                        showSearchForm = true
                    } label: {
                        Image(systemName: "magnifyingglass")
                    }
                    .buttonStyle(.bordered)
                    .help("Search Books")
                    Button("Reload") { exitSearchMode(); store.load() }
                    Button("Settings") { showSettings = true }
                    Divider().frame(height: 20)
                    // iOS BrowseView parity: <Books> gallery root + <Sort> order.
                    Picker("", selection: $store.browseMode) {
                        ForEach(BrowseMode.allCases, id: \.self) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                    .pickerStyle(.menu)
                    .fixedSize()
                    .help("Browse: Books, Author, Series, or Tags")
                    Picker("", selection: $store.sortOrder) {
                        ForEach(store.availableSortOrders(), id: \.self) { order in
                            Text(order.rawValue).tag(order)
                        }
                    }
                    .pickerStyle(.menu)
                    .fixedSize()
                    .help("Sort order")
                    Divider().frame(height: 20)
                    Picker("", selection: $viewMode) {
                        Image(systemName: "square.grid.2x2").tag(LibraryViewMode.grid)
                        Image(systemName: "list.bullet").tag(LibraryViewMode.list)
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 80)
                    .help("List/Grid")
                    if isSearchMode {
                        Divider().frame(height: 20)
                        Button("Library") { exitSearchMode() }
                            .buttonStyle(.borderedProminent)
                            .help("Back to library")
                        Text("\(searchResults.count + seriesResults.count) results").foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !store.koboStatus.isEmpty {
                        Text(store.koboStatus).font(.caption).foregroundStyle(store.koboStatus.hasPrefix("Opened") ? .green : .orange).lineLimit(1).truncationMode(.tail).frame(maxWidth: 320, alignment: .trailing)
                    } else {
                        Text(store.modeCountText()).foregroundStyle(.secondary)
                    }
                }.padding(2).background(.bar)
                if isSearchMode {
                    searchResultsContent
                } else if store.isLoading { ProgressView("Loading…").frame(maxWidth:.infinity, maxHeight:.infinity) }
                else if store.drilledKind != nil {
                    drilledContent
                } else if isModeEmpty {
                    VStack(spacing:12){
                        Text(modeEmptyTitle).font(.title3).foregroundStyle(.secondary)
                        Text("Set the SMB server and Calibre path in Settings → SMB").font(.caption).foregroundStyle(.secondary)
                        Button("Open Settings") { showSettings = true }
                    }.frame(maxWidth:.infinity,maxHeight:.infinity)
                } else {
                    modeContent
                }
            }.navigationTitle("Jocala Catalog").onAppear{ store.load() }
            .onChange(of: store.browseMode) { _, _ in
                store.clampSortOrder()
                store.exitDrill()
                exitSearchMode()
                store.load()
            }
            .onChange(of: store.sortOrder) { _, _ in
                // Drilled lists re-sort in memory via drilledBooksSorted;
                // root modes re-query with the new ORDER BY.
                if store.drilledKind == nil && !isSearchMode { store.load() }
            }
            .sheet(isPresented: $showSettings) { SettingsView().frame(width: 620).fixedSize(horizontal: false, vertical: true).background(Color.red.opacity(0.02)) }
            .sheet(isPresented: $showSearchForm) {
                SearchFormSheet(onSearch: { results in
                    enterSearchMode(with: results)
                }, onSearchSeries: { series, tagName in
                    enterSeriesMode(with: series, tagName: tagName)
                })
            }
            .sheet(item: $selectedBook) { book in
                BookInfoView(book: book, detail: selectedDetail, isLoading: isLoadingDetail, store: store, onRead: {
                    Task {
                        let out = await KoboLauncher.open(book: book, store: store)
                        if store.koboError == nil {
                            selectedBook = nil
                        }
                        _ = out
                    }
                }, onClose: { selectedBook = nil })
            }
            .sheet(item: $searchSelectedBook) { book in
                BookInfoView(book: book, detail: selectedDetail, isLoading: isLoadingDetail, store: store, onRead: {
                    Task {
                        let out = await KoboLauncher.open(book: book, store: store)
                        if store.koboError == nil { searchSelectedBook = nil }
                        _ = out
                    }
                }, onClose: { searchSelectedBook = nil })
            }
            .alert("Kobo", isPresented: Binding(get: { store.koboError != nil }, set: { if !$0 { store.koboError = nil } })) {
                Button("OK") { store.koboError = nil }
            } message: { Text(store.koboError ?? "") }
            .alert("Not on Kobo", isPresented: Binding(get: { store.koboMissing != nil }, set: { if !$0 { store.koboMissing = nil } })) {
                Button("Sync & Open") {
                    if let m = store.koboMissing { Task { await KoboLauncher.syncAndOpen(missing: m, store: store) } }
                }
                Button("Cancel", role: .cancel) { store.koboMissing = nil }
            } message: {
                if let m = store.koboMissing {
                    Text("\(m.message)\n\nSync this \(m.mbText) book to Kobo via WiFi?")
                }
            }
            .alert("Calibre Database", isPresented: Binding(get: { store.dbError != nil }, set: { if !$0 { store.dbError = nil } })) {
                Button("Open Settings") { store.dbError = nil; showSettings = true }
                Button("Retry") { store.dbError = nil; Task { await SmbCatalogDB.shared.invalidate() }; store.load() }
                Button("OK", role: .cancel) { store.dbError = nil }
            } message: { Text(store.dbError ?? "") }
            .alert("Stop KOReader from stacking instances?", isPresented: $store.showHandoffPrompt) {
                Button("Install") {
                    Task {
                        await KoboLauncher.installHandoff(store: store)
                        UserDefaults.standard.set(true, forKey: "kobo_handoff_prompt_done")
                        if let b = store.koboHandoffPendingBook {
                            let pending = b
                            store.koboHandoffPendingBook = nil
                            store.showHandoffPrompt = false
                            _ = await KoboLauncher.open(book: pending, store: store)
                        } else {
                            store.showHandoffPrompt = false
                        }
                    }
                }
                Button("Not Now", role: .cancel) {
                    UserDefaults.standard.set(true, forKey: "kobo_handoff_prompt_done")
                    if let b = store.koboHandoffPendingBook {
                        let pending = b
                        store.koboHandoffPendingBook = nil
                        store.showHandoffPrompt = false
                        Task { _ = await KoboLauncher.open(book: pending, store: store) }
                    } else {
                        store.showHandoffPrompt = false
                    }
                }
            } message: { Text("Install a tiny, reversible fix?") }
            .onReceive(NotificationCenter.default.publisher(for: .importedFoldersChanged).debounce(for: .milliseconds(500), scheduler: RunLoop.main)) { _ in if !isSearchMode { store.load() } }
        }.frame(minWidth:892, maxWidth:892, minHeight:794, maxHeight:794)
    }

    @ViewBuilder
    private var searchResultsContent: some View {
        Group {
            if !seriesResults.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("\(seriesResults.count) series with \"\(seriesResultsTagName)\"")
                        .font(.caption).foregroundStyle(.secondary).padding(.horizontal, 12).padding(.top, 8)
                    ScrollView {
                        if viewMode == .list {
                            LazyVStack(alignment: .leading, spacing: 0) {
                                ForEach(seriesResults) { s in
                                    seriesSearchListRow(s)
                                    Divider().padding(.leading, 64)
                                }
                            }.padding(.vertical, 8)
                        } else {
                        HStack(spacing: 0) {
                            Spacer(minLength: 0)
                            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
                                ForEach(seriesResults) { s in
                                VStack(spacing: 6) {
                                    ZStack {
                                        RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.12))
                                        if let path = s.firstBookPath, let img = searchThumbnails[path] ?? store.thumbnails[path] {
                                            Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 187, height: 240)
                                        } else if let path = s.firstBookPath, !path.isEmpty {
                                            Image(systemName: "books.vertical").font(.title2).foregroundStyle(.secondary).opacity(0.6)
                                                .task(id: path) {
                                                    if searchThumbnails[path] != nil { return }
                                                    if let img = await searchThumbnail(for: path) {
                                                        await MainActor.run { searchThumbnails[path] = img }
                                                    }
                                                }
                                        } else {
                                            Image(systemName: "books.vertical").font(.title2).foregroundStyle(.secondary)
                                        }
                                    }.frame(width: 187, height: 240).clipShape(RoundedRectangle(cornerRadius: 6))
                                    Text(s.name).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
                                    Text("\(s.bookCount) books").font(.caption2).foregroundStyle(.secondary)
                                }
                                .contentShape(Rectangle())
                                .onHover { inside in
                                    if inside {
                                        NSCursor.pointingHand.push()
                                    } else {
                                        NSCursor.pop()
                                    }
                                }
                                .onTapGesture { drillSearchSeries(s) }
                            }
                        }
                        .frame(width: 784)
                        Spacer(minLength: 0)
                    }
                    .padding(2)
                        }
                    }
                }
            } else if searchResults.isEmpty {
                VStack {
                    Spacer()
                    VStack(spacing: 12) {
                        Image(systemName: "magnifyingglass").font(.largeTitle).foregroundStyle(.secondary)
                        Text("No Results").font(.title3)
                        Text("No books found for your search").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    if viewMode == .list {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(searchResults) { item in
                                searchResultListRow(item)
                                Divider().padding(.leading, 64)
                            }
                        }.padding(.vertical, 8)
                    } else {
                    HStack(spacing: 0) {
                        Spacer(minLength: 0)
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
                            ForEach(searchResults) { item in
                                searchResultGridItem(item)
                            }
                        }
                        .frame(width: 784)
                        Spacer(minLength: 0)
                    }
                    .padding(2)
                    }
                }
            }
        }
    }

    // MARK: - Browse modes (iOS BrowseView parity)

    private var isModeEmpty: Bool {
        switch store.browseMode {
        case .books: return store.books.isEmpty
        case .author: return store.authors.isEmpty
        case .series: return store.series.isEmpty
        case .tags: return store.tags.isEmpty
        }
    }
    private var modeEmptyTitle: String {
        switch store.browseMode {
        case .books: return "No books"
        case .author: return "No authors"
        case .series: return "No series"
        case .tags: return "No tags"
        }
    }
    private func openBook(_ book: CatalogBook) {
        selectedBook = book
        selectedDetail = nil
        isLoadingDetail = true
        Task { selectedDetail = await store.fetchDetail(for: book); isLoadingDetail = false }
    }
    /// Capped 784pt grid (= 4x187 + 3x12), centered: gaps stay exactly 12pt
    /// at any window width; margins absorb growth.
    private func centeredGrid<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 0) {
            Spacer(minLength: 0)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
                content()
            }
            .frame(width: 784)
            Spacer(minLength: 0)
        }.padding(2)
    }
    @ViewBuilder
    private var modeContent: some View {
        switch store.browseMode {
        case .books: booksContent
        case .author: authorsContent
        case .series: seriesContent
        case .tags: tagsContent
        }
    }
    @ViewBuilder
    private var booksContent: some View {
        ScrollView {
            if viewMode == .grid {
                centeredGrid {
                    ForEach(store.books) { book in
                        BookCell(book: book, store: store, onOpen: { openBook(book) })
                            .task { await store.loadThumbnail(for: book) }
                    }
                }
            } else {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(store.books) { book in
                        HStack(spacing: 12) {
                            ZStack {
                                RoundedRectangle(cornerRadius: 4).fill(Color.secondary.opacity(0.12))
                                if let img = store.thumbnails[book.path] ?? store.coverImageSync(for: book) {
                                    Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 107, height: 160)
                                } else {
                                    Image(systemName: "book.closed").foregroundStyle(.secondary)
                                }
                            }.frame(width: 107, height: 160).clipShape(RoundedRectangle(cornerRadius: 4))
                            VStack(alignment: .leading, spacing: 2) {
                                Text(book.title).font(.body).lineLimit(1)
                                Text(book.author).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                Text(book.path).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer()
                            Button("Details") { openBook(book) }.controlSize(.small)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 6)
                        .contentShape(Rectangle())
                        .onHover { inside in
                            if inside {
                                NSCursor.pointingHand.push()
                            } else {
                                NSCursor.pop()
                            }
                        }
                        .onTapGesture { openBook(book) }
                        .task(id: book.path) { await store.loadThumbnail(for: book) }
                        Divider().padding(.leading, 64)
                    }
                }.padding(.vertical, 8)
            }
        }
    }
    @ViewBuilder
    private var authorsContent: some View {
        ScrollView {
            centeredGrid {
                ForEach(store.authors) { author in
                    authorTile(author)
                }
            }
        }
    }
    @ViewBuilder
    private var seriesContent: some View {
        ScrollView {
            centeredGrid {
                ForEach(store.series) { s in
                    seriesTile(s)
                }
            }
        }
    }
    @ViewBuilder
    private var tagsContent: some View {
        ScrollView {
            if viewMode == .grid {
                centeredGrid {
                    ForEach(store.tags) { tag in
                        VStack(spacing: 4) {
                            // iOS tagGridItem parity: no gray box — big manila tag.
                            Image(systemName: "tag.fill")
                                .font(.system(size: 64))
                                .foregroundStyle(Color(red: 0.784, green: 0.663, blue: 0.431))
                                .frame(width: 187, height: 180)
                            Text(tag.name).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
                            Text("\(tag.bookCount) books").font(.caption2).foregroundStyle(.secondary)
                        }
                        .contentShape(Rectangle())
                        .onHover { inside in
                            if inside {
                                NSCursor.pointingHand.push()
                            } else {
                                NSCursor.pop()
                            }
                        }
                        .onTapGesture { store.drillTag(tag) }
                    }
                }
            } else {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(store.tags) { tag in
                        HStack(spacing: 12) {
                            Image(systemName: "tag.fill")
                                .font(.system(size: 24))
                                .foregroundStyle(Color(red: 0.784, green: 0.663, blue: 0.431))
                                .frame(width: 32, height: 32)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(tag.name).font(.body).lineLimit(1)
                                Text("\(tag.bookCount) books").font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .contentShape(Rectangle())
                        .onHover { inside in
                            if inside {
                                NSCursor.pointingHand.push()
                            } else {
                                NSCursor.pop()
                            }
                        }
                        .onTapGesture { store.drillTag(tag) }
                        Divider().padding(.leading, 56)
                    }
                }.padding(.vertical, 8)
            }
        }
    }
    private func authorTile(_ author: AuthorSummary) -> some View {
        VStack(spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.12))
                if let path = author.firstBookPath, let img = store.thumbnails[path] {
                    Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 187, height: 240)
                } else if let path = author.firstBookPath, !path.isEmpty {
                    Image(systemName: "person.crop.rectangle").font(.title2).foregroundStyle(.secondary).opacity(0.6)
                        .task(id: path) { await tileThumbnail(for: path) }
                } else {
                    Image(systemName: "person").font(.title2).foregroundStyle(.secondary)
                }
            }.frame(width: 187, height: 240).clipShape(RoundedRectangle(cornerRadius: 6))
            Text(author.name).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
            Text("\(author.bookCount) books").font(.caption2).foregroundStyle(.secondary)
        }
        .help("\(author.name)\n\(author.bookCount) books\nClick to browse")
        .contentShape(Rectangle())
        .onHover { inside in
            if inside {
                NSCursor.pointingHand.push()
            } else {
                NSCursor.pop()
            }
        }
        .onTapGesture { store.drillAuthor(author) }
    }
    private func seriesTile(_ s: SeriesSummary) -> some View {
        VStack(spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.12))
                if let path = s.firstBookPath, let img = store.thumbnails[path] {
                    Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 187, height: 240)
                } else if let path = s.firstBookPath, !path.isEmpty {
                    Image(systemName: "books.vertical").font(.title2).foregroundStyle(.secondary).opacity(0.6)
                        .task(id: path) { await tileThumbnail(for: path) }
                } else {
                    Image(systemName: "books.vertical").font(.title2).foregroundStyle(.secondary)
                }
            }.frame(width: 187, height: 240).clipShape(RoundedRectangle(cornerRadius: 6))
            Text(s.name).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
            Text("\(s.bookCount) books").font(.caption2).foregroundStyle(.secondary)
        }
        .help("\(s.name)\n\(s.bookCount) books\nClick to browse")
        .contentShape(Rectangle())
        .onHover { inside in
            if inside {
                NSCursor.pointingHand.push()
            } else {
                NSCursor.pop()
            }
        }
        .onTapGesture { store.drillSeries(s) }
    }
    private func tileThumbnail(for path: String) async {
        if store.thumbnails[path] != nil { return }
        // smb:// and local paths both handled; covers come from the covers/
        // cache or a live SMB cover.jpg download. No /Volumes mount reads.
        if path.hasPrefix("http://") || path.hasPrefix("https://") { return }
        if let img = await ThumbnailService.thumbnail(for: path, coverHash: nil) {
            await MainActor.run { store.thumbnails[path] = img }
        }
    }
    @ViewBuilder
    private var drilledContent: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Button {
                    store.exitDrill()
                } label: {
                    Image(systemName: "chevron.left")
                    Text("Back")
                }
                .buttonStyle(.bordered)
                Text(store.drilledTitle ?? "").font(.headline).lineLimit(1).truncationMode(.tail)
                Spacer()
                if store.isDrilling {
                    ProgressView().scaleEffect(0.7)
                } else {
                    Text(store.modeCountText()).font(.caption).foregroundStyle(.secondary)
                }
            }.padding(8)
            ScrollView {
                centeredGrid {
                    ForEach(drilledCatalogBooks()) { book in
                        BookCell(book: book, store: store, onOpen: { openBook(book) })
                            .task { await store.loadThumbnail(for: book) }
                    }
                }
            }
        }
    }
    private func drilledCatalogBooks() -> [CatalogBook] {
        // Drilled rows are catalogue books: covers resolve via the SMB
        // cover.jpg next to the book (hasCover true, like search results).
        store.drilledBooksSorted().map {
            CatalogBook(id: $0.id, title: $0.title, author: $0.author,
                        path: $0.path, hasCover: true, coverHash: nil)
        }
    }

    private func searchThumbnail(for path: String) async -> NSImage? {
        // Single-database rule: covers come from the covers/ cache or a live
        // SMB cover.jpg download. No /Volumes mount reads.
        if let img = await ThumbnailService.thumbnail(for: path, coverHash: nil) {
            return img
        }
        // smb:// paths are not local files — do not fall through to fileURL handling.
        if path.hasPrefix("smb://") { return nil }
        if path.hasPrefix("http://") || path.hasPrefix("https://") { return nil }
        let url = URL(fileURLWithPath: path)
        return await ThumbnailService.thumbnail(for: url)
    }

    private func searchResultGridItem(_ item: SearchResult) -> some View {
        // Per-cell View (own @State image): the cell re-renders itself when
        // its cover arrives instead of depending on a parent body refresh.
        // (A shared-dict-only design left the last cells of a burst blank:
        // all writes landed, but those cells never re-evaluated.)
        SearchResultCell(
            item: item, store: store,
            cached: searchThumbnails[item.filePath] ?? store.thumbnails[item.filePath],
            onImage: { path, img in
                searchThumbnails[path] = img
                // BookInfoView reads store.thumbnails — mirror it there.
                store.thumbnails[path] = img
            },
            onSelect: selectSearchResult
        )
    }

    private func searchResultListRow(_ item: SearchResult) -> some View {
        SearchResultRow(
            item: item, store: store,
            cached: searchThumbnails[item.filePath] ?? store.thumbnails[item.filePath],
            onImage: { path, img in
                searchThumbnails[path] = img
                store.thumbnails[path] = img
            },
            onSelect: selectSearchResult
        )
    }

    private func seriesSearchListRow(_ s: SeriesSummary) -> some View {
        SeriesSearchRow(
            s: s, store: store,
            cached: s.firstBookPath.flatMap { searchThumbnails[$0] ?? store.thumbnails[$0] },
            load: { path in await searchThumbnail(for: path) },
            onImage: { path, img in
                searchThumbnails[path] = img
                store.thumbnails[path] = img
            },
            onDrill: drillSearchSeries
        )
    }

    private func selectSearchResult(_ tapped: SearchResult) {
        let book = CatalogBook(id: tapped.bookId ?? 0, title: tapped.title ?? tapped.fileName, author: tapped.author ?? "Unknown", path: tapped.filePath, hasCover: true, coverHash: nil)
        searchSelectedBook = book
        selectedDetail = nil
        isLoadingDetail = true
        Task { selectedDetail = await store.fetchDetail(for: book); isLoadingDetail = false }
        // Ensure the cover is in store.thumbnails for BookInfoView even if
        // the cell task hasn't finished yet.
        Task { await store.loadThumbnail(for: book) }
    }

    private func drillSearchSeries(_ s: SeriesSummary) {
        Task {
            do {
                let books = try await SmbCatalogDB.shared.booksBySeries(id: s.id)
                let results = books.map { b in SearchResult(filePath: b.path, fileName: URL(fileURLWithPath: b.path).lastPathComponent, bookId: b.id, title: b.title, author: b.author, series: s.name, tags: [], authorSort: b.authorSort) }
                await MainActor.run { searchResults = results; seriesResults = [] ; loadSearchThumbnails() }
            } catch {
                await MainActor.run { store.dbError = error.localizedDescription }
            }
        }
    }

    // Search result cell with its OWN cover state: the cell re-renders itself
    // when its image arrives instead of depending on a parent body refresh.
    struct SearchResultCell: View {
        let item: SearchResult
        let store: CatalogStore
        let cached: NSImage?
        let onImage: (String, NSImage) -> Void
        let onSelect: (SearchResult) -> Void
        @State private var img: NSImage?
        var body: some View {
            VStack(spacing: 6) {
                ZStack {
                    RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.12))
                    if let img = img ?? cached ?? store.thumbnails[item.filePath] {
                        Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 187, height: 240)
                    } else {
                        Image(systemName: "book.closed").font(.title2).foregroundStyle(.secondary).opacity(0.6)
                    }
                }.frame(width: 187, height: 240).clipShape(RoundedRectangle(cornerRadius: 6))
                    .task(id: item.filePath) {
                        if img != nil { return }
                        if let loaded = await ThumbnailService.thumbnail(for: item.filePath, coverHash: nil) {
                            img = loaded
                            await MainActor.run { onImage(item.filePath, loaded) }
                        }
                    }
                Text(item.title ?? item.fileName).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
                Text(item.author ?? "").font(.caption2).foregroundStyle(.secondary).lineLimit(1).frame(width: 187)
            }
            .contentShape(Rectangle())
            .onHover { inside in
                if inside {
                    NSCursor.pointingHand.push()
                } else {
                    NSCursor.pop()
                }
            }
            .onTapGesture { onSelect(item) }
        }
    }

    // Search list-mode row: horizontal layout mirroring the library Books
    // list rows (cover thumb + title/author/path + Details). Own @State
    // image like SearchResultCell so burst arrivals re-render the row.
    struct SearchResultRow: View {
        let item: SearchResult
        let store: CatalogStore
        let cached: NSImage?
        let onImage: (String, NSImage) -> Void
        let onSelect: (SearchResult) -> Void
        @State private var img: NSImage?
        var body: some View {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 4).fill(Color.secondary.opacity(0.12))
                    if let img = img ?? cached ?? store.thumbnails[item.filePath] {
                        Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 107, height: 160)
                    } else {
                        Image(systemName: "book.closed").foregroundStyle(.secondary)
                    }
                }.frame(width: 107, height: 160).clipShape(RoundedRectangle(cornerRadius: 4))
                    .task(id: item.filePath) {
                        if img != nil { return }
                        if let loaded = await ThumbnailService.thumbnail(for: item.filePath, coverHash: nil) {
                            img = loaded
                            await MainActor.run { onImage(item.filePath, loaded) }
                        }
                    }
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title ?? item.fileName).font(.body).lineLimit(1)
                    Text(item.author ?? "").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    Text(item.filePath).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer()
                Button("Details") { onSelect(item) }.controlSize(.small)
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
            .contentShape(Rectangle())
            .onHover { inside in
                if inside {
                    NSCursor.pointingHand.push()
                } else {
                    NSCursor.pop()
                }
            }
            .onTapGesture { onSelect(item) }
        }
    }

    // Series search list-mode row: cover thumb (or books icon) + name/count.
    // Loader is injected so the row stays a dumb view over the caller's
    // smb-guarded thumbnail path.
    struct SeriesSearchRow: View {
        let s: SeriesSummary
        let store: CatalogStore
        let cached: NSImage?
        let load: (String) async -> NSImage?
        let onImage: (String, NSImage) -> Void
        let onDrill: (SeriesSummary) -> Void
        @State private var img: NSImage?
        var body: some View {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 4).fill(Color.secondary.opacity(0.12))
                    if let path = s.firstBookPath, let img = img ?? cached ?? store.thumbnails[path] {
                        Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 107, height: 160)
                    } else {
                        Image(systemName: "books.vertical").foregroundStyle(.secondary)
                    }
                }.frame(width: 107, height: 160).clipShape(RoundedRectangle(cornerRadius: 4))
                    .task(id: s.firstBookPath) {
                        guard let path = s.firstBookPath, !path.isEmpty, img == nil else { return }
                        if let loaded = await load(path) {
                            img = loaded
                            await MainActor.run { onImage(path, loaded) }
                        }
                    }
                VStack(alignment: .leading, spacing: 2) {
                    Text(s.name).font(.body).lineLimit(1)
                    Text("\(s.bookCount) books").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .contentShape(Rectangle())
            .onHover { inside in
                if inside {
                    NSCursor.pointingHand.push()
                } else {
                    NSCursor.pop()
                }
            }
            .onTapGesture { onDrill(s) }
        }
    }

    private func enterSearchMode(with results: [SearchResult]) {
        isSearchMode = true
        searchResults = results
        seriesResults = []
        searchThumbnails = [:]
        loadSearchThumbnails()
    }
    private func enterSeriesMode(with series: [SeriesSummary], tagName: String) {
        isSearchMode = true
        seriesResults = series
        seriesResultsTagName = tagName
        searchResults = []
        searchThumbnails = [:]
    }
    private func exitSearchMode() {
        isSearchMode = false
        searchResults = []
        seriesResults = []
        searchThumbnails = [:]
    }
    private func loadSearchThumbnails() {
        // prewarm first screen already handled per-cell .task; no bulk
    }
}

struct ServerRowView: View {
    let s: SmbServer
    @Binding var servers: [SmbServer]
    @Binding var calibreLibs: [CalibreLibraryConfig]
    var onTest: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack{ Text(s.host).font(.headline); Spacer(); Text("\(s.shares.count) shares").font(.caption).foregroundStyle(.secondary) }
            Text("user=\(s.user) domain=\(s.domain) shares=\(s.shares.map{$0.name}.joined(separator:", "))").font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
        }
        .padding(6)
        .background(Color.secondary.opacity(0.08)).cornerRadius(6)
        .contextMenu {
            Button("Delete", role: .destructive){ servers.removeAll{$0.host==s.host}; SmbServer.save(servers); calibreLibs = CalibreManager.shared.libraries }
            Button("Test Connection"){ onTest() }
        }
    }
}

/// A Kobo eReader endpoint: IP plus optional SSH password, edited in place.
/// Passwords live in the app password store (KeychainHelper) — never
/// encoded to UserDefaults alongside the IPs.
struct KoboDevice: Identifiable, Codable, Equatable {
    var id = UUID()
    var ip: String
    var password: String = ""
    enum CodingKeys: String, CodingKey { case id, ip }
}

struct SettingsView: View {
    @Environment(\.dismiss) var dismiss
    @AppStorage("library_source") var librarySource = "smb"
    @State var smbServer = ""
    @State var smbShare = ""
    @State var smbUser = ""
    @State var smbPass = ""
    @State var smbDomain = ""
    @State var showPass = false
    @State var calibrePath = ""
    @State var localDir = ""
    @State var status = ""
    @State var isSyncing = false
    @State var smbConnPass: Bool? = nil
    @State var smbDbPass: Bool? = nil
    @AppStorage("kobo_ip") var koboIP: String = ""
    @State var koboDevices: [KoboDevice] = []
    @State var newKoboIP = ""
    @State private var lastSavedKoboPasswords: [String: String] = [:]
    @State private var revealedKoboPasswords: Set<UUID> = []
    @State private var koboTestResults: [String: String] = [:]
    @AppStorage("theme_preference") var themePreference = 0
    private let fieldWidth: CGFloat = 380 // ~50 chars visible, unlimited content

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    // Source — either-or: SMB or a local Calibre folder
                    GroupBox {
                        VStack(spacing: 10) {
                            HStack(spacing: 4) {
                                Text("Source").frame(width: 115, alignment: .trailing).font(.caption)
                                ForEach([("smb", "SMB"), ("local", "Local")], id: \.0) { tag, label in
                                    Button(label) { librarySource = tag }
                                        .font(.caption)
                                        .padding(.horizontal, 10)
                                        .padding(.vertical, 4)
                                        .background(librarySource == tag ? Color.accentColor : Color(nsColor: NSColor.controlBackgroundColor))
                                        .foregroundColor(librarySource == tag ? .white : .primary)
                                        .clipShape(Capsule())
                                        .buttonStyle(.plain)
                                }
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("Local Calibre Path").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("Choose a Calibre folder…", text: $localDir).textFieldStyle(.roundedBorder).autocorrectionDisabled().font(.system(.body, design: .monospaced)).frame(width: fieldWidth - 140).disabled(librarySource != "local")
                                Button("Browse") { browseLocal() }.controlSize(.small).disabled(librarySource != "local")
                                Button("Test") { Task { await testLocal() } }.controlSize(.small).disabled(librarySource != "local")
                                if status.contains("Local") {
                                    Text(status.contains("OK") ? "Pass" : "Fail")
                                        .font(.caption).fontWeight(.bold)
                                        .foregroundStyle(status.contains("OK") ? .green : .red)
                                        .padding(.horizontal, 6).padding(.vertical, 2)
                                        .background((status.contains("OK") ? Color.green : Color.red).opacity(0.12))
                                        .cornerRadius(4)
                                }
                                Spacer()
                            }
                        }
                    } label: { Label("Library Source", systemImage: "externaldrive").font(.headline) }

                    // SMB
                    GroupBox {
                        VStack(spacing: 10) {
                            HStack(spacing: 8) {
                                Text("SMB Server").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("hostname or IP", text: $smbServer).textFieldStyle(.roundedBorder).autocorrectionDisabled().font(.system(.body, design: .monospaced)).frame(width: fieldWidth - 110)
                                Button("Test SMB") { Task { await testSMBConnection() } }.controlSize(.small).frame(width: 90)
                                if let pass = smbConnPass {
                                    Text(pass ? "Pass" : "Fail")
                                        .font(.caption).fontWeight(.bold)
                                        .foregroundStyle(pass ? .green : .red)
                                        .padding(.horizontal, 6).padding(.vertical, 2)
                                        .background((pass ? Color.green : Color.red).opacity(0.12))
                                        .cornerRadius(4)
                                }
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("Share").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("share name", text: $smbShare).textFieldStyle(.roundedBorder).autocorrectionDisabled().frame(width: fieldWidth - 110)
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("SMB Calibre Path").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("folder of metadata.db, e.g. calibre/", text: $calibrePath).textFieldStyle(.roundedBorder).autocorrectionDisabled().frame(width: fieldWidth - 110)
                                Button("Test Calibre") { Task { await testSMBDatabase() } }.controlSize(.small).frame(width: 90)
                                if let pass = smbDbPass {
                                    Text(pass ? "Pass" : "Fail")
                                        .font(.caption).fontWeight(.bold)
                                        .foregroundStyle(pass ? .green : .red)
                                        .padding(.horizontal, 6).padding(.vertical, 2)
                                        .background((pass ? Color.green : Color.red).opacity(0.12))
                                        .cornerRadius(4)
                                }
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("User").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("username", text: $smbUser).textFieldStyle(.roundedBorder).autocorrectionDisabled().frame(width: fieldWidth)
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("Password").frame(width: 115, alignment: .trailing).font(.caption)
                                Group {
                                    if showPass { TextField("Required", text: $smbPass) } else { SecureField("Required", text: $smbPass) }
                                }.textFieldStyle(.roundedBorder).frame(width: fieldWidth)
                                Button(showPass ? "Hide" : "Show") { showPass.toggle() }.controlSize(.small)
                                Spacer()
                            }
                            HStack(spacing: 8) {
                                Text("Domain").frame(width: 115, alignment: .trailing).font(.caption)
                                TextField("optional", text: $smbDomain).textFieldStyle(.roundedBorder).autocorrectionDisabled().frame(width: fieldWidth)
                                Spacer()
                            }
                        }
                    } label: { Label("SMB", systemImage: "network").font(.headline) }
                    .disabled(librarySource == "local")
                    .opacity(librarySource == "local" ? 0.5 : 1)

                    // Kobo IPs
                    GroupBox {
                        VStack(alignment: .leading, spacing: 8) {
                            ForEach($koboDevices) { $device in
                                HStack(spacing: 8) {
                                    Button {
                                        koboIP = device.ip
                                        saveKoboDevices()
                                        status = "Default Kobo: \(device.ip)"
                                    } label: {
                                        Image(systemName: koboIP == device.ip && !device.ip.isEmpty ? "star.fill" : "star")
                                            .foregroundStyle(koboIP == device.ip && !device.ip.isEmpty ? .yellow : .secondary)
                                    }.buttonStyle(.plain).contentShape(Rectangle()).help(koboIP == device.ip ? "Default" : "Set as default")
                                    TextField("IP address", text: $device.ip).textFieldStyle(.roundedBorder).font(.system(.body, design: .monospaced)).autocorrectionDisabled().frame(width: 170)
                                        .onSubmit { saveKoboDevices() }
                                    Text("Password").font(.caption)
                                    Group {
                                        if revealedKoboPasswords.contains(device.id) { TextField("optional", text: $device.password) }
                                        else { SecureField("optional", text: $device.password) }
                                    }.textFieldStyle(.roundedBorder).autocorrectionDisabled().font(.system(.body, design: .monospaced)).frame(width: 110)
                                        .onSubmit { saveKoboDevices() }
                                    Button(revealedKoboPasswords.contains(device.id) ? "Hide" : "Show") {
                                        if revealedKoboPasswords.contains(device.id) { revealedKoboPasswords.remove(device.id) }
                                        else { revealedKoboPasswords.insert(device.id) }
                                    }.controlSize(.small)
                                    Spacer()
                                    Button("Test") { Task { await testKobo(ip: device.ip) } }.controlSize(.small)
                                    Button(role: .destructive) {
                                        removeKobo(ip: device.ip)
                                    } label: { Image(systemName: "trash").foregroundStyle(.red) }.buttonStyle(.plain).help("Remove")
                                    Group {
                                        if let result = koboTestResults[device.ip] {
                                            Text(result)
                                                .font(.caption).fontWeight(.bold)
                                                .foregroundStyle(result == "Pass" ? .green : .red)
                                                .padding(.horizontal, 6).padding(.vertical, 2)
                                                .background((result == "Pass" ? Color.green : Color.red).opacity(0.12))
                                                .cornerRadius(4)
                                        } else {
                                            Text("Pass")
                                                .font(.caption).fontWeight(.bold)
                                                .padding(.horizontal, 6).padding(.vertical, 2)
                                                .opacity(0)
                                        }
                                    }
                                    .frame(width: 45, alignment: .leading)
                                }
                                .padding(4).background(Color.secondary.opacity(0.06)).cornerRadius(6)
                            }
                            HStack(spacing: 8) {
                                TextField("Kobo IP address", text: $newKoboIP).textFieldStyle(.roundedBorder).font(.system(.body, design: .monospaced)).autocorrectionDisabled().frame(width: 170)
                                    .onSubmit { addKobo() }
                                Button("Add") { addKobo() }.disabled(newKoboIP.trimmingCharacters(in: .whitespaces).isEmpty)
                                Spacer()
                            }
                            Text("Star selects default for “Read on Kobo”. IP + password edit in place; password is optional. Ping tests awake.").font(.caption2).foregroundStyle(.secondary)
                        }
                        .onChange(of: koboDevices) { _, _ in saveKoboDevices() }
                    } label: { Label("Kobo", systemImage: "ipad.landscape").font(.headline) }

                    // Appearance — Theme like Search Date: Theme  [Light] [Dark] [System]
                    HStack(spacing: 8) {
                        Text("Theme").foregroundColor(Color(red: 0.5, green: 0.83, blue: 0.98))
                            .fontWeight(.bold).frame(width: 50, alignment: .leading)
                        HStack(spacing: 4) {
                            ForEach([(1, "Light"), (2, "Dark"), (0, "System")], id: \.0) { tag, label in
                                Button(label) { themePreference = tag }
                                    .font(.caption)
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 4)
                                    .background(themePreference == tag ? Color.accentColor : Color(nsColor: NSColor.controlBackgroundColor))
                                    .foregroundColor(themePreference == tag ? .white : .primary)
                                    .clipShape(Capsule())
                                    .buttonStyle(.plain)
                            }
                        }
                        Spacer()
                    }
                    .padding(.horizontal, 12)
                    .frame(minHeight: 28)
                }.padding(.horizontal, 12).padding(.top, 12).padding(.bottom, 6).frame(maxWidth: .infinity)
            }.navigationTitle("Settings").toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Save") { if save() { dismiss() } }.buttonStyle(.borderedProminent) }
            }
        }.onAppear { load() }
    }

    private func load() {
        // Load source toggle + local dir (shared with Qt via same defaults keys)
        if UserDefaults.standard.string(forKey: "library_source") == nil {
            UserDefaults.standard.set("smb", forKey: "library_source")
        }
        librarySource = UserDefaults.standard.string(forKey: "library_source") ?? "smb"
        localDir = UserDefaults.standard.string(forKey: "local_library_dir") ?? ""
        // Load SMB from saved single server — blank when never configured.
        smbServer = ""; smbShare = ""; smbUser = ""; smbPass = ""; smbDomain = ""; calibrePath = ""
        if let s = SmbServer.saved.first {
            smbServer = s.host
            smbUser = s.user
            smbDomain = s.domain
            if let pw = KeychainHelper.read(account: s.host), !pw.isEmpty { smbPass = pw }
            if let share = s.shares.first {
                smbShare = share.name
                // Share-relative dir of metadata.db, e.g. "calibre/metadata.db" -> "calibre/".
                let remote = share.calibreMetadataPath
                if let slash = remote.lastIndex(of: "/") {
                    calibrePath = String(remote[..<slash]) + "/"
                }
            }
        }
        // Kobo devices — empty until the user adds their own. Migrates the
        // legacy [String] list; per-IP passwords come from the app store.
        var devs: [KoboDevice] = []
        if let data = UserDefaults.standard.data(forKey: "kobo_ips") {
            if let d = try? JSONDecoder().decode([KoboDevice].self, from: data) { devs = d }
            else if let arr = try? JSONDecoder().decode([String].self, from: data) { devs = arr.map { KoboDevice(ip: $0) } }
        }
        if devs.isEmpty && !koboIP.isEmpty { devs = [KoboDevice(ip: koboIP)] }
        var pwMap = loadKoboPasswordMap()
        // Migrate the interim single global password onto the default device.
        if pwMap.isEmpty, let legacy = KeychainHelper.read(account: "kobo"), !legacy.isEmpty {
            let idx = devs.firstIndex(where: { $0.ip == koboIP }) ?? devs.indices.first
            if let i = idx { devs[i].password = legacy; pwMap[devs[i].ip] = legacy }
        }
        for i in devs.indices where devs[i].password.isEmpty {
            if let pw = pwMap[devs[i].ip] { devs[i].password = pw }
        }
        koboDevices = devs
        // Force-write the password map here: the change-guard in
        // saveKoboDevices() can't see this migration (lastSaved is computed
        // from the same in-memory state, so it compares equal and skips the
        // write) — and sshSync reads the store, not this state. Without this,
        // a migrated password shows in the field but is never used.
        let migratedMap = koboPasswordMap()
        if !migratedMap.isEmpty,
           let data = try? JSONEncoder().encode(migratedMap),
           let s = String(data: data, encoding: .utf8) {
            KeychainHelper.save(password: s, for: "kobo-passwords")
        }
        lastSavedKoboPasswords = migratedMap
        if !koboDevices.contains(where: { $0.ip == koboIP }) && !koboDevices.isEmpty { koboIP = koboDevices[0].ip }
        status = ""
    }
    /// { ip: password } for devices with a password — app store only.
    private func koboPasswordMap() -> [String: String] {
        Dictionary(uniqueKeysWithValues: koboDevices.compactMap { d -> (String, String)? in
            let ip = d.ip.trimmingCharacters(in: .whitespacesAndNewlines)
            return d.password.isEmpty || ip.isEmpty ? nil : (ip, d.password)
        })
    }
    private func loadKoboPasswordMap() -> [String: String] {
        guard let s = KeychainHelper.read(account: "kobo-passwords"),
              let d = s.data(using: .utf8),
              let dict = try? JSONDecoder().decode([String: String].self, from: d) else { return [:] }
        return dict
    }
    private func saveKoboDevices() {
        if let data = try? JSONEncoder().encode(koboDevices) { UserDefaults.standard.set(data, forKey: "kobo_ips") }
        UserDefaults.standard.set(koboIP, forKey: "kobo_ip")
        let dict = koboPasswordMap()
        if dict != lastSavedKoboPasswords {
            lastSavedKoboPasswords = dict
            if let data = try? JSONEncoder().encode(dict), let s = String(data: data, encoding: .utf8) {
                KeychainHelper.save(password: s, for: "kobo-passwords")
            }
        }
    }
    private func addKobo() {
        let ip = newKoboIP.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !ip.isEmpty else { return }
        guard !koboDevices.contains(where: { $0.ip == ip }) else { status = "Already exists"; return }
        koboDevices.append(KoboDevice(ip: ip))
        if koboIP.isEmpty { koboIP = ip }
        saveKoboDevices(); newKoboIP = ""; status = "Added \(ip)"
    }
    private func removeKobo(ip: String) {
        koboDevices.removeAll { $0.ip == ip }
        koboTestResults.removeValue(forKey: ip)
        if koboIP == ip { koboIP = koboDevices.first?.ip ?? ""; }
        saveKoboDevices()
        status = koboDevices.isEmpty ? "No Kobo IPs" : "Removed \(ip)"
        if koboIP.isEmpty && !koboDevices.isEmpty { koboIP = koboDevices[0].ip; saveKoboDevices() }
    }
    private func browseLocal() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.prompt = "Choose"
        panel.message = "Pick your Calibre library folder (the one containing metadata.db)."
        if !localDir.isEmpty { panel.directoryURL = URL(fileURLWithPath: localDir) }
        if panel.runModal() == .OK, let url = panel.url {
            localDir = url.path
            librarySource = "local"
        }
    }

    private func testLocal() async {
        var dir = localDir.trimmingCharacters(in: .whitespacesAndNewlines)
        if dir.hasPrefix("file://") { dir = String(dir.dropFirst("file://".count)) }
        while dir.hasSuffix("/") && dir.count > 1 { dir = String(dir.dropLast()) }
        guard !dir.isEmpty else { await MainActor.run { status = "Fail Local: enter a folder first" }; return }
        let dbPath = (dir as NSString).appendingPathComponent("metadata.db")
        guard FileManager.default.fileExists(atPath: dbPath) else {
            await MainActor.run { status = "Fail Local: no metadata.db in \(dir)" }
            return
        }
        let result: String = await Task.detached(priority: .userInitiated) { () -> String in
            var db: OpaquePointer?
            guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let d = db else {
                return "Fail Local: cannot open metadata.db"
            }
            defer { sqlite3_close(d) }
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(d, "SELECT count(*) FROM books", -1, &stmt, nil) == SQLITE_OK, let s = stmt else {
                return "Fail Local: not a Calibre database"
            }
            defer { sqlite3_finalize(s) }
            let n = sqlite3_step(s) == SQLITE_ROW ? Int(sqlite3_column_int64(s, 0)) : -1
            return n >= 0 ? "OK Local \(n) books" : "Fail Local: unreadable"
        }.value
        await MainActor.run { status = result }
    }

    /// Returns false when the sheet should stay open (SMB selected but incomplete).
    private func save() -> Bool {
        // Persist either-or source first (shared with Qt).
        if librarySource != "smb" && librarySource != "local" { librarySource = "smb" }
        UserDefaults.standard.set(librarySource, forKey: "library_source")
        var cleanLocal = localDir.trimmingCharacters(in: .whitespacesAndNewlines)
        if cleanLocal.hasPrefix("file://") { cleanLocal = String(cleanLocal.dropFirst("file://".count)) }
        while cleanLocal.hasSuffix("/") && cleanLocal.count > 1 { cleanLocal = String(cleanLocal.dropLast()) }
        localDir = cleanLocal
        UserDefaults.standard.set(cleanLocal, forKey: "local_library_dir")
        let host = smbServer.trimmingCharacters(in: .whitespacesAndNewlines)
        let shareName = smbShare.trimmingCharacters(in: .whitespacesAndNewlines)
        let user = smbUser.trimmingCharacters(in: .whitespacesAndNewlines)
        let domain = smbDomain.trimmingCharacters(in: .whitespacesAndNewlines)
        let pass = smbPass
        // Library dir inside the share holding metadata.db, e.g. "calibre/".
        var libDir = calibrePath.trimmingCharacters(in: .whitespacesAndNewlines)
        while libDir.hasPrefix("/") { libDir = String(libDir.dropFirst()) }
        if !libDir.isEmpty && !libDir.hasSuffix("/") { libDir += "/" }
        calibrePath = libDir
        if librarySource == "smb" {
            guard !host.isEmpty, !shareName.isEmpty else {
                // Kobo default is independent of SMB — persist it even when
                // SMB validation blocks dismissal, so a star tap is never lost.
                saveKoboDevices()
                status = koboIP.isEmpty ? "Enter SMB host and share first"
                    : "Enter SMB host and share first (Kobo default saved: \(koboIP))"
                return false
            }
        }
        // Save single server (port 445 is the SMB standard; Keychain holds the password).
        let metaPath = "\(libDir)metadata.db"
        let server = SmbServer(label: "", host: host, port: 445, user: user, domain: domain, shares: [SmbShare(name: shareName, calibreMetadataPath: metaPath)])
        SmbServer.save([server])
        if !pass.isEmpty && !host.isEmpty { KeychainHelper.save(password: pass, for: server.host) }
        // Save single Calibre library primary
        let fullPath = "\(server.host)/\(shareName)/\(metaPath)"
        var libs = CalibreManager.shared.libraries
        if let idx = libs.firstIndex(where: { $0.type == .smb && $0.smbHost == server.host }) {
            libs[idx].path = fullPath; libs[idx].name = "Main"; libs[idx].isPrimary = true
            for i in libs.indices { libs[i].isPrimary = (i == idx) }
            CalibreManager.shared.libraries = libs; CalibreManager.shared.save()
        } else {
            // clear old smb libs and add single
            libs.removeAll { $0.type == .smb }
            let cfg = CalibreLibraryConfig(name: "Main", type: .smb, path: fullPath, isPrimary: true)
            libs.append(cfg); CalibreManager.shared.libraries = libs; CalibreManager.shared.save()
        }
        saveKoboDevices()
        if librarySource == "local" {
            status = "Saved Local \(cleanLocal.isEmpty ? "(no folder)" : cleanLocal)"
        } else {
            status = "Saved SMB \(host)/\(shareName)/\(metaPath)"
        }
        // Drop the cached in-memory snapshot and tell the catalog to reload
        // (same as Reindex) — otherwise the source toggle looks dead until
        // the user manually hits Reload.
        Task {
            await SmbCatalogDB.shared.invalidate()
            await MainActor.run {
                NotificationCenter.default.post(name: .importedFoldersChanged, object: nil)
            }
        }
        return true
    }
    private func smbTestFields() -> (host: String, share: String, user: String, pass: String, domain: String, libDir: String)? {
        let host = smbServer.trimmingCharacters(in: .whitespacesAndNewlines)
        let share = smbShare.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !host.isEmpty, !share.isEmpty else { return nil }
        var libDir = calibrePath.trimmingCharacters(in: .whitespacesAndNewlines)
        while libDir.hasPrefix("/") { libDir = String(libDir.dropFirst()) }
        if !libDir.isEmpty && !libDir.hasSuffix("/") { libDir += "/" }
        return (host, share, smbUser.trimmingCharacters(in: .whitespacesAndNewlines), smbPass,
                smbDomain.trimmingCharacters(in: .whitespacesAndNewlines), libDir)
    }

    private func testSMBConnection() async {
        guard let f = smbTestFields() else {
            await MainActor.run { smbConnPass = false; status = "Fail SMB-connection: enter host and share first" }
            return
        }
        do {
            try await SmbService.testConnection(host: f.host, user: f.user, password: f.pass, share: f.share, domain: f.domain)
            await MainActor.run { smbConnPass = true; status = "OK SMB-connection \(f.host)/\(f.share)" }
        }
        catch { await MainActor.run { smbConnPass = false; status = "Fail SMB-connection \(error.localizedDescription)" } }
    }

    private func testSMBDatabase() async {
        guard let f = smbTestFields() else {
            await MainActor.run { smbDbPass = false; status = "Fail SMB-database: enter host and share first" }
            return
        }
        do {
            // metadata.db found at the path is a pass (lightweight listing, no 44MB download).
            let dir = f.libDir.isEmpty ? "" : String(f.libDir.dropLast())
            let entries = try await SmbService.listDirectory(host: f.host, user: f.user, password: f.pass, share: f.share, domain: f.domain, path: dir)
            let found = entries.contains { $0.name.lowercased() == "metadata.db" }
            await MainActor.run {
                smbDbPass = found
                status = found ? "OK SMB-database \(f.host)/\(f.share)/\(f.libDir)metadata.db found" : "Fail SMB-database \(f.host)/\(f.share)/\(f.libDir) — no metadata.db here"
            }
        }
        catch { await MainActor.run { smbDbPass = false; status = "Fail SMB-database \(error.localizedDescription)" } }
    }
    private func sync() async {
        // Single-database rule: no import/sync — the live database (SMB or
        // local folder) IS the library. Re-fetch it and refresh the catalog view.
        let src = UserDefaults.standard.string(forKey: "library_source") ?? "smb"
        isSyncing = true; status = src == "local" ? "Reading local Calibre…" : "Reading SMB Calibre…"
        await SmbCatalogDB.shared.invalidate()
        await MainActor.run {
            status = src == "local" ? "Refreshed from local Calibre" : "Refreshed from SMB Calibre"; isSyncing = false
            NotificationCenter.default.post(name: .importedFoldersChanged, object: nil)
        }
    }
    private func testKobo(ip: String? = nil) async {
        let target = (ip ?? koboIP).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { await MainActor.run{ status="Enter Kobo IP" }; return }
        await MainActor.run{ status="Testing Kobo \(target)… (ping)"; koboTestResults[target] = nil }
        let result = await Task.detached(priority: .userInitiated) { () -> (String, Int32) in
            let pingPaths = ["/sbin/ping", "/bin/ping", "/usr/bin/ping"]
            var pingPath: String? = nil
            for p in pingPaths { if FileManager.default.fileExists(atPath: p) { pingPath = p; break } }
            let exe = pingPath ?? "/sbin/ping"
            let p = Process()
            p.executableURL = URL(fileURLWithPath: exe)
            p.arguments = ["-c", "1", "-W", "1000", "-o", target]
            let pipe = Pipe()
            p.standardOutput = pipe; p.standardError = pipe
            do {
                try p.run()
                let deadline = Date().addingTimeInterval(2)
                while p.isRunning && Date() < deadline { usleep(50000) }
                if p.isRunning { p.terminate() }
                let d = pipe.fileHandleForReading.readDataToEndOfFile()
                let out = String(data: d, encoding: .utf8) ?? ""
                if p.isRunning { return (out + "\n(timed out)", 124) }
                return (out, p.terminationStatus)
            } catch { return (error.localizedDescription, -1) }
        }.value
        let ok = result.1 == 0
        let summary = result.0.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n").last ?? ""
        await MainActor.run{
            if ok { status = "Kobo awake \(target) (\(summary.prefix(120)))"; koboTestResults[target] = "Pass" }
            else { status = "Kobo is sleeping or unreachable at \(target) — press power button to wake."; koboTestResults[target] = "Fail" }
        }
    }
}

struct BookCell: View {
    let book: CatalogBook
    @ObservedObject var store: CatalogStore
    var onOpen: (() -> Void)? = nil
    var body: some View {
        VStack(spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.12))
                if let img = store.thumbnails[book.path] {
                    Image(nsImage: img).resizable().aspectRatio(contentMode: .fit)
                        .frame(width: 187, height: 240)
                } else if book.hasCover {
                    // Placeholder, not spinner — spinner for 7K is thundering herd; Android shows book.closed until loaded
                    Image(systemName: "book.closed").font(.title2).foregroundStyle(.secondary).opacity(0.6)
                } else {
                    Image(systemName: "book.closed").font(.title2).foregroundStyle(.secondary)
                }
            }
            .frame(width: 187, height: 240).clipShape(RoundedRectangle(cornerRadius: 6))
            Text(book.title).font(.caption).lineLimit(2).multilineTextAlignment(.center).frame(width: 187)
            Text(book.author).font(.caption2).foregroundStyle(.secondary).lineLimit(1).frame(width: 187)
        }
        .help("\(book.title) — \(book.author)\n\(book.path)\nClick for details")
        .contentShape(Rectangle())
        .onHover { inside in
            if inside {
                NSCursor.pointingHand.push()
            } else {
                NSCursor.pop()
            }
        }
        .onTapGesture { onOpen?() }
        .contextMenu {
            Button("Show Details") { onOpen?() }
            Button("Open on Kobo") { Task { await KoboLauncher.open(book: book, store: store) } }
            Button("Copy Kobo query") {
                let q = "\(book.author) \(book.title)"
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(q, forType: .string)
            }
        }
    }
}

// Book information screen — shown on tap, Read button launches on Kobo only
struct BookInfoView: View {
    let book: CatalogBook
    let detail: BookDetail?
    let isLoading: Bool
    @ObservedObject var store: CatalogStore
    var onRead: () -> Void
    var onClose: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    HStack(alignment: .top, spacing: 16) {
                        ZStack {
                            RoundedRectangle(cornerRadius: 8).fill(Color.secondary.opacity(0.12))
                            if let img = store.thumbnails[book.path] {
                                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 160, height: 240).clipped()
                            } else if let img = store.coverImageSync(for: book) {
                                Image(nsImage: img).resizable().aspectRatio(contentMode: .fit).frame(width: 160, height: 240).clipped()
                            } else if book.hasCover {
                                ProgressView()
                            } else {
                                Image(systemName: "book.closed").font(.largeTitle).foregroundStyle(.secondary)
                            }
                        }
                        .frame(width: 160, height: 240).clipShape(RoundedRectangle(cornerRadius: 8))
                        VStack(alignment: .leading, spacing: 8) {
                            Text(book.title).font(.title2).fontWeight(.semibold).lineLimit(3)
                            Text(book.author).font(.title3).foregroundStyle(.secondary)
                            if let d = detail {
                                if let s = d.series, !s.isEmpty {
                                    Text("Series: \(s) #\(String(format: "%g", d.seriesIndex))").font(.body).foregroundStyle(.secondary)
                                }
                                if let tags = d.tags, !tags.isEmpty {
                                    Text(tags).font(.callout).foregroundStyle(.secondary).lineLimit(3)
                                }
                                if let pub = d.publisher, !pub.isEmpty {
                                    Text("Publisher: \(pub)").font(.callout).foregroundStyle(.secondary)
                                }
                                if let isbn = d.isbn, !isbn.isEmpty {
                                    Text("ISBN: \(isbn)").font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                                }
                            } else if isLoading {
                                ProgressView().scaleEffect(0.7)
                            }
                            Spacer()
                        }
                        Spacer()
                    }
                    if let d = detail, let c = d.comments, !c.isEmpty {
                        Text(c).font(.system(.body, design: .default)).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled).padding(10).background(Color.black.opacity(0.04)).cornerRadius(6)
                    } else if isLoading {
                        ProgressView("Loading details…").frame(maxWidth: .infinity)
                    }
                    // Location removed per request
                }
                .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 8)
            }
            Divider()
            HStack(spacing: 12) {
                Button(action: onRead) {
                    Label("Read on Kobo", systemImage: "book.fill").font(.body)
                }
                .buttonStyle(.borderedProminent).controlSize(.large).keyboardShortcut(.defaultAction)
                .help("Opens this book on the Kobo via KOReader (internal SSH)")
                Button("Close") { onClose() }.controlSize(.large).keyboardShortcut(.cancelAction)
                Spacer()
                if !store.koboStatus.isEmpty {
                    Text(store.koboStatus).font(.callout).foregroundStyle(store.koboStatus.hasPrefix("Opened") ? .green : .orange).lineLimit(1)
                }
            }
            .padding(.horizontal, 20).padding(.vertical, 12).background(.bar)
        }
        .frame(width: 620, height: 600)
        .alert("Kobo", isPresented: Binding(get: { store.koboError != nil }, set: { if !$0 { store.koboError = nil } })) {
            Button("OK") { store.koboError = nil }
        } message: { Text(store.koboError ?? "") }
    }
}

@main
struct CatalogSwiftApp: App {
    @AppStorage("theme_preference") private var themePreference = 0
    var body: some Scene {
        WindowGroup { ContentView()
                .preferredColorScheme(themePreference == 1 ? .light : themePreference == 2 ? .dark : nil)
        }
            .windowStyle(.titleBar)
            .windowResizability(.contentSize)
            .commands {
                CommandGroup(after: .appInfo) {
                    Button("Open Data Folder") { NSWorkspace.shared.open(CatalogPaths.base) }.keyboardShortcut("o")
                    Button("Settings…") { NSWorkspace.shared.open(URL(string:"x-apple.systempreferences:")!) }.keyboardShortcut(",")
                }
            }
    }
    init() {
        // Single-database rule: the ONLY database lives on the Settings SMB
        // server (read live into memory). Nothing is created here except the
        // covers/thumbnails caches. No library.db, no library_smb.db,
        // no library_web.db, no remote_smb snapshot, no Books tree.
        let base = CatalogPaths.base
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: base.appendingPathComponent("covers"), withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: base.appendingPathComponent("thumbnails"), withIntermediateDirectories: true)
        // Covers are on-demand via ThumbnailService (per-cell lazy SMB download).
    }
}
