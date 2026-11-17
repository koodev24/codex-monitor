# Codex Account Monitor

[![GitHub release](https://img.shields.io/github/v/release/koodev24/codex-monitor)](https://github.com/koodev24/codex-monitor/releases/latest)
[![Release build](https://img.shields.io/github/actions/workflow/status/koodev24/codex-monitor/release-v2-tauri.yml?label=release%20build)](https://github.com/koodev24/codex-monitor/actions)
![macOS](https://img.shields.io/badge/macOS-supported-blue)
![Windows](https://img.shields.io/badge/Windows-supported-blue)
![Linux](https://img.shields.io/badge/Linux-supported-blue)

A native desktop app to monitor Codex API usage and quotas across all your ChatGPT accounts — 5-hour and weekly windows, reset countdowns, per-account reset credits, automatic fetching, and one-click updates.

## Download

Grab the latest release for your OS from the [Releases page](https://github.com/koodev24/codex-monitor/releases/latest):

| OS | File |
| --- | --- |
| macOS (Apple Silicon) | `CodexMonitor_2.x.x_aarch64.dmg` |
| Windows (x64) | `CodexMonitor_2.x.x_x64_en-US.msi` |
| Linux | `.AppImage`, `.deb`, or `.rpm` |

### macOS first launch

The app is unsigned, so macOS will refuse to open it on first launch. Either right-click the app and choose **Open**, or clear the quarantine flag once:

```bash
xattr -dr com.apple.quarantine /Applications/CodexMonitor.app
open /Applications/CodexMonitor.app
```

For the smoothest future updates, keep the app in `~/Applications` — the in-app updater replaces it in place.

### A note on first launch

When you open the app for the first time, macOS asks you to confirm — that's just macOS getting to know a new app. Right-click it, choose **Open**, and you're set for good, updates included.

### Migrating from the old Python app (v1.x)

If you still run the original Python build, just use its built-in updater: v2 releases ship a compatibility bundle, so the old app downloads it, swaps itself out, and reopens as the new Tauri app — accounts, history, and settings carry over automatically.

## Features

- **Multi-account quotas** — remaining quota and reset countdowns for both the 5-hour and weekly windows, per account.
- **Reset credits** — per-account ledger with expiry and remaining time.
- **Fetching** — one-click quota fetch per account or all at once, plus configurable auto-fetch intervals.
- **Account management** — add accounts via Codex login, archive the ones you don't use, and switch the active Codex account safely: Codex closes, fresh tokens are preserved, the selected session is written, and Codex reopens automatically.
- **Menu-bar popover (macOS)** — the tray title shows the active account's 5-hour and weekly quota; click it for a compact account list, one-step switching, and dashboard access.
- **Live logs** — a collapsible activity panel with one-click clear and status copy. It has a fixed height and cannot be resized.
- **Settings** — menu bar visibility, desktop notifications, and automatic update checks, saved immediately.
- **In-app updates** — signed update feed with manual check and automatic background checks.

## Usage

1. Launch the app and click **Add account** to sign in through Codex login.
2. Click **Refresh quota** (or wait for auto-fetch) to pull usage for the active account, or **Fetch all accounts** for every backup.
3. Hover any toolbar button for what it does; click the **eye** to toggle archived accounts, the **sun/moon** for light/dark theme, and the **gear** for settings.
4. Click **Activity** at the bottom to expand or collapse logs. Switching accounts asks once, then handles Codex close, credential switch, and reopen as a single action.
5. On macOS, closing the window keeps Codex Monitor running in the menu bar. Use the tray popover to reopen the dashboard, or **Quit Codex Monitor** from its menu to exit. If you turn off **Show menu bar item** in Settings, closing the window quits the app instead.

## How updating works

- The **cloud-download** button checks a signed update feed (`latest.json`); if an update is found it downloads, verifies the signature, installs, and relaunches.
- The app also checks silently on startup and every 6 hours — you'll get a status message, never a popup.
- Old v1 Python installs follow the same `latest` release through their own updater and migrate in place (see above).

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
npm run tauri dev      # full app with hot reload
npm run tauri build    # local release bundle (never committed)
cargo test             # Rust backend tests (apps/desktop/src-tauri)
cargo clippy --all-targets -- -D warnings
```

Project layout:

```text
apps/desktop/
  src/                 # React frontend (components, store, formatting)
  src-tauri/           # Rust backend (API client, storage, watcher, updater)
scripts/
  set-version.mjs      # syncs version from release tags (CI)
```

## Releasing

Push a `v2*` tag and CI builds every platform, signs the updater artifacts, publishes the `latest.json` feed, and attaches the legacy-migration bundle:

```bash
git tag v2.3.0 && git push origin v2.3.0
```

Rules that keep the updater feed healthy:

- **Only cut `v2*` tags.** A legacy `v1*` release would become "latest" without an update manifest and silently break in-app updates for everyone.
- Branch builds publish as **drafts** so dev builds can never steal the "latest" slot.
- Locally built bundles are never committed (see `.gitignore`) — releases come from CI only.

## Auth file notes

The app watches `~/.codex/auth.json` and expects the Codex auth file to look like this:

```json
{
  "auth_mode": "chatgpt",
  "OPENAI_API_KEY": null,
  "tokens": {
    "id_token": "eyJ....",
    "access_token": "eyJ....",
    "refresh_token": "rt_...",
    "account_id": "uuid"
  },
  "last_refresh": "2026-04-24T15:17:00.949966Z"
}
```

`access_token` is used for quota fetches. `last_refresh` and token rotation are also tracked so the app can surface when Codex silently refreshes auth in the background.
