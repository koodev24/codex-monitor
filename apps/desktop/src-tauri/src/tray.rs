//! Menu-bar presence: a template icon, a native menu, and a small React
//! popover that mirrors the main window's account list.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use crate::commands::Snapshot;

pub const TRAY_WINDOW: &str = "tray";
const TRAY_ID: &str = "codex-monitor-tray";
const TRAY_REFRESH_EVENT: &str = "tray-refresh";
const OPEN_ITEM_ID: &str = "open-main";
const QUIT_ITEM_ID: &str = "quit-app";
const TRAY_WIDTH: f64 = 320.0;
const TRAY_HEIGHT: f64 = 460.0;
/// Gap between the menu-bar item and the popover, in physical pixels.
const TRAY_GAP: f64 = 6.0;

/// Set when a mouse-down closes the popover. The matching mouse-up must not
/// immediately reopen it, or the tray icon can never close its own popover.
static SUPPRESS_NEXT_CLICK: AtomicBool = AtomicBool::new(false);

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    create_popover(app)?;

    let menu = build_menu(app, current_locale(app))?;

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Codex Monitor")
        .title("Codex")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_icon_event);

    // macOS shows the text label on its own: no menu-bar glyph competes with
    // the quota numbers.
    #[cfg(not(target_os = "macos"))]
    let builder = match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone()),
        None => builder,
    };

    builder.build(app)?;
    Ok(())
}

fn current_locale(app: &AppHandle) -> &'static str {
    app.state::<crate::commands::AppState>()
        .inner
        .lock()
        .map(|state| crate::state::resolved_locale(&state.locale))
        .unwrap_or("en")
}

fn menu_labels(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "zh-CN" => ("打开 Codex Monitor", "退出 Codex Monitor"),
        "zh-TW" => ("開啟 Codex Monitor", "結束 Codex Monitor"),
        "ja" => ("Codex Monitor を開く", "Codex Monitor を終了"),
        _ => ("Open Codex Monitor", "Quit Codex Monitor"),
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>, locale: &str) -> tauri::Result<Menu<R>> {
    let (open_label, quit_label) = menu_labels(locale);
    let menu = Menu::new(app)?;
    menu.append(
        &MenuItemBuilder::with_id(OPEN_ITEM_ID, open_label)
            .accelerator("CmdOrCtrl+O")
            .build(app)?,
    )?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItemBuilder::with_id(QUIT_ITEM_ID, quit_label).build(app)?)?;
    Ok(menu)
}

/// Relabel the native tray menu after the interface language changes.
pub fn set_locale(app: &AppHandle, locale: &str) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Err("Menu bar item is unavailable.".into());
    };
    let menu = build_menu(app, locale).map_err(|e| format!("Tray menu update failed: {e}"))?;
    tray.set_menu(Some(menu))
        .map_err(|e| format!("Tray menu update failed: {e}"))
}

/// Show or hide the menu-bar item. Hiding also dismisses the popover so no
/// orphaned window is left behind when the item disappears.
pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    if !visible {
        hide_popover(app);
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Err("Menu bar item is unavailable.".into());
    };
    tray.set_visible(visible)
        .map_err(|e| format!("Menu bar item update failed: {e}"))
}

/// Update the menu-bar text with the active account's remaining quota for both
/// windows. Called from `get_snapshot`, which the UI polls, so the title stays
/// live.
pub fn update_title(app: &AppHandle, snapshot: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    // A macOS menu-bar item with neither icon nor text has no clickable area.
    // Keep a plain label until the first quota reading arrives so the item can
    // still be found (and the popover opened) on a fresh install.
    let title = title_or_fallback(snapshot);
    let tooltip = tooltip_text(snapshot);

    let _ = tray.set_tooltip(Some(tooltip));
    #[cfg(not(target_os = "windows"))]
    {
        if let Err(error) = tray.set_title(Some(&title)) {
            eprintln!("tray title update failed: {error}");
        }
    }
    #[cfg(target_os = "windows")]
    let _ = (title, tray);
}

