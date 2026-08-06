mod commands;
mod credentials;
mod models;
mod network;
mod providers;
mod settings;
mod system;

use commands::AppState;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Color, Effect, EffectState, EffectsBuilder},
    Manager, PhysicalPosition, PhysicalSize, Position, Rect, Size, WebviewWindow, WindowEvent,
};

/// 面板最近一次 show 的时间戳（毫秒），用于忽略紧随其后的失焦。
static PANEL_SHOWN_AT_MS: AtomicU64 = AtomicU64::new(0);
const BLUR_GRACE_MS: u64 = 350;
/// 主面板正在拖拽排序时抑制失焦 hide，避免拖到一半窗口被关掉。
static PANEL_DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mark_panel_shown() {
    PANEL_SHOWN_AT_MS.store(now_ms(), Ordering::SeqCst);
}

fn should_hide_on_blur() -> bool {
    if PANEL_DRAG_ACTIVE.load(Ordering::SeqCst) {
        return false;
    }
    now_ms().saturating_sub(PANEL_SHOWN_AT_MS.load(Ordering::SeqCst)) >= BLUR_GRACE_MS
}

/// 前端拖拽排序时调用，防止 tray 面板失焦自动隐藏。
#[tauri::command]
fn set_panel_drag_active(active: bool) {
    PANEL_DRAG_ACTIVE.store(active, Ordering::SeqCst);
}

fn physical_tray_size(tray_rect: &Rect, scale: f64) -> PhysicalSize<f64> {
    match tray_rect.size {
        Size::Physical(s) => PhysicalSize {
            width: s.width as f64,
            height: s.height as f64,
        },
        Size::Logical(s) => s.to_physical(scale),
    }
}

fn tray_rect_is_valid(tray_rect: &Rect, scale: f64) -> bool {
    let size = physical_tray_size(tray_rect, scale);
    size.width > 1.0 && size.height > 1.0
}

/// tray rect 无效时：优先光标附近，否则主屏右上角（菜单栏下方）。
fn position_panel_fallback(window: &WebviewWindow) {
    let Ok(panel_size) = window.outer_size() else {
        return;
    };

    if let Ok(cursor) = window.cursor_position() {
        let x = cursor.x - (panel_size.width as f64 / 2.0);
        let y = cursor.y + 8.0;
        let _ = window.set_position(Position::Physical(PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        )));
        return;
    }

    if let Ok(Some(monitor)) = window.primary_monitor() {
        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();
        let x = monitor_pos.x + monitor_size.width as i32 - panel_size.width as i32 - 16;
        let y = monitor_pos.y + 28;
        let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
    }
}

fn position_panel_near_tray(window: &WebviewWindow, tray_rect: Option<Rect>) {
    let Ok(scale) = window.scale_factor() else {
        position_panel_fallback(window);
        return;
    };
    let Ok(panel_size) = window.outer_size() else {
        return;
    };

    let Some(tray_rect) = tray_rect.filter(|r| tray_rect_is_valid(r, scale)) else {
        position_panel_fallback(window);
        return;
    };

    let tray_pos: PhysicalPosition<f64> = tray_rect.position.to_physical(scale);
    let tray_size = physical_tray_size(&tray_rect, scale);

    let x = tray_pos.x + (tray_size.width / 2.0) - (panel_size.width as f64 / 2.0);
    let y = tray_pos.y + tray_size.height + 4.0;

    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    )));
}

fn show_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    position_panel_near_tray(&window, tray_rect);
    mark_panel_shown();
    let _ = window.show();
    let _ = window.set_focus();
}

fn hide_panel_if_blurred(window: &WebviewWindow) {
    if !should_hide_on_blur() {
        return;
    }
    let _ = window.hide();
}

fn toggle_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    match window.is_visible() {
        Ok(true) => {
            let _ = window.hide();
        }
        _ => {
            show_panel(app, tray_rect);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let _ = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            let open = MenuItem::with_id(app, "open", "Open Panel", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Meterbar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            // 菜单栏专用黑字透明底；macOS template 会随菜单栏自动着色。
            let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
                .expect("failed to load tray template icon");

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Meterbar — 左键打开面板，右键打开菜单")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_panel(app, None),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle(), Some(rect));
                    }
                })
                .build(app)?;

            let app_handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                // 窗口 + WKWebView 均需透明，否则 NSVisualEffectView（BehindWindow）被不透明白底盖住
                if let Err(err) = window.set_background_color(Some(Color(0, 0, 0, 0))) {
                    eprintln!("set_background_color failed: {err}");
                }
                #[cfg(target_os = "macos")]
                {
                    if let Err(err) = window.set_effects(
                        EffectsBuilder::new()
                            .effect(Effect::HudWindow)
                            .state(EffectState::FollowsWindowActiveState)
                            .radius(20.0)
                            .build(),
                    ) {
                        eprintln!("set_effects failed: {err}");
                    }
                }
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            hide_panel_if_blurred(&window);
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::set_cursor_session_token,
            commands::clear_cursor_session_token,
            commands::set_deepseek_api_key,
            commands::clear_deepseek_api_key,
            commands::refresh_cursor,
            commands::refresh_codex,
            commands::refresh_deepseek,
            commands::refresh_system,
            commands::refresh_latency,
            commands::get_settings,
            commands::update_settings,
            commands::get_panel_state,
            commands::refresh_all,
            commands::diagnose_local_session,
            set_panel_drag_active,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
