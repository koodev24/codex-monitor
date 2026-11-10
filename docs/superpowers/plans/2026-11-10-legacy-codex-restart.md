# Restore the pre-hardening Codex restart flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the v2.3.0/v2.3.1 account-switch restart sequence in the current Tauri app without changing unrelated current behavior.

**Architecture:** Keep the existing `restart_codex(State<AppState>)` Tauri command and frontend call path. Replace only the macOS quit, exit polling, settle, and reopen helpers with the historical name-based sequence, while retaining current activity-log calls and user-facing status handling.

**Tech Stack:** Rust/Tauri 2 backend, React/TypeScript frontend, Cargo tests, npm/Vite checks, GitHub Actions tag release.

**Spec:** `docs/superpowers/specs/2026-11-10-legacy-codex-restart-design.md`

## Global Constraints

- Keep the restart command macOS-only and preserve its existing IPC signature.
- Use the v2.3.0/v2.3.1 timing: 40 × 250 ms exit polls, 500 ms settle, two `open -a Codex` attempts with a 1.5 s retry gap.
- Do not change account switching, auth, quota, storage, updater, or UI behavior.
- Do not add unrelated untracked files to Git.
- Release through the existing `v2*` tag workflow; do not hand-edit generated release artifacts.

## Review Focus

- A successful quit must reopen Codex promptly without the current 2 s + 15 s + 5 s delay chain; test the command plan and retry count.
- A still-running Codex process must stop the reopen step after the 10 s legacy timeout; preserve the existing user-facing error.
- A failed first `open -a Codex` must get exactly one 1.5 s retry and then a useful error.
- Non-macOS builds must retain the current unsupported-platform result.
- Current frontend busy/status/log behavior must remain unchanged.

---

### Task 1: Pin the historical restart command plan with a failing test

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs` test module

**Interfaces:**
- Produces: a testable helper describing the legacy open command and retry schedule.

- [ ] **Step 1: Write the failing test**

  Add a unit test asserting that the legacy plan contains two `open -a Codex` attempts, that attempt one has no delay, attempt two has a 1,500 ms delay, and the exit poll budget is 40 iterations at 250 ms.

- [ ] **Step 2: Run the focused Rust test to verify it fails**

  Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml legacy_restart_plan`

  Expected: compile failure because the new helper is not implemented.

### Task 2: Replace only the restart implementation

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs` macOS restart helpers and command

**Interfaces:**
- Consumes: the legacy plan from Task 1.
- Produces: the existing `restart_codex(State<AppState>) -> Result<String, String>` command and unchanged frontend IPC behavior.

- [ ] **Step 1: Implement the minimal helper and rerun the focused test**

  Add the command-plan helper with the historical values; confirm the focused test passes.

- [ ] **Step 2: Implement the legacy runtime sequence**

  Keep current start/reopen logging, but use name-based AppleScript quit, `pgrep -x Codex` polling for 40 × 250 ms, a 500 ms settle, and two `open -a Codex` attempts separated by 1,500 ms. Remove the auth-file guard, bundle-path discovery, 2-second settle, long grace period, stability watch, and multi-form launch loop from this command only.

- [ ] **Step 3: Run the Rust test suite and clippy**

  Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`

  Run: `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`

### Task 3: Verify the unchanged application surface and release readiness

**Files:**
- Inspect: `apps/desktop/src/App.tsx`, `apps/desktop/src/store/snapshotSlice.ts`, `.github/workflows/release-v2-tauri.yml`
- Modify only if required: release notes/changelog location already present in the repository

- [ ] **Step 1: Run frontend build and tests**

  Run: `npm --prefix apps/desktop run build`

  Run: `npm --prefix apps/desktop test`

- [ ] **Step 2: Review the diff and untracked-file boundary**

  Confirm the diff contains only the restart implementation, its regression test, the design/plan documents, and intentional release metadata. Confirm unrelated untracked files are not staged.

- [ ] **Step 3: Commit the atomic fix**

  Use: `git add apps/desktop/src-tauri/src/commands.rs docs/superpowers/specs/2026-11-10-legacy-codex-restart-design.md docs/superpowers/plans/2026-11-10-legacy-codex-restart.md`

  Commit message: `fix: restore legacy Codex restart flow`

- [ ] **Step 4: Tag the next patch release and push it**

  Create annotated tag `v2.3.10` from the verified commit and push the commit and tag to `origin`. The existing release workflow will derive versions and publish updater artifacts from the tag.

- [ ] **Step 5: Verify the remote release trigger**

  Confirm the tag exists on GitHub and the `Release v2 (Tauri)` workflow is running or has completed for `v2.3.10`; report the release URL and any CI result.
