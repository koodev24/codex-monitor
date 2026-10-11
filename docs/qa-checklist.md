# QA checklist

Manual verification status for shipped behaviour. Ticked means verified on the
stated build; unticked means it still needs a human pass. Refs #26.

## Accounts and quota (v2.2.x, macOS)

- [x] Fetch quota for one account and for all accounts (status + spinners)
- [x] 5h and weekly quota lines side by side when short-window data exists
- [x] Weekly-only accounts render exactly as before
- [x] Reset countdowns tick and clamp at zero
- [x] Add account via Codex login, archive/unarchive, switch active account
- [x] Auto-fetch interval persists and fires

## Data and logs

- [x] Export downloads `codex-monitor-backup.json`
- [x] Import merges accounts newer-wins (never replaces), then reloads
- [x] Logs always visible, clear works, survive panel resize (viewer hides
      below 20% panel height instead of crushing)

## Updates

- [x] Old Python installs migrate to Tauri via the compat zip (proven live)
- [x] Tauri check-for-updates finds signed releases via `latest.json`
- [x] Downloaded updates verify signatures before install

## Release process

- [x] `v2*` tag builds all platforms with signed updater artifacts
- [x] Branch builds stay drafts and never steal the "latest" slot
- [ ] Windows smoke pass (needs a Windows machine)
- [ ] Linux smoke pass (needs a Linux machine)

## Deferred or cancelled

- Theme/accent picker: cancelled by PM (light/dark toggle remains)
- App signing/notarization: blocked on Apple Developer budget (#11);
  unsigned flow with one-time approval is the accepted path
- Automated UI smoke (boots the app against fixture auth): no harness yet;
  frontend pure logic is covered by `npm test` (vitest), backend by
  `cargo test`
