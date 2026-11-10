# Account Copy, Spinner Sizing, and Log Follow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore account email copying, preserve spinner geometry at narrow widths, and add an optional persistent log-follow control.

**Architecture:** Keep behavior in the existing React/Tauri UI. Use the current Clipboard API and status store, shared icon button sizing, and localStorage for the new UI preference.

**Tech Stack:** React 19, TypeScript, Redux Toolkit, Tailwind CSS, Tauri 2, lucide-react.

**Spec:** `docs/superpowers/specs/2026-11-10-account-copy-spinner-log-follow-design.md`

## Global Constraints

- Icon buttons have a 36px minimum square target; spinner glyphs have explicit 16px dimensions and do not shrink.
- Log follow defaults off and persists in localStorage.
- Store log follow under `cm-logs-auto-scroll`; missing or inaccessible storage defaults off.
- No new dependencies or backend API changes.
- Release version comes from tag `v2.4.0`; changelog describes user impact.

## Review Focus

- Truncated email label: copy must still copy the complete source email.
- Clipboard rejection: show an error status without crashing.
- Narrow action column: icon controls must retain square dimensions and remain reachable through table scrolling.
- Long logs: enabled follow reaches the bottom after each refresh; disabled follow preserves the reader's scroll position.
- Preference restore: enabled/disabled choice survives app reload; missing or invalid storage defaults off.

---

### Task 1: Restore per-account email copy

**Files:**
- Modify: `apps/desktop/src/App.tsx`

- [x] Add a copy icon button beside each email, before current/archive badges.
- [x] Copy the full email with `navigator.clipboard.writeText`, report success through `setStatus`, and report clipboard failure through `setStatus`.
- [x] Verify TypeScript and production build with `cd apps/desktop && npm run build`.

### Task 2: Stabilize icon and spinner geometry

**Files:**
- Modify: `apps/desktop/src/App.tsx`

- [x] Set shared `IconBtn` targets to `icon-lg` with explicit `min-h-9 min-w-9 shrink-0`.
- [x] Set `Spin` glyph dimensions to `size-4 shrink-0` so loading state remains centered and cannot compress.
- [x] Verify production build with `cd apps/desktop && npm run build`.

### Task 3: Add persistent log-follow toggle

**Files:**
- Modify: `apps/desktop/src/App.tsx`

- [x] Add localStorage-backed `alwaysScrollToBottom`, defaulting to false when the key is absent or unreadable.
- [x] Add an “Activity log” header with a right-aligned `ArrowDownToLine` icon button; expose state with `aria-pressed` and selected styling.
- [x] Scroll the log viewport to its current bottom when enabled and after log entries change; do not alter scroll position when disabled.
- [x] Verify production build with `cd apps/desktop && npm run build`.

### Task 4: Document and release

**Files:**
- Modify: `CHANGELOG.md`

- [x] Add v2.4.0 notes for email copy, stable icon sizing, and optional log follow.
- [x] Review staged diff and `git diff --check`; commit focused changes.
- [x] Create and push annotated tag `v2.4.0` to `https://github.com/koodev24/codex-monitor.git`.
- [x] Monitor the existing release workflow; verify platform assets, `latest.json`, and published release.

Release evidence: workflow run `42832643713` passed on macOS, Windows, Ubuntu, and compatibility ZIP. Published `latest.json` reports version `2.4.0`.
