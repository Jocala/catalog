# Jocala Catalog — Calibre Library Browser

Browse your Calibre ebook library from your Windows, macOS, or Linux PC.
Find a book fast, see its details, then read it on your Kobo: open a book
already on the device, or sync a missing one over with one click.

![Library grid](https://www.jocala.com/catalog/images/catalog-grid.png)

## Features

- **★ Read on Kobo** — open a book directly on your Kobo, or sync it to the device if it is not there yet.
- **Fast library grid** — scroll thousands of books with covers. Tested against a 6,755-book library.
- **Search that helps** — by title, author, series, or tag, with a form that fills the dropdowns for you.
- **Book detail** — cover, author, publisher, ISBN, and description at a glance before you read.
- **Live Calibre library** — reads your Calibre database on your PC or over the LAN. No import step, no duplicates.
- **Windows, macOS & Linux** — same app on all three platforms. Same features, same flow.

![Settings](https://www.jocala.com/catalog/images/catalog-settings.png)
![Search](https://www.jocala.com/catalog/images/catalog-search.png)

## Get Catalog v1.04

Catalog is free for Windows, macOS, and Linux. No ads, no tracking. Just point Catalog at your Calibre library and start reading.

| Platform | Size | Download |
|----------|------|----------|
| Windows | TBD | [jocala-catalog.1.04.exe](https://www.jocala.com/catalog/jocala-catalog.1.04.exe) |
| macOS | TBD | [jocala-catalog.1.04.dmg](https://www.jocala.com/catalog/jocala-catalog.1.04.dmg) |
| Linux | TBD | [jocala-catalog.1.04.tar.gz](https://www.jocala.com/catalog/jocala-catalog.1.04.tar.gz) |

More at [jocala.com/catalog](https://www.jocala.com/catalog/) ·
[Changelog](https://www.jocala.com/catalog/changelog.txt)

© 2026 Jocala Software · Read on Kobo requires KOReader and SSH.

## Build from source

One tree, three targets. Qt Widgets shell (`qt/`) over the Rust engine
(`crates/catalog-core` + `crates/catalog-ffi`); `crates/catalog-cli` is a
headless acceptance harness.

- **Windows** (static Qt 6.11.1): mirror `qt` + `crates/` + `Cargo.toml`/`Cargo.lock`,
  run `qt/build-catalog-windows.ps1`, then `qt/package-catalog-windows.ps1`.
- **Linux** (system Qt via `qt6-base-dev`): run `qt/build-catalog-linux.sh`,
  then `qt/package-catalog-linux.sh` (TGZ).
- **macOS** (static Qt 6.11.1): run `qt/build-catalog-macos.sh`
  (universal app), then `qt/package-catalog-macos.sh` (signed + notarized DMG).

Verify: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`.

Full procedure (release, staging, AppId): `AGENTS.md`.
