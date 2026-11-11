//! Codex desktop lifecycle, independent of its display/executable name.
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
struct Process {
    pid: u32,
    main: bool,
}

fn bundle_processes(table: &str, bundle: &Path, executable: &Path) -> Vec<Process> {
    table
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (pid, command) = line.split_once(char::is_whitespace)?;
            let command = Path::new(command.trim());
            let pid = pid.parse().ok()?;
            (command.starts_with(bundle.join("Contents"))).then_some(Process {
                pid,
                main: command == executable,
            })
        })
        .collect()
}

trait Desktop {
    fn processes(&mut self) -> Result<Vec<Process>, String>;
    fn quit(&mut self) -> Result<(), String>;
    fn stop(&mut self, force: bool) -> Result<(), String>;
    fn launch(&mut self) -> Result<(), String>;
    fn sleep(&mut self, duration: Duration);
    fn log(&mut self, message: &str);
}

fn wait_until_stopped(desktop: &mut impl Desktop, polls: usize) -> Result<bool, String> {
    for _ in 0..polls {
        if desktop.processes()?.is_empty() {
            return Ok(true);
        }
        desktop.sleep(Duration::from_millis(250));
    }
    Ok(desktop.processes()?.is_empty())
}

fn stop_remaining(desktop: &mut impl Desktop) -> Result<(), String> {
    if wait_until_stopped(desktop, 16)? {
        return Ok(());
    }
    desktop.log("Restart: stopping remaining Codex bundle processes (SIGTERM).");
    desktop.stop(false)?;
    if wait_until_stopped(desktop, 4)? {
        return Ok(());
    }
    desktop.log("Restart: force-stopping remaining Codex bundle processes (SIGKILL).");
    desktop.stop(true)?;
    if wait_until_stopped(desktop, 8)? {
        return Ok(());
    }
    Err("Codex processes did not stop; reopen cancelled to avoid overlapping instances.".into())
}

fn restart(desktop: &mut impl Desktop) -> Result<(), String> {
    if let Err(error) = desktop.quit() {
        desktop.log(&format!(
            "Restart: native quit failed ({error}); stopping bundle processes."
        ));
    }
    stop_remaining(desktop)?;
    desktop.sleep(Duration::from_millis(500));
    let mut last_error = "launch accepted, but Codex main process did not stay running".to_string();
    for attempt in 1..=3 {
        desktop.log(&format!(
            "Restart: launching resolved Codex bundle (attempt {attempt}/3)."
        ));
        match desktop.launch() {
            Err(error) => last_error = error,
            Ok(()) => {
                // LaunchServices acceptance is not proof of a running app. Require
                // the same main PID for a full second, never a leftover helper.
                let mut previous = None;
                let mut stable = 0;
                for _ in 0..40 {
                    let main = desktop
                        .processes()?
                        .into_iter()
                        .find(|process| process.main)
                        .map(|p| p.pid);
                    if main.is_some() && main == previous {
                        stable += 1;
                    } else {
                        stable = 0;
                    }
                    if stable >= 4 {
                        desktop.log("Restart: Codex main process is running; launch verified.");
                        return Ok(());
                    }
                    previous = main;
                    desktop.sleep(Duration::from_millis(250));
                }
            }
        }
        if attempt < 3 {
            // Clear any partial start before retrying. Never stack launches.
            stop_remaining(desktop)?;
            desktop.sleep(Duration::from_millis(500));
        }
    }
    Err(format!("Could not restart Codex: {last_error}. Open Codex manually; account switch remains applied."))
}

