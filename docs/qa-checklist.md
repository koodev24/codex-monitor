# QA checklist

Manual verification status for shipped behaviour. Ticked means verified on the
stated build; unticked means it still needs a human pass. Refs #26.

## Accounts and quota (v2.2.x, macOS)

- [x] Fetch quota for one account and for all accounts (status + spinners)
- [x] 5h and weekly quota bars render as separate lines when short-window data exists
- [x] Weekly-only accounts render exactly as before
- [x] Reset countdowns tick and clamp at zero
- [x] Add account via Codex login, archive/unarchive, switch active account
- [x] Auto-fetch interval persists and fires

## Data and logs

- [x] Export downloads `codex-monitor-backup.json`
- [x] Import merges accounts newer-wins (never replaces), then reloads
- [x] Activity panel collapses/expands, clear works, and the fixed-height log
      viewer survives window resize

## Updates

- [x] Old Python data migrates on first launch; v1 users install the macOS DMG manually (the compat zip was removed in v2.6.0)
- [x] Tauri check-for-updates finds signed releases via `latest.json`
- [x] Downloaded updates verify signatures before install

## Interface language (v2.6.0)

- [x] Language selector switches the dashboard, dialogs, settings, and menu-bar popover
- [x] System follows the operating-system language
- [x] Native menu-bar menu labels follow the selected language
- [x] Backend diagnostic logs stay in English

## Release process

- [x] `v2*` tag builds all platforms with signed updater artifacts
- [x] Release workflow publishes only after every platform build and the finalize step succeed
- [x] Published releases are immutable; cut a new version instead of moving a tag
- [ ] Windows smoke pass (needs a Windows machine)
- [ ] Linux smoke pass (needs a Linux machine)

## Deferred or cancelled

- Theme/accent picker: cancelled by PM (light/dark toggle remains)
- App signing/notarization: blocked on Apple Developer budget (#11);
  unsigned flow with one-time approval is the accepted path
- Automated UI smoke (boots the app against fixture auth): no harness yet;
  frontend pure logic is covered by `npm test` (vitest), backend by
  `cargo test`
