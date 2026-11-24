# Changelog

## v2.6.0 - 2026-11-24

### Added

- An interface language selector with System, English, Simplified Chinese, Traditional Chinese, and Japanese. The choice persists and applies to the dashboard, menu-bar popover, dialogs, settings, and interface status messages.
- Localized native menu-bar menu labels that follow the selected interface language.
- A Japanese, Simplified Chinese, and Traditional Chinese README alongside the English version.

### Changed

- Release assets now use one naming scheme on every platform: `CodexMonitor-<version>-<os>-<arch>.<ext>`.
- macOS releases ship both Apple Silicon and Intel builds.
- Releases regenerate the signed updater feed (`latest.json`) after every platform has finished, then attach `CodexMonitor-<version>-SHA256SUMS.txt`.
- Release notes are generated from this changelog instead of a generic placeholder.
- Backend diagnostic logs stay in English so bug reports remain consistent.

### Fixed

- Changing the interface language no longer reports a settings error when the menu-bar item is hidden.
- The first status message after startup follows the saved language instead of staying in English.

### Removed

- The `CodexMonitor-macOS.zip` compatibility asset. Users still on the v1 Python app must migrate manually; see the README.

## v2.5.2 - 2026-11-17

### Added

- A Settings dialog with an on/off switch for the macOS menu-bar item. The choice persists across launches.
- Desktop-notification and automatic-update-check switches, so background behaviour can be disabled without editing files.

### Changed

- Closing the main window now quits Codex Monitor when the menu bar item is turned off, instead of leaving a hidden process with no way to reopen it.
- Automatic update checks are skipped entirely when the setting is off; manual checks still work.

## v2.5.1 - 2026-11-16

### Fixed

- Restore the Windows and Linux release builds. v2.5.0's release job stopped at Rust lints for macOS-only restart helpers, so only the macOS build was published; v2.5.1 ships all three platforms.

## v2.5.0 - 2026-11-16

### Added

- macOS menu-bar presence: both remaining windows of the active account are shown as menu-bar text (`5h·100% 7d·42%`), and clicking it opens a popover for checking accounts and switching in one step.
- A safe account switch that confirms whether Codex is running, closes it before writing credentials, verifies the applied session, and reopens the same Codex bundle. A failed write reopens the previous session; a failed reopen is reported without hiding the successful switch.
- A collapsible Activity panel with a fixed-height log viewer instead of a permanently visible, resizable log pane.
- Add account opens a chooser first: start the ChatGPT sign-in, or import a saved session from an `auth.json` file or pasted JSON text.

### Changed

- Redesigned the dashboard around one active-account card and compact account rows, with clear weekly/5-hour quota, reset, and last-fetch details.
- The main window uses the native macOS title bar again, so it drags, zooms, and moves like any other app window.
- Use the shadcn neutral theme instead of the green primary colour; quota meters keep their own levels — green above 50% left, amber from 21–50%, red at 20% or less — so a full meter no longer blends into the interface.
- Both quota windows render as separate progress bars in the dashboard and the menu bar, with a human countdown (`3d 9h 7m`) and a same-day or dated reset stamp. Weekly-only accounts show one bar.
- Dashboard quota rows use a fixed five-column grid for the window label, bar, percentage, countdown, and reset stamp. Every bar is the same length, and the wider window uses the extra space instead of letting text length resize the bar.
- Remove the leading account initial from dashboard rows and the menu-bar popover. Email and quota now start at the row edge, leaving more width for the address and reset details.
- The menu-bar refresh button now asks OpenAI for fresh usage on every account and explains itself in a tooltip; it previously only re-read the cached snapshot, which looked like nothing happened.
- Closing the main window now keeps Codex Monitor running in the macOS menu bar; use the tray menu to reopen or quit.

### Fixed

- Remove old native hover labels across the dashboard and menu-bar popover. Hover details now come only from explicit shadcn tooltips, so stale browser titles cannot appear next to the current account.
- Drag the main window by its title bar again: the previous overlay title bar used a CSS property that only Electron honours, so it never reached Tauri's drag handler. The window now keeps the native title bar.
- The menu-bar item is text only, with no glyph competing with the quota numbers, and keeps a `Codex` label before the first quota reading so it stays clickable on a fresh install.
- Refresh a saved session before switching only when its access token is actually expiring. A still-valid session is never rotated on a guess; with single-use refresh tokens, that guess could end the session.
- Stamp a fresh handoff timestamp and fill in the workspace id when writing the live session, so Codex does not spend a single-use refresh token during startup.
- Keep saved sessions in sync when Codex rotates tokens, both after a switch and when the active account is clicked again.
- Treat two sessions as the same account only when the workspace id and the ChatGPT user both match.
- Write the live session through a symlinked `auth.json` instead of replacing the link, and refuse a hard-linked file with a clear message instead of leaving two divergent copies of the credentials.
- Keep long sign-in and import errors inside the Add account dialog, and clear a previous attempt's error when the dialog opens.
- Update menu-bar quota countdowns while the popover is open instead of leaving them frozen until the next refresh.
- Cancel an in-flight sign-in when the Add account dialog closes, so an abandoned browser login cannot linger in the background.
- Anchor the menu-bar popover to the tray item and keep it on the monitor that owns the item, including displays to the left of the primary display.
- Let a second click on the menu-bar item dismiss the popover instead of immediately reopening it.

## v2.4.2 - 2026-11-11

### Fixed

- Restart Codex after account switching even when its app or executable is renamed.
- Stop remaining desktop helpers before reopening, with a bounded force-stop fallback for stuck processes.
- Verify that the new main process stays running instead of treating a launch request as success.
- Keep the monitor responsive during restart and reject overlapping restart requests.

## v2.4.1 - 2026-11-10

### Fixed

- Keep the spinner visually centered with a stationary ring behind its rotating arc.
- Restore icon buttons to their original 32px size.
- Move the log auto-scroll toggle into the existing log controls row.

## v2.4.0 - 2026-11-10

### Added

- Copy an account's full email from its row with the adjacent copy button.
- Opt into always following new activity log entries; the choice persists across restarts.

### Fixed

- Keep icon buttons square and spinners centered when the app window is narrow.

## v2.3.10

### Fixed

- Restored the pre-hardening Codex restart sequence used after switching accounts: quit Codex, wait for the process to exit, then reopen it with the short LaunchServices retry flow.
- Kept the current switch dialog, status/log updates, updater, auth, quota, storage, and UI behavior unchanged.