#[cfg(target_os = "macos")]
pub fn run(log: impl Fn(&str)) -> Result<(), String> {
    use std::sync::Mutex;
    static RESTART_LOCK: Mutex<()> = Mutex::new(());
    let _guard = RESTART_LOCK
        .try_lock()
        .map_err(|_| "Codex restart already in progress.".to_string())?;
    let mut desktop = native::MacDesktop::resolve(log)?;
    restart(&mut desktop)
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};
    use std::time::Instant;

    // AppKit identifies renamed apps without Apple Events automation permission.
    // Adapted at the architectural level from the MIT Codex Account Switcher:
    // https://github.com/liuzhao1225/codex-account-switcher/blob/main/Sources/CodexAccountSwitcher/DesktopController.swift
    const APPKIT: &str = r#"
ObjC.import('AppKit');
function run(args) {
    var apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(args[1]);
    if (args[0] === 'quit') {
        for (var i = 0; i < apps.count; i++) {
            var app = apps.objectAtIndex(i);
            if (ObjC.unwrap(app.bundleURL.path) === args[2]) app.terminate;
        }
        return 'quit requested';
    }
    var url = apps.count ? apps.objectAtIndex(0).bundleURL :
        $.NSWorkspace.sharedWorkspace.URLForApplicationWithBundleIdentifier(args[1]);
    if (!url) throw new Error('Codex desktop is not installed (com.openai.codex).');
    var bundle = $.NSBundle.bundleWithURL(url);
    return JSON.stringify({bundle: ObjC.unwrap(url.path), executable: ObjC.unwrap(bundle.executablePath)});
}
"#;

    fn bounded_output(command: &mut Command) -> Result<Output, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return child.wait_with_output().map_err(|e| e.to_string());
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("macOS lifecycle command timed out after 4s".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn appkit(args: &[&str]) -> Result<Output, String> {
        bounded_output(
            Command::new("/usr/bin/osascript")
                .args(["-l", "JavaScript", "-e", APPKIT])
                .args(args),
        )
    }

    fn successful(output: Output) -> Result<Output, String> {
        if output.status.success() {
            Ok(output)
        } else {
            Err(format!(
                "macOS command failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    pub(super) struct MacDesktop<F> {
        bundle: PathBuf,
        executable: PathBuf,
        bundle_id: String,
        log: F,
    }

    impl<F: Fn(&str)> MacDesktop<F> {
        pub(super) fn resolve(log: F) -> Result<Self, String> {
            let output = successful(appkit(&["resolve", "com.openai.codex"])?)?;
            #[derive(serde::Deserialize)]
            struct Location {
                bundle: PathBuf,
                executable: PathBuf,
            }
            let location: Location = serde_json::from_slice(&output.stdout)
                .map_err(|e| format!("Cannot resolve Codex bundle: {e}"))?;
            if !location.bundle.is_dir()
                || !location.executable.is_file()
                || !location
                    .executable
                    .starts_with(location.bundle.join("Contents/MacOS"))
            {
                return Err("Codex bundle has no valid main executable.".into());
            }
            log(&format!(
                "Restart: resolved bundle {} (main {}).",
                location.bundle.display(),
                location.executable.display()
            ));
            Ok(Self {
                bundle: location.bundle,
                executable: location.executable,
                bundle_id: "com.openai.codex".into(),
                log,
            })
        }
    }

    impl<F: Fn(&str)> Desktop for MacDesktop<F> {
        fn processes(&mut self) -> Result<Vec<Process>, String> {
            // -x includes only this user's processes. comm contains executable
            // paths, not arbitrary command arguments that might mention Codex.
            let output = successful(
                Command::new("/bin/ps")
                    .args(["-x", "-o", "pid=,comm="])
                    .output()
                    .map_err(|e| e.to_string())?,
            )?;
            Ok(bundle_processes(
                &String::from_utf8_lossy(&output.stdout),
                &self.bundle,
                &self.executable,
            ))
        }
        fn quit(&mut self) -> Result<(), String> {
            self.log("Restart: requesting native Codex termination.");
            successful(appkit(&[
                "quit",
                &self.bundle_id,
                self.bundle.to_str().ok_or("Codex path is not UTF-8")?,
            ])?)
            .map(|_| ())
        }
        fn stop(&mut self, force: bool) -> Result<(), String> {
            // Refresh identities before signalling; never use broad pkill names.
            let mut processes = self.processes()?;
            processes.sort_by_key(|process| process.main); // Helpers before main.
            for process in processes {
                let output = Command::new("/bin/kill")
                    .arg(if force { "-KILL" } else { "-TERM" })
                    .arg(process.pid.to_string())
                    .output()
                    .map_err(|e| e.to_string())?;
                if !output.status.success()
                    && self.processes()?.iter().any(|p| p.pid == process.pid)
                {
                    return Err(format!("Could not stop Codex process {}", process.pid));
                }
            }
            Ok(())
        }
        fn launch(&mut self) -> Result<(), String> {
            // LaunchServices selects the captured bundle, without an extra
            // instance (-n), stale Electron arguments, or direct binary launch.
            successful(bounded_output(
                Command::new("/usr/bin/open").arg("-a").arg(&self.bundle),
            )?)
            .map(|_| ())
        }
        fn sleep(&mut self, duration: Duration) {
            std::thread::sleep(duration);
        }
        fn log(&mut self, message: &str) {
            (self.log)(message);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "requires a logged-in macOS GUI session and clang; uses an isolated fixture app"]
        fn native_restart_relaunches_renamed_fixture_app() {
            use std::fs;
            let bundle_id = format!("com.koodev24.restart-fixture-{}", std::process::id());
            let dir = std::env::temp_dir()
                .join(&bundle_id)
                .join("Renamed Fixture.app");
            fs::create_dir_all(dir.join("Contents/MacOS")).unwrap();
            let bundle = fs::canonicalize(dir).unwrap();
            let executable = bundle.join("Contents/MacOS/RenamedHost");
            let source = bundle.join("fixture.m");
            fs::write(&source, "#import <AppKit/AppKit.h>\nint main(void) { @autoreleasepool { [[NSApplication sharedApplication] run]; } return 0; }").unwrap();
            assert!(Command::new("/usr/bin/clang")
                .args(["-framework", "AppKit"])
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .unwrap()
                .success());
            fs::write(bundle.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{bundle_id}</string><key>CFBundleExecutable</key><string>RenamedHost</string><key>CFBundleName</key><string>Renamed Fixture</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#)).unwrap();
            // Cleanup always targets only this test's unique bundle, even on panic.
            struct Fixture(MacDesktop<fn(&str)>);
            impl Drop for Fixture {
                fn drop(&mut self) {
                    let _ = self.0.stop(true);
                    let _ = wait_until_stopped(&mut self.0, 8);
                    let _ = fs::remove_dir_all(self.0.bundle.parent().unwrap());
                }
            }
            let mut fixture = Fixture(MacDesktop {
                bundle,
                executable,
                bundle_id,
                log: |line| {
                    assert!(!line.contains("native quit failed"), "{line}");
                    eprintln!("{line}");
                },
            });
            restart(&mut fixture.0).expect("cold launch must work");
            let first = fixture
                .0
                .processes()
                .unwrap()
                .into_iter()
                .find(|p| p.main)
                .unwrap()
                .pid;
            restart(&mut fixture.0).expect("native quit and relaunch must work");
            let second = fixture
                .0
                .processes()
                .unwrap()
                .into_iter()
                .find(|p| p.main)
                .unwrap()
                .pid;
            assert_ne!(first, second, "restart must create a new main process");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_renamed_main_and_helpers_without_matching_other_apps() {
        let table = "12 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT\n13 /Applications/ChatGPT.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper\n14 /Applications/ChatGPT.app.copy/Contents/MacOS/ChatGPT\n15 /usr/local/bin/codex\n";
        assert_eq!(
            bundle_processes(
                table,
                Path::new("/Applications/ChatGPT.app"),
                Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT")
            ),
            vec![
                Process {
                    pid: 12,
                    main: true
                },
                Process {
                    pid: 13,
                    main: false
                }
            ]
        );
    }

    #[derive(Default)]
    struct FakeDesktop {
        launched: usize,
        never_starts: bool,
        helpers: bool,
        refuse_stop: bool,
        force_stops: usize,
        polls: usize,
        fail_launches: usize,
        quit_error: bool,
        process_error: bool,
        start_delay: usize,
    }

    impl Desktop for FakeDesktop {
        fn processes(&mut self) -> Result<Vec<Process>, String> {
            self.polls += 1;
            if self.process_error {
                return Err("process inspection failed".into());
            }
            if self.launched > 0 && self.start_delay > 0 {
                self.start_delay -= 1;
                return Ok(vec![]);
            }
            if self.helpers {
                return Ok(vec![Process {
                    pid: 2,
                    main: false,
                }]);
            }
            Ok(
                if self.launched > self.fail_launches && !self.never_starts {
                    vec![Process { pid: 3, main: true }]
                } else {
                    vec![]
                },
            )
        }
        fn quit(&mut self) -> Result<(), String> {
            if self.quit_error {
                Err("quit rejected".into())
            } else {
                Ok(())
            }
        }
        fn stop(&mut self, force: bool) -> Result<(), String> {
            if force {
                self.force_stops += 1;
            }
            if force && !self.refuse_stop {
                self.helpers = false;
            }
            Ok(())
        }
        fn launch(&mut self) -> Result<(), String> {
            assert!(!self.helpers, "must never launch while old helpers remain");
            self.launched += 1;
            if self.launched <= self.fail_launches {
                Err("LaunchServices busy".into())
            } else {
                Ok(())
            }
        }
        fn sleep(&mut self, _: Duration) {}
        fn log(&mut self, _: &str) {}
    }

    #[test]
    fn accepted_open_without_main_process_is_not_success() {
        let mut desktop = FakeDesktop {
            never_starts: true,
            ..Default::default()
        };
        assert!(restart(&mut desktop).is_err());
        assert_eq!(desktop.launched, 3);
    }

    #[test]
    fn clears_stuck_helpers_before_relaunch() {
        let mut desktop = FakeDesktop {
            helpers: true,
            ..Default::default()
        };
        restart(&mut desktop).unwrap();
        assert_eq!(desktop.force_stops, 1);
        assert_eq!(desktop.launched, 1);
    }
    #[test]
    fn refuses_launch_when_old_helpers_cannot_be_stopped() {
        let mut desktop = FakeDesktop {
            helpers: true,
            refuse_stop: true,
            ..Default::default()
        };
        assert!(restart(&mut desktop).is_err());
        assert_eq!(desktop.launched, 0);
    }

    #[test]
    fn inspection_failure_never_means_app_has_exited() {
        let mut desktop = FakeDesktop {
            process_error: true,
            ..Default::default()
        };
        assert!(restart(&mut desktop).is_err());
        assert_eq!(desktop.launched, 0);
    }

    #[test]
    fn waits_for_slow_launch_without_stacking_open_requests() {
        let mut desktop = FakeDesktop {
            start_delay: 20,
            ..Default::default()
        };
        restart(&mut desktop).unwrap();
        assert_eq!(desktop.launched, 1);
        assert!(desktop.polls >= 25);
    }

    #[test]
    fn retries_launchservices_errors() {
        let mut desktop = FakeDesktop {
            fail_launches: 2,
            ..Default::default()
        };
        restart(&mut desktop).unwrap();
        assert_eq!(desktop.launched, 3);
    }

    #[test]
    fn quit_error_does_not_prevent_stopping_and_relaunch() {
        let mut desktop = FakeDesktop {
            quit_error: true,
            helpers: true,
            ..Default::default()
        };
        restart(&mut desktop).unwrap();
        assert_eq!(desktop.force_stops, 1);
        assert_eq!(desktop.launched, 1);
    }
}
