# Codex Monitor

English | [简体中文](docs/i18n/README.zh-CN.md) | [繁體中文](docs/i18n/README.zh-TW.md) | [日本語](docs/i18n/README.ja.md)

[![GitHub release](https://img.shields.io/github/v/release/koodev24/codex-monitor)](https://github.com/koodev24/codex-monitor/releases/latest)
[![Release build](https://img.shields.io/github/actions/workflow/status/koodev24/codex-monitor/release-v2-tauri.yml?label=release%20build)](https://github.com/koodev24/codex-monitor/actions)
![macOS](https://img.shields.io/badge/macOS-supported-blue)
![Windows](https://img.shields.io/badge/Windows-supported-blue)
![Linux](https://img.shields.io/badge/Linux-supported-blue)

A native desktop app for monitoring Codex API usage across your ChatGPT accounts: 5-hour and weekly quotas, reset countdowns, per-account reset credits, automatic fetching, and signed in-app updates.

## Download

Get the latest build from the [Releases page](https://github.com/koodev24/codex-monitor/releases/latest). Release asset names are the same on every platform:

| OS | Architecture | File |
| --- | --- | --- |
| macOS | Apple Silicon | `CodexMonitor-<version>-macos-arm64.dmg` |
| macOS | Intel | `CodexMonitor-<version>-macos-x64.dmg` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64-setup.exe` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64.msi` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.AppImage` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.deb` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.rpm` |

Each release also includes:

- `latest.json` — the updater manifest the app reads; it lists the signed updater artifact for each platform. Its name is fixed by the updater endpoint, so it intentionally has no version in the filename. Do not edit or republish it by hand.
- `CodexMonitor-<version>-SHA256SUMS.txt` — checksums for the downloadable artifacts.
- `CodexMonitor-<version>-macos-*.app.tar.gz` and `.sig` files — updater payloads and signatures. Normal macOS installs use the DMG.

Verify a download with the checksum file:

```bash
shasum -a 256 -c CodexMonitor-<version>-SHA256SUMS.txt
```

### macOS first launch

The app is unsigned, so macOS blocks it on first launch. Right-click the app and choose **Open**, or clear the quarantine flag once:

```bash
xattr -dr com.apple.quarantine /Applications/CodexMonitor.app
open /Applications/CodexMonitor.app
```

Keep the app in `/Applications` or `~/Applications`. The in-app updater replaces the bundle in place.

### Migrating from the v1 Python app

v2.6.0 stops publishing the `CodexMonitor-macOS.zip` bridge that the old Python updater relied on. If you still run v1, migrate manually:

1. Quit the old app.
2. Download the macOS DMG for your Mac (Apple Silicon or Intel) from the latest release.
3. Open the DMG and move **CodexMonitor.app** to `/Applications` or `~/Applications`.
4. Launch the new app. On first start it imports saved accounts and usage history from the old data directory when present; the old files are only read, never deleted.
5. If accounts are missing, use **Add account** to sign in or import a saved `auth.json`. You can remove the old Python app afterwards.

## Features

- **Multi-account quotas** — remaining quota and reset countdowns for the 5-hour and weekly windows, per account.
- **Reset credits** — per-account ledger with expiry and remaining time.
- **Fetching** — one-click quota fetch per account or for all accounts, plus configurable auto-fetch intervals.
- **Account management** — add accounts through Codex login, archive unused ones, and switch the active account safely: Codex closes, fresh tokens are preserved, the selected session is written, and Codex reopens automatically.
- **Menu-bar popover (macOS)** — the menu-bar title shows the active account's 5-hour and weekly quota; click it for a compact account list, one-step switching, and dashboard access.
- **Live logs** — a fixed-height activity panel with one-click copy and clear.
- **Settings** — interface language, menu-bar visibility, desktop notifications, and automatic update checks, saved immediately.
- **In-app updates** — signed update feed with manual checks and background checks every 6 hours.

## Languages

The interface ships in English, Simplified Chinese, Traditional Chinese, and Japanese. Choose a language in **Settings → Language**, or keep **System** to follow the OS language. The native menu-bar menu and the dashboard use the same setting, and status messages from the app follow it too. Backend diagnostic logs remain in English so they stay useful in bug reports.

## Usage

1. Launch the app and click **Add account** to sign in through Codex login.
2. Click **Refresh quota** (or wait for auto-fetch) to pull usage for the active account, or **Fetch all accounts** for every backup.
3. Hover any toolbar button for what it does; click the **eye** to toggle archived accounts, the **sun/moon** for light/dark theme, and the **gear** for settings.
4. Click **Activity** at the bottom to expand or collapse logs. Switching accounts asks once, then handles Codex close, credential switch, and reopen as a single action.
5. On macOS, closing the window keeps Codex Monitor running in the menu bar. Use the tray popover to reopen the dashboard, or **Quit** from its menu to exit. If you turn off **Show menu bar item** in Settings, closing the window quits the app instead.

## How updating works

- The **cloud-download** button checks `latest.json` from the latest release. If an update exists, it downloads the artifact for your OS and architecture, verifies the signature, installs, and relaunches.
- Checks also run silently at startup and every 6 hours. Results appear in the status bar, never in a popup.
- `latest.json` is regenerated by CI after every platform build finishes, so it always points at assets from the same release. That is why published releases are never moved or edited by hand.

## Development

Requirements: Node 22 and a stable Rust toolchain.

```bash
cd apps/desktop
npm install
npm run tauri dev
```

Useful commands (from `apps/desktop` unless noted):

```bash
npm run build          # frontend typecheck (tsc) + Vite build
npm test               # frontend unit tests
npm run tauri dev      # full app with hot reload
npm run tauri build    # local release bundle (never committed)
cargo test             # Rust backend tests (apps/desktop/src-tauri)
cargo clippy --all-targets -- -D warnings
```

Project layout:

```text
apps/desktop/
  src/                 # React frontend (components, store, i18n, formatting)
  src-tauri/           # Rust backend (API client, storage, watcher, updater)
scripts/
  set-version.mjs      # syncs version from release tags for CI
  finalize-release.mjs # normalizes assets and writes latest.json
  release-notes.mjs    # extracts release notes from CHANGELOG.md
```

## Releasing

Push a `v2*` tag. CI creates a draft release, builds macOS arm64, macOS x64, Windows x64, and Linux x64, then normalizes every asset name, regenerates `latest.json`, writes `CodexMonitor-<version>-SHA256SUMS.txt`, and publishes the release once:

```bash
git tag v2.6.0
git push origin v2.6.0
```

Release rules:

- **Only cut `v2*` tags.** This workflow only runs for `v2*`; v1 releases are no longer produced.
- Published releases are never moved or overwritten. Cut a new patch version instead.
- Locally built bundles are never committed (see `.gitignore`); releases come from CI only.
- The release body is generated from `CHANGELOG.md`, so add the version section before tagging.