/// Menu-bar label, ordered short window first: `5h·100% 7d·42%`.
/// A missing window shortens the label rather than showing a guessed number.
fn title_text(snapshot: &Snapshot) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(short) = short_remaining(snapshot) {
        parts.push(format!("5h·{short}%"));
    }
    if let Some(weekly) = weekly_remaining(snapshot) {
        parts.push(format!("7d·{weekly}%"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Same as `title_text`, but never empty: the menu-bar item must remain visible
/// before the first quota refresh (and when an account has no usage yet).
fn title_or_fallback(snapshot: &Snapshot) -> String {
    title_text(snapshot).unwrap_or_else(|| "Codex".to_string())
}

fn tooltip_text(snapshot: &Snapshot) -> String {
    let Some(email) = snapshot.current_email.as_deref() else {
        return "Codex Monitor".to_string();
    };
    let mut parts = vec![format!("Codex Monitor — {email}")];
    if let Some(short) = short_remaining(snapshot) {
        parts.push(format!("5h {short}% left"));
    }
    if let Some(weekly) = weekly_remaining(snapshot) {
        parts.push(format!("7d {weekly}% left"));
    }
    parts.join(" · ")
}

/// Remaining percent for the rolling 5h window. Unlike the weekly window this
/// never falls back to `primary_window`: those fields describe a different
/// quota and would print the same number twice in the menu bar.
fn short_remaining(snapshot: &Snapshot) -> Option<u8> {
    remaining_from(snapshot, |usage| {
        usage.short_window.as_ref().and_then(|w| w.used_percent)
    })
}

fn weekly_remaining(snapshot: &Snapshot) -> Option<u8> {
    remaining_from(snapshot, |usage| {
        usage
            .weekly_window
            .as_ref()
            .or(usage.secondary_window.as_ref())
            .or(usage.primary_window.as_ref())
            .and_then(|window| window.used_percent)
            .or(usage.used_percent)
    })
}

/// Turn an `used_percent` reading for the active account into whole percent
/// remaining, clamped to a sane range. Non-finite values are treated as absent.
fn remaining_from(
    snapshot: &Snapshot,
    used_percent: impl Fn(&crate::models::AccountUsage) -> Option<f64>,
) -> Option<u8> {
    let email = snapshot.current_email.as_deref()?;
    let used = used_percent(snapshot.accounts.get(email)?)?;
    if !used.is_finite() {
        return None;
    }
    Some((100.0 - used).round().clamp(0.0, 100.0) as u8)
}

fn create_popover(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(TRAY_WINDOW).is_some() {
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(app, TRAY_WINDOW, WebviewUrl::App("tray.html".into()))
        .title("Codex Monitor")
        .inner_size(TRAY_WIDTH, TRAY_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;

    // Behave like a native menu: dismiss as soon as focus moves elsewhere.
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            if let Some(window) = app_handle.get_webview_window(TRAY_WINDOW) {
                let _ = window.hide();
            }
        }
    });

    Ok(())
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        OPEN_ITEM_ID => show_main_window(app),
        QUIT_ITEM_ID => app.exit(0),
        _ => {}
    }
}

fn handle_tray_icon_event<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state,
        rect,
        ..
    } = event
    {
        let app = tray.app_handle();
        match button_state {
            MouseButtonState::Down => {
                // Hide on press so the click that dismisses the popover does
                // not also count as a fresh "open" on release.
                if let Some(window) = app.get_webview_window(TRAY_WINDOW) {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                        SUPPRESS_NEXT_CLICK.store(true, Ordering::SeqCst);
                    }
                }
            }
            MouseButtonState::Up => {
                if SUPPRESS_NEXT_CLICK.swap(false, Ordering::SeqCst) {
                    return;
                }
                toggle_popover(app, rect);
            }
        }
    }
}

fn toggle_popover<R: Runtime>(app: &AppHandle<R>, rect: tauri::Rect) {
    let Some(window) = app.get_webview_window(TRAY_WINDOW) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    position_near_tray(&window, rect);
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(TRAY_WINDOW, TRAY_REFRESH_EVENT, ());
}

