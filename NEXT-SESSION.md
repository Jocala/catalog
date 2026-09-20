# Next session: Rust owns Kobo SSH (`russh`) + sync convergence

Date noted: 2026-09-20. Status: Swift Sync & Open shipped and verified tonight; the
Rust port is PLANNED, not started.

## Goal
One Kobo sync implementation in `catalog-core`, owning SSH on macOS AND Windows.
End the three-transport sprawl (Swift `/usr/bin/ssh`, C# SSH.NET, dead Rust
`/usr/bin/ssh` shell-out in `kobo.rs`).

## Decisions already taken (do not relitigate without cause)
- Sync UX: confirm dialog — `"Title" was not found on this device. / Sync this
  N MB book to Kobo via WiFi?` — `[Sync & Open] [Cancel]`. (Shipped in Swift.)
- Push bytes: raw Calibre EPUB at the `.kepub.epub` predicted path. Kepubify = v2.
- No auto-push, no Nickel writes, no Calibre writes. Nickel picks books up on
  reboot/USB-disconnect (proven live 2026-09-20 via the KOBOeReader eject).
- Transport MUST be shell-channel + heredoc: Kobo Dropbear swallows EXEC
  requests (bisected 2026-09-19). The base64-heredoc script is proven; reuse it
  byte-for-byte. Never raw binary on shared stdin (fails with
  "EOF in backquote substitution" — hit 2026-09-20).
- New dep is `russh` (+ helpers as needed), pure Rust / MSVC-clean. NOT
  `ssh2`/libssh2 (native deps, Windows pain). Auth: none/BatchMode-style —
  these Kobos accept Dropbear root login without credentials; no keys involved.
- FFI stays house-style: blocking fns, tokio runtime inside, JSON in/out.
- Swift adoption of core deferred (bridging cost, no user-visible gain). Mark
  `KoboSync.swift` as converging-toward-core when touched.

## Work order
1. **Core module** `crates/catalog-core/src/kobo_sync.rs` (transport-free parts
   first): `epub_source(book_id)` (`data`table, `format='EPUB'`, follow `db.rs`
   patterns) → `sync_plan` (reuses `kobo::predicted_path`) →
   `build_push_script` (goldens) → `parse_push_output` + shell-quote helper.
   Check fixture coverage: does `tests/fixtures/metadata.db` have EPUB rows in
   `data`? Extend `make-fixture.sql` minimally if not.
2. **`russh` transport in core**: TCP → handshake → auth-none → shell channel,
   feed heredoc, collect output, enforce timeouts (mirror Swift `sshSync`
   deadlines: probe 5s, check 6s, find 30s, push 180s). Sequence per push:
   df guard → `mkdir -p` → no-clobber re-check → push `.part` → `wc -c`
   verify → `mv` → append `kobo_index.txt` (core resolves platform data dir).
3. **Mac verify**: `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`.
4. **Live interop vs Color (192.168.1.74)**: russh build on macOS first (no VM
   needed). Kobo awake required (it naps aggressively; power button wakes).
   SAFETY: first push goes to a scratch path under `/tmp`, NOT `/mnt/onboard`;
   verify bytes, delete. Then the real Ulysses script.
5. **FFI**: `kobo_sync_to_device(book_id, ip, settings_json) -> JSON`
   (blocking) + Mac smoke tests vs fixture.
6. **Windows (win10 MCP)**: follow `reader/windows/CATALOG.md` mirror flow
   (Rust tree → `C:\source\reader\catalog`; note VM starts via `virsh start
   win10` on debian, ~2min boot). `cargo build --release -p catalog-ffi` →
   DLL → thin `CatalogWin` down to `KoboSync.cs` P/Invoke + progress →
   `dotnet build` 0/0, `dotnet test` green → live Ulysses script (user-driven).
7. **Cleanup**: delete or `#[cfg(unix)]`-gate dead `kobo.rs::ssh_sync`;
   update `AGENTS.md` Rust section + `CATALOG.md` if mirror paths changed.

## Machine / device state snapshot (2026-09-20 end of night)
- Color (Kobo Clara Colour, FW 4.46.23836): `192.168.1.74`, Ulysses synced +
  Nickel-indexed. No `.part` junk (auto-cleaned by push prep).
- `kobo_index.txt` (Sep-19 base + Ulysses appended) — stale for anything
  else synced/deleted since; refresh-on-miss still covers opens.
- win10 VM: state unknown at close; `win11` stays OFF (IP collision with Vega
  stick at 192.168.1.137 — unrelated to Kobo work, do not start).
- Catalog Swift bundle: `source/catalog/build/Jocala Catalog.app`, Dev-ID
  signed + notarized + stapled (accepted). Single LaunchServices registration
  (stale zzz + Trash entries deregistered).

## Open threads (not blocking the Rust work)
- Bundle LAN grant: `.app` launch still gets SMB posix 50; bare-binary
  Terminal launch works. Next: Local Network toggle dance, else macOS reboot
  (worked 2026-09-19). All tonight's testing went through the bare binary.
- AGENTS.md one-liner unwritten: "quit the app before rebuilding the bundle
  (replacing the binary under a live process earns a kernel SIGKILL,
  CODESIGNING Code 2 — hit 2026-09-20)."
- `cargo test --workspace` baseline never run on the restored tree (resolves
  OK: cargo 1.98.0-nightly; bundled rusqlite first build takes minutes).
- Calibre on-device checkmark after WiFi push: theory says yes post-Nickel
  scan; live USB verification still open.
