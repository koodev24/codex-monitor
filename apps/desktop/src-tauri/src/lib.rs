// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use tauri::Emitter;
use tauri::Manager;
pub mod api;
pub mod auth;
pub mod commands;
pub mod oauth_login;
pub mod formatters;
#[cfg(feature = "legacy-migrate")]
pub mod legacy;
pub mod models;
pub mod state;
pub mod storage;
pub mod watcher;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage({
            let (monitor, log_path) = commands::build_monitor_state();
            commands::AppState {
                inner: std::sync::Mutex::new(monitor),
                log_path,
                login: std::sync::Mutex::new(None),
            }
        })
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = app.primary_monitor() {
                    let scale = monitor.scale_factor();
                    if scale > 0.0 {
                        let area = monitor.work_area().size;
                        let width = (area.width as f64 / scale * 0.8).round().max(540.0);
                        let height = (area.height as f64 / scale * 0.8).round().max(320.0);
                        if let Err(e) = win.set_size(tauri::Size::Logical(tauri::LogicalSize {
                            width,
                            height,
                        })) {
                            eprintln!("initial window size failed: {e}");
                        }
                        if let Err(e) = win.center() {
                            eprintln!("initial window center failed: {e}");
                        }
                    }
                }
            }
            let handle = app.handle().clone();
            let auth = crate::auth::AuthFileService::with_defaults();
            let watch_dir = auth
                .auth_file_path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let target = auth.auth_file_path.clone();
            let mut watcher = watcher::AuthFileWatcher::new(watch_dir, target, move || {
                let _ = handle.emit("auth-file-changed", ());
            });
            if let Err(e) = watcher.start() {
                eprintln!("auth watcher failed to start: {e}");
            }
            std::mem::forget(watcher);
            Ok(())
        })
        .invoke_handler(commands::all_commands())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