fn rect_physical(rect: tauri::Rect, scale: f64) -> (f64, f64, f64, f64) {
    let (x, y) = match rect.position {
        tauri::Position::Physical(position) => (position.x as f64, position.y as f64),
        tauri::Position::Logical(position) => (position.x * scale, position.y * scale),
    };
    let (width, height) = match rect.size {
        tauri::Size::Physical(size) => (size.width as f64, size.height as f64),
        tauri::Size::Logical(size) => (size.width * scale, size.height * scale),
    };
    (x, y, width, height)
}

/// Anchor the popover to the tray item, not the cursor, then keep it inside
/// the monitor that owns the item. The returned coordinates are physical.
fn popover_position(
    tray: (f64, f64, f64, f64),
    size: (f64, f64),
    monitor: Option<(f64, f64, f64, f64)>,
    below: bool,
) -> (f64, f64) {
    let (tray_x, tray_y, tray_width, tray_height) = tray;
    let (width, height) = size;
    let mut x = tray_x + tray_width / 2.0 - width / 2.0;
    let mut y = if below {
        tray_y + tray_height + TRAY_GAP
    } else {
        tray_y - height - TRAY_GAP
    };
    if let Some((monitor_x, monitor_y, monitor_width, monitor_height)) = monitor {
        x = x.max(monitor_x);
        if monitor_width >= width {
            x = x.min(monitor_x + monitor_width - width);
        }
        y = y.max(monitor_y);
        if monitor_height >= height {
            y = y.min(monitor_y + monitor_height - height);
        }
    }
    (x, y)
}

