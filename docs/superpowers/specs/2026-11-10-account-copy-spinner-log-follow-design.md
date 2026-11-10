# Account Copy, Spinner Sizing, and Log Follow

## Intent

Restore the per-account email copy action from the Python app, keep action spinners visually centered at narrow window widths, and let users opt into following new log output.

## Goals

- Add a copy icon beside each account email. Copy the exact email and show a status message, matching the old Python behavior.
- Give every icon button a fixed 36px square target and give spinner glyphs explicit 16px dimensions so flex and narrow layouts cannot squeeze them.
- Add an “always scroll to bottom” icon toggle in the log viewer header. Default off; persist the choice across app restarts; scroll immediately when enabled and after log entries change.

## Non-goals and constraints

- Keep account actions, restart flow, log contents, updater, and existing preferences unchanged.
- Do not add dependencies or alter backend storage. Persist the new UI-only preference in Tauri webview `localStorage`.
- Keep the app usable at narrow widths; allow the existing table scroll area and toolbar wrapping to handle overflow.

## Approaches

- **UI-only change (chosen):** copy action in the email cell, shared fixed icon sizing, and a localStorage-backed log-follow control. This matches existing clipboard and theme-preference patterns with minimal cross-layer risk.
- **Backend preference:** add a persisted setting to Rust state and snapshot APIs. This duplicates a UI-only preference across the bridge and requires backend schema/API changes.
- **Hide or collapse actions at narrow widths:** reduces width but removes discoverable account actions and does not fix spinner geometry.

## Interaction details

- Copy button sits immediately after the email, before status badges. It copies the full email even when the visible label truncates.
- Copy success reports `Copied {email}.`; clipboard failure reports a clear error.
- Log follow uses `ArrowDownToLine`, placed in a dedicated “Activity log” header at the upper-right of the viewer. `aria-pressed` and selected styling show its state.
- Enabling follow jumps to the current bottom. New log entries keep the viewport at the bottom only while enabled.
- Store the preference under `cm-logs-auto-scroll`; missing or inaccessible storage defaults to off.

## Release

- This is additive user-facing functionality, so release as `v2.4.0`.
- Add a curated changelog entry. The tag triggers the existing cross-platform release workflow and updater manifest generation.
