use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// (size_bytes, mtime_nanos). None when the file is missing.
/// Used by both the watcher thread and the poll fallback so rapid
/// successive writes coalesce (mirrors the 250ms settle + signature poll).
pub fn file_signature(path: &Path) -> Option<(u64, i128)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some((meta.len(), mtime.as_nanos() as i128))
}

fn event_touches_target(event: &Event, target: &Path) -> bool {
    event.paths.iter().any(|p| same_file(p, target))
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if a.file_name().is_some() && a.file_name() == b.file_name() {
        let ap = a.parent().and_then(|p| p.canonicalize().ok());
        let bp = b.parent().and_then(|p| p.canonicalize().ok());
        if ap.is_some() && ap == bp {
            return true;
        }
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

enum Ctrl {
    Stop,
}

/// Debounced auth-file watcher. Mirrors watcher.py (watchdog Observer) plus
/// the ui.py settle logic: modify/create/delete/move events affecting the
/// target file collapse into one callback after 250ms of quiet.
/// Delete events are delivered too — the callback decides how to degrade
/// (carry-over: old #7 crashed with Errno 2 here).
pub struct AuthFileWatcher {
    watch_dir: PathBuf,
    target_file: PathBuf,
    callback: Arc<Mutex<Box<dyn Fn() + Send + 'static>>>,
    worker: Option<JoinHandle<()>>,
    ctrl: Option<Sender<Ctrl>>,
    _watcher: Option<RecommendedWatcher>,
}

impl AuthFileWatcher {
    pub fn new(
        watch_dir: PathBuf,
        target_file: PathBuf,
        callback: impl Fn() + Send + 'static,
    ) -> Self {
        Self {
            watch_dir,
            target_file,
            callback: Arc::new(Mutex::new(Box::new(callback))),
            worker: None,
            ctrl: None,
            _watcher: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        std::fs::create_dir_all(&self.watch_dir).map_err(|e| e.to_string())?;
        let (tx_event, rx_event): (Sender<notify::Result<Event>>, Receiver<notify::Result<Event>>) =
            mpsc::channel();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
            let _ = tx_event.send(res);
        })
        .map_err(|e| e.to_string())?;
        watcher
            .watch(&self.watch_dir, RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;
        watcher
            .configure(Config::default().with_poll_interval(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;

        let (tx_ctrl, rx_ctrl) = mpsc::channel::<Ctrl>();
        let target = self.target_file.clone();
        let callback = Arc::clone(&self.callback);
        let worker = thread::spawn(move || {
            let settle = Duration::from_millis(250);
            let mut pending = false;
            loop {
                if rx_ctrl.try_recv().is_ok() {
                    break;
                }
                match rx_event.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(event)) => {
                        if matches!(
                            event.kind,
                            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                        ) && event_touches_target(&event, &target)
                        {
                            pending = true;
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if pending {
                    // Trailing-edge debounce: keep waiting while events keep
                    // arriving, so a burst always coalesces into one callback
                    // no matter how the OS spreads out delivery. Capped so
                    // sustained churn can delay but never starve the callback.
                    let mut quiet_windows = 0;
                    let stopped = loop {
                        thread::sleep(settle);
                        let mut more = false;
                        while rx_event.try_recv().is_ok() {
                            more = true;
                        }
                        if rx_ctrl.try_recv().is_ok() {
                            break true;
                        }
                        quiet_windows += 1;
                        if !more || quiet_windows >= 8 {
                            break false;
                        }
                    };
                    if stopped {
                        break;
                    }
                    pending = false;
                    if let Ok(cb) = callback.lock() {
                        cb();
                    }
                }
            }
        });

        self.worker = Some(worker);
        self.ctrl = Some(tx_ctrl);
        self._watcher = Some(watcher);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.ctrl.take() {
            let _ = tx.send(Ctrl::Stop);
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self._watcher = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;

    fn temp_target(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("codex-watch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), dir.join("auth.json"))
    }

    #[test]
    fn modify_and_delete_emit_exactly_batched_events() {
        let (dir, target) = temp_target("events");
        std::fs::write(&target, "{}").unwrap();
        let (tx, rx) = std_mpsc::channel();
        let target_for_cb = target.clone();
        let mut w = AuthFileWatcher::new(dir, target.clone(), move || {
            let _ = tx.send(file_signature(&target_for_cb));
        });
        w.start().unwrap();
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(&target, "{\"a\":1}").unwrap();
        std::fs::write(&target, "{\"a\":2}").unwrap();
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(first.is_some());
        // Rapid writes coalesce: no second event within the settle window.
        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());

        std::fs::remove_file(&target).unwrap();
        let gone = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(gone, None);
        w.stop();
    }

    #[test]
    fn signature_tracks_size_and_mtime() {
        let (_dir, target) = temp_target("sig");
        assert_eq!(file_signature(&target), None);
        std::fs::write(&target, "{}").unwrap();
        let a = file_signature(&target).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        std::fs::write(&target, "{\"x\":1}").unwrap();
        let b = file_signature(&target).unwrap();
        assert_ne!(a, b);
    }
}