fn position_near_tray<R: Runtime>(window: &tauri::WebviewWindow<R>, rect: tauri::Rect) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let tray = rect_physical(rect, scale);
    let size = window.outer_size().ok();
    let width = size.map(|s| s.width as f64).unwrap_or(TRAY_WIDTH);
    let height = size.map(|s| s.height as f64).unwrap_or(TRAY_HEIGHT);

    let center_x = tray.0 + tray.2 / 2.0;
    let center_y = tray.1 + tray.3 / 2.0;
    let monitor = window
        .monitor_from_point(center_x, center_y)
        .ok()
        .flatten()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            (
                position.x as f64,
                position.y as f64,
                size.width as f64,
                size.height as f64,
            )
        });
    let (x, y) = popover_position(tray, (width, height), monitor, cfg!(target_os = "macos"));
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(popover) = app.get_webview_window(TRAY_WINDOW) {
        let _ = popover.hide();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_popover(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(TRAY_WINDOW) {
        let _ = window.hide();
    }
}

pub fn hide(app: &AppHandle) {
    hide_popover(app);
}

pub fn quit(app: &AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn snapshot(email: Option<&str>, used: Option<f64>) -> Snapshot {
        let mut accounts = HashMap::new();
        if let Some(email) = email {
            accounts.insert(
                email.to_string(),
                crate::models::AccountUsage {
                    used_percent: used,
                    ..Default::default()
                },
            );
        }
        Snapshot {
            accounts,
            current_email: email.map(str::to_string),
            auto_fetch: "Off".into(),
            auto_fetch_options: vec![],
            sort_column: None,
            sort_asc: true,
            show_archived: false,
            logs_expanded: false,
            show_menu_bar: true,
            notifications_enabled: true,
            auto_check_updates: true,
            locale: "system".into(),
            auth_file_exists: true,
            backup_emails: vec![],
            app_version: "test".into(),
        }
    }

    fn window(used: f64) -> crate::models::RateLimitWindow {
        crate::models::RateLimitWindow {
            used_percent: Some(used),
            ..Default::default()
        }
    }

    fn windows(email: &str, short: Option<f64>, weekly: Option<f64>) -> Snapshot {
        let mut snapshot = snapshot(Some(email), None);
        let usage = snapshot.accounts.get_mut(email).expect("account");
        usage.short_window = short.map(window);
        usage.weekly_window = weekly.map(window);
        snapshot
    }

    #[test]
    fn menu_bar_title_lists_both_windows() {
        assert_eq!(
            title_text(&windows("a@x.y", Some(0.0), Some(58.0))).as_deref(),
            Some("5h·100% 7d·42%")
        );
    }

    #[test]
    fn menu_bar_title_shortens_when_a_window_is_missing() {
        assert_eq!(
            title_text(&windows("a@x.y", None, Some(58.0))).as_deref(),
            Some("7d·42%")
        );
        assert_eq!(
            title_text(&windows("a@x.y", Some(20.0), None)).as_deref(),
            Some("5h·80%")
        );
        assert_eq!(title_text(&windows("a@x.y", None, None)), None);
    }

    #[test]
    fn menu_bar_keeps_a_clickable_label_before_quota_arrives() {
        let snapshot = windows("a@x.y", None, None);
        assert_eq!(title_or_fallback(&snapshot), "Codex");
        assert_eq!(
            title_or_fallback(&windows("a@x.y", Some(0.0), Some(58.0))),
            "5h·100% 7d·42%"
        );
    }

    #[test]
    fn short_window_does_not_fall_back_to_the_weekly_window() {
        let mut snapshot = windows("a@x.y", None, Some(58.0));
        assert_eq!(short_remaining(&snapshot), None);
        let usage = snapshot.accounts.get_mut("a@x.y").expect("account");
        usage.primary_window = Some(window(10.0));
        assert_eq!(
            short_remaining(&snapshot),
            None,
            "primary is not the 5h window"
        );
        assert_eq!(weekly_remaining(&snapshot), Some(42));
    }

    #[test]
    fn tooltip_names_the_account_and_each_window() {
        let text = tooltip_text(&windows("a@x.y", Some(0.0), Some(58.0)));
        assert_eq!(text, "Codex Monitor — a@x.y · 5h 100% left · 7d 42% left");
        assert_eq!(tooltip_text(&snapshot(None, None)), "Codex Monitor");
    }

    #[test]
    fn weekly_remaining_is_rounded_and_clamped() {
        assert_eq!(
            weekly_remaining(&snapshot(Some("a@x.y"), Some(12.4))),
            Some(88)
        );
        assert_eq!(
            weekly_remaining(&snapshot(Some("a@x.y"), Some(-4.0))),
            Some(100)
        );
        assert_eq!(
            weekly_remaining(&snapshot(Some("a@x.y"), Some(104.0))),
            Some(0)
        );
        assert_eq!(weekly_remaining(&snapshot(Some("a@x.y"), None)), None);
        assert_eq!(weekly_remaining(&snapshot(None, Some(50.0))), None);
        let mut missing = snapshot(Some("a@x.y"), Some(50.0));
        missing.accounts.clear();
        assert_eq!(weekly_remaining(&missing), None);
    }

    #[test]
    fn popover_hangs_below_the_tray_item_and_centers_on_it() {
        let (x, y) = popover_position(
            (1200.0, 0.0, 68.0, 60.0),
            (320.0, 460.0),
            Some((0.0, 0.0, 1728.0, 1117.0)),
            true,
        );
        assert_eq!(x, 1074.0);
        assert_eq!(y, 66.0);
    }

    #[test]
    fn popover_sits_above_a_bottom_taskbar() {
        let (_, y) = popover_position(
            (1200.0, 1000.0, 40.0, 40.0),
            (320.0, 460.0),
            Some((0.0, 0.0, 1920.0, 1080.0)),
            false,
        );
        assert_eq!(y, 534.0);
    }

    #[test]
    fn popover_stays_on_a_monitor_left_of_the_primary() {
        // Negative origins are valid on multi-monitor macOS; never clamp to 0.
        let (x, y) = popover_position(
            (-900.0, 0.0, 68.0, 60.0),
            (320.0, 460.0),
            Some((-1440.0, 0.0, 1440.0, 900.0)),
            true,
        );
        assert_eq!(x, -1026.0);
        assert_eq!(y, 66.0);
    }

    #[test]
    fn popover_is_clamped_to_the_monitor_edges() {
        let (x, _) = popover_position(
            (1900.0, 0.0, 68.0, 60.0),
            (320.0, 460.0),
            Some((0.0, 0.0, 1920.0, 1080.0)),
            true,
        );
        assert_eq!(x, 1600.0);
        let (_, y) = popover_position(
            (10.0, -100.0, 68.0, 60.0),
            (320.0, 460.0),
            Some((0.0, 0.0, 1920.0, 1080.0)),
            true,
        );
        assert_eq!(y, 0.0, "a popover must not start above its monitor");
    }
}
