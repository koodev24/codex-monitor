# Restore the pre-hardening Codex restart flow

## Intent

Restore the restart behavior from the last known stable Tauri versions (v2.3.0/v2.3.1) for the account-switch dialog. Those versions quit Codex, wait briefly for the process to leave, then reopen it through LaunchServices. The current app’s other features and release/update behavior stay unchanged.

## Goals

- Make the Restart Codex button follow the v2.3.0/v2.3.1 sequence.
- Keep the current frontend switch dialog, busy state, status messages, and activity logging.
- Preserve all current auth, quota, storage, updater, and UI enhancements outside this restart command.
- Publish the fix as the next patch release so the existing in-app updater can discover it.

## Non-goals

- No changes to account switching, auth-file contents, quota fetching, or updater signing.
- No redesign of the UI or release workflow.
- No new platform-specific restart behavior for Windows or Linux.

## Constraints

- The restart command remains macOS-only and keeps its existing Tauri IPC signature.
- The workspace contains unrelated untracked files; they must not be added or changed.
- The release version must be derived from the next `v2*` tag by the existing CI version script.

## Approaches

1. **Exact legacy Tauri port:** restore the v2.3.0/v2.3.1 timing and command sequence, including name-based quit, `pgrep -x Codex`, 10-second maximum wait, 500 ms settle, and two `open -a Codex` attempts. This is the selected approach because it is the only historical implementation in the Tauri line that predates the reported restart regressions.
2. **Keep the hardened flow and tune timeouts:** retain bundle discovery, auth guarding, multi-round opens, and stability watches while shortening waits. This preserves more safeguards but does not reproduce the known-good behavior and can still report a delayed or skipped reopen.
3. **Restore the v1 Python flow:** use the older Python app’s 8-second AppleScript poll and bundle-id/path/name fallbacks. This is not selected because the current product is the Tauri app and v2.3.0/v2.3.1 provide a direct Tauri reference.

## Implementation call

Modify only `apps/desktop/src-tauri/src/commands.rs`. Keep the current `restart_codex(State<AppState>)` signature and logging, but replace the post-switch restart helpers with the selected legacy sequence. Add a small unit-testable command-plan helper so the retry count and LaunchServices command are pinned without trying to launch Codex during CI. Update the release notes/changelog entry only if the repository already has a release note location; do not touch generated bundles or unrelated untracked files.

## Verification

- Run the focused Rust regression test for the legacy command plan.
- Run the full frontend build/tests and Rust test/clippy commands used by the release workflow.
- Review the final diff and confirm only the restart implementation, its regression test, this design/plan documentation, and release metadata are included.
- Tag the next patch release and confirm the GitHub Actions release workflow is triggered by the `v2*` tag.
