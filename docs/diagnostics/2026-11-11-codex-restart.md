# Codex restart investigation — v2.4.2

## Evidence

The earliest Python account-switch restart was introduced by `cf355da`, shipped in v1.3.0. It sent AppleScript quit, delayed one second, then activated by the display name "Codex". Later Python releases added bundle-ID/path fallbacks. No user confirmation or recorded runtime result identifies one historical release as reliable. The v2.3.10 rollback selected v2.3.0/v2.3.1 because they predated hardening; that was not evidence of a working restart.

On the investigated Mac, native AppKit lookup for `com.openai.codex` returned `/Applications/ChatGPT.app`, with main executable `/Applications/ChatGPT.app/Contents/MacOS/ChatGPT`. The previous `pgrep -x Codex` check cannot identify that process. Name-based quit/open can also address the wrong application or fail to find it. The previous main-only check did not wait for desktop helpers, and an accepted `open` request was reported as success without checking the new process.

## Prior art

Two other account-switch tools were read to compare their restart behavior. Both identify the running app through AppKit (`NSRunningApplication`/`NSWorkspace`) with bundle ID `com.openai.codex` rather than a display name; one also retries relaunch because macOS can briefly retain a terminated instance, and scopes helper termination. No code was copied.

## Change

Resolve the running bundle and executable through AppKit before termination. Request native termination without sending Apple Events to an application name. Track this user's executable paths inside the captured bundle, including helpers. If normal shutdown stalls, send SIGTERM, then SIGKILL only to remaining processes inside that bundle. Never reopen over processes that could not be stopped.

Open the captured bundle through LaunchServices. Require the same main PID to remain visible for one second. Retry at most three times, waiting up to ten seconds per accepted launch, and clear partial starts before retrying. Helpers alone do not count as a successful launch. Native lookup/open commands have a four-second timeout. A backend mutex rejects concurrent restart requests. Lifecycle work runs on a blocking worker rather than the UI thread.

## Validation

- Three initial regressions failed against the old behavior: renamed process detection, leftover helpers, and false launch success. They pass with the replacement.
- Full Rust suite: 54 passed, one GUI integration test excluded by default.
- Explicit GUI integration test: passed. A temporary app with a distinct bundle ID, spaces in its path, and renamed executable cold-started, quit through AppKit, and relaunched with a different main PID. It never targets the current Codex session.
- Frontend production build and all 16 frontend tests passed with Node 24.
- `cargo clippy --all-targets -- -D warnings` passed.

The real Codex session was inspected read-only. Its live account-switch/rendering behavior was not exercised because terminating it would interrupt this task. Process survival verifies launch, not complete application UI readiness.

Run the GUI fixture on a logged-in Mac:

```sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml native_restart_relaunches_renamed_fixture_app -- --ignored --nocapture
```
