# Changelog

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
