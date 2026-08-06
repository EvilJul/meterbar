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
    LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Rect,
    Size, WebviewWindow, WindowEvent,
};

/// 菜单栏下方锚点（逻辑点）。
const MENU_BAR_OFFSET_LOGICAL: f64 = 28.0;

/// 面板最近一次 show 的时间戳（毫秒），用于忽略紧随其后的失焦。
static PANEL_SHOWN_AT_MS: AtomicU64 = AtomicU64::new(0);
/// 全屏 Space 下 show→focus 时序更慢，grace 略放宽避免刚弹出就被 hide。
const BLUR_GRACE_MS: u64 = 600;
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

/// 将 monitor 的物理原点/尺寸还原为 AppKit 逻辑点矩形（tao 用该屏 scale 做 from_logical）。
fn monitor_logical_bounds(monitor: &Monitor) -> Option<(f64, f64, f64, f64)> {
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    let origin = monitor.position();
    let size = monitor.size();
    Some((
        origin.x as f64 / scale,
        origin.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
fn point_in_rect(x: f64, y: f64, ox: f64, oy: f64, ow: f64, oh: f64) -> bool {
    x >= ox && x < ox + ow && y >= oy && y < oy + oh
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// 在目标屏内把面板水平夹紧，避免贴边溢出。
fn clamp_panel_x(x: f64, panel_w: f64, ox: f64, ow: f64) -> f64 {
    if panel_w >= ow {
        return ox;
    }
    clamp(x, ox, ox + ow - panel_w)
}

/// Cocoa 坐标系：visibleFrame 内，面板顶边贴可见区顶（菜单栏下），水平以 anchor_x 居中。
/// 返回 `setFrameTopLeftPoint` 所需的 (x, top_y)。
fn cocoa_panel_top_left(
    anchor_x: f64,
    panel_w: f64,
    vf_x: f64,
    vf_y: f64,
    vf_w: f64,
    vf_h: f64,
) -> (f64, f64) {
    let x = clamp_panel_x(anchor_x - panel_w / 2.0, panel_w, vf_x, vf_w);
    let top_y = vf_y + vf_h;
    (x, top_y)
}

/// 判断点是否落在 Cocoa 矩形内（半开区间，与 AppKit 命中一致）。
fn cocoa_point_in_rect(x: f64, y: f64, ox: f64, oy: f64, ow: f64, oh: f64) -> bool {
    x >= ox && x < ox + ow && y >= oy && y < oy + oh
}

/// 窗口中心是否仍在目标屏 frame 内（用于 show 后二次校验）。
fn cocoa_frame_center_on_screen(
    frame_x: f64,
    frame_y: f64,
    frame_w: f64,
    frame_h: f64,
    screen_x: f64,
    screen_y: f64,
    screen_w: f64,
    screen_h: f64,
) -> bool {
    let cx = frame_x + frame_w / 2.0;
    let cy = frame_y + frame_h / 2.0;
    cocoa_point_in_rect(cx, cy, screen_x, screen_y, screen_w, screen_h)
}

fn panel_logical_size(window: &WebviewWindow) -> Option<LogicalSize<f64>> {
    let scale = window.scale_factor().ok()?;
    if scale <= 0.0 {
        return None;
    }
    let size = window.outer_size().ok()?;
    Some(size.to_logical::<f64>(scale))
}

fn set_panel_logical_position(window: &WebviewWindow, x: f64, y: f64) {
    let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
}

/// 菜单栏图标逻辑高度典型约 22pt；用于在多屏不同 scale 下消歧。
const TRAY_ICON_LOGICAL_HEIGHT: f64 = 22.0;
/// 菜单栏带高度（逻辑点）；tray Y 因 Retina pixels_high 翻转常越界，定位时只用此带。
const MENU_BAR_BAND_LOGICAL: f64 = 80.0;

fn monitors_same(a: &Monitor, b: &Monitor) -> bool {
    a.position() == b.position() && a.size() == b.size()
}

/// 用某屏 scale 把 tray 物理坐标还原为逻辑点。
///
/// 只校验中心 **X** 落在该屏内：tray-icon/tao 在 Retina 主屏用 `CGDisplayPixelsHigh`
///（物理像素）做 Y 翻转，还原后的逻辑 Y 经常超出真实屏高，XY 同时校验会误拒主屏。
fn tray_logical_on_monitor(
    monitor: &Monitor,
    tray_pos: PhysicalPosition<f64>,
    tray_size: PhysicalSize<f64>,
) -> Option<(LogicalPosition<f64>, LogicalSize<f64>, f64)> {
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    let (ox, _oy, ow, _oh) = monitor_logical_bounds(monitor)?;
    let logical_pos = tray_pos.to_logical::<f64>(scale);
    let logical_size = tray_size.to_logical::<f64>(scale);
    // 误用其它屏 scale 时图标逻辑尺寸会明显偏离菜单栏高度。
    if logical_size.height < 8.0 || logical_size.height > 48.0 {
        return None;
    }
    let cx = logical_pos.x + logical_size.width / 2.0;
    if cx >= ox && cx < ox + ow {
        let score = (logical_size.height - TRAY_ICON_LOGICAL_HEIGHT).abs();
        Some((logical_pos, logical_size, score))
    } else {
        None
    }
}

/// 解析托盘所在屏：逐屏用该屏 scale 反解 tray 物理坐标，按图标逻辑尺寸择优。
fn resolve_tray_monitor(
    window: &WebviewWindow,
    tray_pos: PhysicalPosition<f64>,
    tray_size: PhysicalSize<f64>,
) -> Option<(Monitor, LogicalPosition<f64>, LogicalSize<f64>)> {
    let monitors = window.available_monitors().ok()?;
    let mut best: Option<(f64, Monitor, LogicalPosition<f64>, LogicalSize<f64>)> = None;
    for monitor in monitors {
        if let Some((pos, size, score)) = tray_logical_on_monitor(&monitor, tray_pos, tray_size) {
            let replace = best.as_ref().is_none_or(|(best_score, ..)| score < *best_score);
            if replace {
                best = Some((score, monitor, pos, size));
            }
        }
    }
    best.map(|(_, monitor, pos, size)| (monitor, pos, size))
}

/// tao 的 cursor_position 用主屏 scale 把 AppKit 点乘成「物理」；这里反解回逻辑点。
fn cursor_logical_position(window: &WebviewWindow) -> Option<LogicalPosition<f64>> {
    let cursor = window.cursor_position().ok()?;
    let primary_scale = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .filter(|s| *s > 0.0)
        .or_else(|| window.scale_factor().ok().filter(|s| *s > 0.0))?;
    Some(cursor.to_logical::<f64>(primary_scale))
}

/// 仅按逻辑 X 命中屏（Y 在 Retina/全屏下不可靠）。
fn monitor_containing_logical_x(window: &WebviewWindow, x: f64) -> Option<Monitor> {
    let monitors = window.available_monitors().ok()?;
    for monitor in monitors {
        if let Some((ox, _oy, ow, _oh)) = monitor_logical_bounds(&monitor) {
            if x >= ox && x < ox + ow {
                return Some(monitor);
            }
        }
    }
    None
}

fn panel_y_on_monitor(
    monitor: &Monitor,
    tray_logical: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
) -> f64 {
    let Some((_ox, oy, _ow, _oh)) = monitor_logical_bounds(monitor) else {
        return MENU_BAR_OFFSET_LOGICAL;
    };
    if let Some((pos, size)) = tray_logical {
        // 仅当 tray Y 落在菜单栏带内才采用；否则用屏顶 + 偏移（规避 Retina Y 翻转）。
        if pos.y >= oy && pos.y <= oy + MENU_BAR_BAND_LOGICAL {
            return pos.y + size.height + 4.0;
        }
    }
    oy + MENU_BAR_OFFSET_LOGICAL
}

/// 无 tray 时：鼠标所在屏 → 主屏 → 窗口当前屏；锚在菜单栏下方。
/// 注意：current_monitor 常是面板上次所在的副屏，全屏点击时不能优先于鼠标/主屏。
fn position_panel_fallback(window: &WebviewWindow) {
    let Some(panel) = panel_logical_size(window) else {
        return;
    };

    let cursor = cursor_logical_position(window);
    let monitor = cursor
        .as_ref()
        .and_then(|c| monitor_containing_logical_x(window, c.x))
        .or_else(|| window.primary_monitor().ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        eprintln!("[meterbar] fallback: no monitor");
        return;
    };
    let Some((ox, _oy, ow, _oh)) = monitor_logical_bounds(&monitor) else {
        return;
    };

    let x = if let Some(c) = cursor {
        clamp_panel_x(c.x - panel.width / 2.0, panel.width, ox, ow)
    } else {
        clamp_panel_x(ox + ow - panel.width - 16.0, panel.width, ox, ow)
    };
    let y = panel_y_on_monitor(&monitor, None);
    eprintln!(
        "[meterbar] fallback mon=({ox:.0},{ow:.0}) scale={:.2} panel=({x:.1},{y:.1}) cursor={:?}",
        monitor.scale_factor(),
        cursor.map(|c| (c.x, c.y))
    );
    set_panel_logical_position(window, x, y);
}

/// 托盘 id：macOS 上用于取出 NSStatusItem → button.window.screen。
const METERBAR_TRAY_ID: &str = "meterbar";

/// 每次 show/toggle 重算面板位置。
///
/// macOS：优先 status item 所在 NSScreen（点哪块菜单栏就锚哪块），
/// 全屏时其次 `NSScreen.main`（菜单栏焦点屏），最后才用 mouse。
/// 直接 `setFrameTopLeftPoint`；tao tray/cursor 在全屏 Space 下不可靠。
/// 非 macOS 或 AppKit 失败时退回 tao 逻辑。
fn position_panel_near_tray(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    tray_rect: Option<Rect>,
) {
    #[cfg(target_os = "macos")]
    {
        if position_panel_via_appkit(app, window) {
            return;
        }
        eprintln!("[meterbar] appkit position unavailable, falling back to tao");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    position_panel_near_tray_tao(window, tray_rect);
}

/// tao 路径（非 macOS / AppKit 失败时）：优先鼠标屏，再用 tray 锚 X。
fn position_panel_near_tray_tao(window: &WebviewWindow, tray_rect: Option<Rect>) {
    let win_scale = window.scale_factor().unwrap_or(1.0);
    let Some(panel) = panel_logical_size(window) else {
        return;
    };

    let cursor = cursor_logical_position(window);
    let mouse_monitor = cursor
        .as_ref()
        .and_then(|c| monitor_containing_logical_x(window, c.x));

    let Some(tray_rect) = tray_rect.filter(|r| tray_rect_is_valid(r, win_scale)) else {
        position_panel_fallback(window);
        return;
    };

    // macOS tray-icon：Rect 已是 Physical（按 status item window.backingScaleFactor 编码）。
    let tray_pos: PhysicalPosition<f64> = tray_rect.position.to_physical(win_scale);
    let tray_size = physical_tray_size(&tray_rect, win_scale);
    let tray_resolved = resolve_tray_monitor(window, tray_pos, tray_size);

    let (monitor, tray_anchor, via) = match (mouse_monitor, tray_resolved) {
        (Some(mouse_mon), Some((tray_mon, _, _))) if !monitors_same(&mouse_mon, &tray_mon) => {
            let on_mouse = tray_logical_on_monitor(&mouse_mon, tray_pos, tray_size)
                .map(|(p, s, _)| (p, s));
            (mouse_mon, on_mouse, "mouse_over_tray_mismatch")
        }
        (_, Some((tray_mon, pos, size))) => (tray_mon, Some((pos, size)), "tray"),
        (Some(mouse_mon), None) => {
            let on_mouse = tray_logical_on_monitor(&mouse_mon, tray_pos, tray_size)
                .map(|(p, s, _)| (p, s));
            (mouse_mon, on_mouse, "mouse_only")
        }
        (None, None) => {
            position_panel_fallback(window);
            return;
        }
    };

    let Some((ox, _oy, ow, _oh)) = monitor_logical_bounds(&monitor) else {
        position_panel_fallback(window);
        return;
    };

    let anchor_x = if let Some((pos, size)) = tray_anchor.as_ref() {
        pos.x + size.width / 2.0
    } else if let Some(c) = cursor {
        c.x
    } else {
        ox + ow - 16.0
    };
    let x = clamp_panel_x(anchor_x - panel.width / 2.0, panel.width, ox, ow);
    let y = panel_y_on_monitor(&monitor, tray_anchor);

    eprintln!(
        "[meterbar] pos via={via} win_scale={win_scale:.2} tray_phys=({:.1},{:.1}) size_phys=({:.1},{:.1}) mon=({ox:.0},{ow:.0}) mon_scale={:.2} panel=({x:.1},{y:.1}) cursor={:?}",
        tray_pos.x,
        tray_pos.y,
        tray_size.width,
        tray_size.height,
        monitor.scale_factor(),
        cursor.map(|c| (c.x, c.y))
    );
    set_panel_logical_position(window, x, y);
}

/// 两块 NSScreen 是否同一物理屏（比 pointer 相等更稳）。
#[cfg(target_os = "macos")]
fn nsscreen_same(
    a: &objc2_app_kit::NSScreen,
    b: &objc2_app_kit::NSScreen,
) -> bool {
    let fa = a.frame();
    let fb = b.frame();
    (fa.origin.x - fb.origin.x).abs() < 0.5
        && (fa.origin.y - fb.origin.y).abs() < 0.5
        && (fa.size.width - fb.size.width).abs() < 0.5
        && (fa.size.height - fb.size.height).abs() < 0.5
}

/// status item window 几何（Send）：用于在 NSScreen::screens 里回查所在屏。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct StatusItemGeom {
    anchor_x: f64,
    /// status item window.screen.frame.origin（Cocoa）
    screen_x: f64,
    screen_y: f64,
    screen_w: f64,
    screen_h: f64,
}

/// 从 tray-icon 的 NSStatusItem 取 button.window.screen 几何 + 锚点 X。
/// `with_inner_tray_icon` 要求返回值 Send，故不直接带回 Retained<NSScreen>。
#[cfg(target_os = "macos")]
fn appkit_status_item_geom(app: &tauri::AppHandle) -> Option<StatusItemGeom> {
    use objc2::MainThreadMarker;

    let tray = app.tray_by_id(METERBAR_TRAY_ID)?;
    tray.with_inner_tray_icon(|inner| {
        let mtm = MainThreadMarker::new()?;
        let item = inner.ns_status_item()?;
        let button = item.button(mtm)?;
        let window = button.window()?;
        let screen = window.screen()?;
        let sf = screen.frame();
        let wf = window.frame();
        Some(StatusItemGeom {
            anchor_x: wf.origin.x + wf.size.width / 2.0,
            screen_x: sf.origin.x,
            screen_y: sf.origin.y,
            screen_w: sf.size.width,
            screen_h: sf.size.height,
        })
    })
    .ok()
    .flatten()
}

/// 按 frame origin/size 在 screens 里找回 Retained<NSScreen>。
#[cfg(target_os = "macos")]
fn appkit_screen_matching_frame(
    mtm: objc2::MainThreadMarker,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSScreen>> {
    use objc2_app_kit::NSScreen;
    let screens = NSScreen::screens(mtm);
    let count = screens.count();
    for i in 0..count {
        let screen = screens.objectAtIndex(i);
        let f = screen.frame();
        if (f.origin.x - x).abs() < 0.5
            && (f.origin.y - y).abs() < 0.5
            && (f.size.width - w).abs() < 0.5
            && (f.size.height - h).abs() < 0.5
        {
            return Some(screen);
        }
    }
    None
}

/// 选锚屏：status item →（mouse≠main 时）main → mouse → main fallback。
/// 返回 (screen, anchor_x, via)。
#[cfg(target_os = "macos")]
fn appkit_resolve_anchor_screen(
    app: &tauri::AppHandle,
    mtm: objc2::MainThreadMarker,
    log_screens: bool,
) -> Option<(objc2::rc::Retained<objc2_app_kit::NSScreen>, f64, &'static str)> {
    use objc2_app_kit::{NSEvent, NSScreen};
    use objc2_foundation::NSPointInRect;

    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let main = NSScreen::mainScreen(mtm);

    if log_screens {
        for (idx, screen) in screens.iter().enumerate() {
            let f = screen.frame();
            let vf = screen.visibleFrame();
            eprintln!(
                "[meterbar] nsscreen[{idx}] name={} frame=({:.0},{:.0},{:.0}x{:.0}) visible=({:.0},{:.0},{:.0}x{:.0}) scale={:.2}",
                screen.localizedName(),
                f.origin.x,
                f.origin.y,
                f.size.width,
                f.size.height,
                vf.origin.x,
                vf.origin.y,
                vf.size.width,
                vf.size.height,
                screen.backingScaleFactor()
            );
        }
    }

    let mut mouse_screen = None;
    let count = screens.count();
    for i in 0..count {
        let screen = screens.objectAtIndex(i);
        if NSPointInRect(mouse, screen.frame()) {
            mouse_screen = Some(screen);
            break;
        }
    }

    let status_geom = appkit_status_item_geom(app);
    let status_screen = status_geom.and_then(|g| {
        appkit_screen_matching_frame(mtm, g.screen_x, g.screen_y, g.screen_w, g.screen_h)
            .map(|s| (s, g.anchor_x))
    });
    let mouse_name = mouse_screen
        .as_ref()
        .map(|s| s.localizedName().to_string())
        .unwrap_or_else(|| "?".into());
    let main_name = main
        .as_ref()
        .map(|s| s.localizedName().to_string())
        .unwrap_or_else(|| "?".into());
    let status_name = status_screen
        .as_ref()
        .map(|(s, _)| s.localizedName().to_string())
        .unwrap_or_else(|| "-".into());
    let menubar_focus_mismatch = match (&main, &mouse_screen) {
        (Some(m), Some(ms)) => !nsscreen_same(m, ms),
        _ => false,
    };
    // 粗判：mouse 与 main 不一致时，多半处于全屏菜单栏焦点与内容屏分离。
    eprintln!(
        "[meterbar] screen_pick mouse=({:.1},{:.1}) mouse_screen={mouse_name} main={main_name} status_item={status_name} menubar_focus_mismatch={menubar_focus_mismatch}",
        mouse.x, mouse.y
    );

    if let Some((screen, anchor_x)) = status_screen {
        return Some((screen, anchor_x, "status_item_screen"));
    }

    // 全屏常见：tray 点在有菜单栏焦点的屏（main），mouseLocation 仍落在另一块 content 屏。
    if menubar_focus_mismatch {
        if let Some(m) = main {
            return Some((m, mouse.x, "main_screen_menubar"));
        }
    }

    if let Some(screen) = mouse_screen {
        return Some((screen, mouse.x, "nsevent_mouse"));
    }

    main.map(|s| (s, mouse.x, "main_screen_fallback"))
}

/// AppKit 真源：status item / 菜单栏屏 → visibleFrame 顶边锚面板。
/// 返回 true 表示已成功设置 frame。
#[cfg(target_os = "macos")]
fn position_panel_via_appkit(app: &tauri::AppHandle, window: &WebviewWindow) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::NSPoint;

    let Ok(ns_window_ptr) = window.ns_window() else {
        return false;
    };
    if ns_window_ptr.is_null() {
        return false;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    let mouse = objc2_app_kit::NSEvent::mouseLocation();
    let Some((screen, anchor_x, via)) = appkit_resolve_anchor_screen(app, mtm, true) else {
        return false;
    };

    let vf = screen.visibleFrame();
    let screen_frame = screen.frame();
    let screen_name = screen.localizedName();

    // SAFETY: ns_window_ptr 来自 Tauri，生命周期随 window。
    let ns_window = unsafe { &*(ns_window_ptr as *const NSWindow) };
    let panel_size = ns_window.frame().size;
    let panel_w = panel_size.width;
    let panel_h = panel_size.height;
    if panel_w < 1.0 || panel_h < 1.0 {
        return false;
    }

    let (x, top_y) = cocoa_panel_top_left(
        anchor_x,
        panel_w,
        vf.origin.x,
        vf.origin.y,
        vf.size.width,
        vf.size.height,
    );
    // Cocoa：top_y 是顶边 Y；frame.origin.y 是底边。用 setFrame 强制整窗落到锚点屏。
    let origin_y = top_y - panel_h;
    use objc2_foundation::NSRect;
    let dest = NSRect::new(NSPoint::new(x, origin_y), panel_size);
    ns_window.setFrame_display(dest, false);

    let actual = ns_window.frame();
    eprintln!(
        "[meterbar] pos via={via} mouse=({:.1},{:.1}) anchor_x={:.1} screen={screen_name} frame=({:.0},{:.0},{:.0}x{:.0}) visible=({:.0},{:.0},{:.0}x{:.0}) top_left=({:.1},{:.1}) panel_size=({:.0}x{:.0}) actual=({:.1},{:.1},{:.0}x{:.0})",
        mouse.x,
        mouse.y,
        anchor_x,
        screen_frame.origin.x,
        screen_frame.origin.y,
        screen_frame.size.width,
        screen_frame.size.height,
        vf.origin.x,
        vf.origin.y,
        vf.size.width,
        vf.size.height,
        x,
        top_y,
        panel_w,
        panel_h,
        actual.origin.x,
        actual.origin.y,
        actual.size.width,
        actual.size.height
    );
    true
}

/// show / orderFront / set_focus 之后：强制按 status item 屏再钉一次 frame。
///
/// 注意：`NSWindow.screen` / frame 中心只能证明坐标落在哪块物理屏，**不能**证明
/// Space 是否正确。旧逻辑仅在「中心不在目标屏」时纠正，会在
/// `MoveToActiveSpace` 把窗口拽到副屏全屏 Space、但 frame 仍短暂留在 Built-in
/// 时误报 `post_show ok`。因此这里始终 re-anchor。
#[cfg(target_os = "macos")]
fn correct_panel_frame_after_show(app: &tauri::AppHandle, window: &WebviewWindow) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;

    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    let Some((target, _, via)) = appkit_resolve_anchor_screen(app, mtm, false) else {
        return;
    };

    let ns_window = unsafe { &*(ns_window_ptr as *const NSWindow) };
    let actual_before = ns_window.frame();
    let tf = target.frame();
    let on_target = cocoa_frame_center_on_screen(
        actual_before.origin.x,
        actual_before.origin.y,
        actual_before.size.width,
        actual_before.size.height,
        tf.origin.x,
        tf.origin.y,
        tf.size.width,
        tf.size.height,
    );

    let win_screen_name = ns_window
        .screen()
        .map(|s| s.localizedName().to_string())
        .unwrap_or_else(|| "?".into());
    let target_name = target.localizedName();

    eprintln!(
        "[meterbar] post_show force via={via} on_target={on_target} win_screen={win_screen_name} target={target_name} before=({:.1},{:.1},{:.0}x{:.0})",
        actual_before.origin.x,
        actual_before.origin.y,
        actual_before.size.width,
        actual_before.size.height
    );
    let _ = position_panel_via_appkit(app, window);
    let actual_after = ns_window.frame();
    let after_screen = ns_window
        .screen()
        .map(|s| s.localizedName().to_string())
        .unwrap_or_else(|| "?".into());
    eprintln!(
        "[meterbar] post_show after via={via} win_screen={after_screen} actual=({:.1},{:.1},{:.0}x{:.0})",
        actual_after.origin.x,
        actual_after.origin.y,
        actual_after.size.width,
        actual_after.size.height
    );
}

/// 让面板尽量浮在其他 App 的全屏 Space 上，且**留在我们钉好的物理屏**。
///
/// Tauri 的主窗口是 `NSWindow`（非 `NSPanel`）。不能用 `object_setClass` 强转：
/// 当前 SDK 上实例大小 NSWindow=464 / NSPanel=456，会触发 objc2 debug 断言并 abort。
/// 可行路径：合法 collectionBehavior + 抬高 level + `orderFrontRegardless`。
/// tiling 互斥位必须恰好其一，否则 `_validateCollectionBehavior:` 会断言。
///
/// **禁止 `MoveToActiveSpace`**：多屏 +「副屏全屏 / 主屏点 tray」时，它会在
/// orderFront 时把窗口拽到副屏的全屏 Space，而 frame 日志仍可能短暂显示 Built-in，
/// 造成「算法自以为对、肉眼在副屏」的错屏。改用 `CanJoinAllSpaces`，由我们
/// 按 status item 屏 `setFrame`，窗口留在锚点屏。
#[cfg(target_os = "macos")]
fn configure_panel_for_fullscreen_spaces(window: &WebviewWindow) {
    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    use objc2_app_kit::{NSPopUpMenuWindowLevel, NSWindow, NSWindowCollectionBehavior};

    unsafe {
        let ns_window = &*(ns_window_ptr as *const NSWindow);

        let mut behavior = ns_window.collectionBehavior();
        // 与 MoveToActiveSpace 互斥：不要跟随「当前全屏 Space」跨屏迁移。
        behavior.remove(NSWindowCollectionBehavior::MoveToActiveSpace);
        behavior.insert(NSWindowCollectionBehavior::CanJoinAllSpaces);
        // Managed / Transient / Stationary 三选一
        behavior.remove(NSWindowCollectionBehavior::Managed);
        behavior.remove(NSWindowCollectionBehavior::Stationary);
        behavior.insert(NSWindowCollectionBehavior::Transient);
        behavior.remove(NSWindowCollectionBehavior::ParticipatesInCycle);
        behavior.insert(NSWindowCollectionBehavior::IgnoresCycle);
        // FullScreen* 三选一
        behavior.remove(NSWindowCollectionBehavior::FullScreenPrimary);
        behavior.remove(NSWindowCollectionBehavior::FullScreenNone);
        behavior.insert(NSWindowCollectionBehavior::FullScreenAuxiliary);
        // FullScreenAllowsTiling / DisallowsTiling 必须恰好其一，否则断言。
        behavior.remove(NSWindowCollectionBehavior::FullScreenAllowsTiling);
        behavior.insert(NSWindowCollectionBehavior::FullScreenDisallowsTiling);
        ns_window.setCollectionBehavior(behavior);

        // alwaysOnTop 只有 Floating(3)，全屏内容仍可能压住；用弹出菜单级。
        ns_window.setLevel(NSPopUpMenuWindowLevel);
        ns_window.setHidesOnDeactivate(false);
        eprintln!(
            "[meterbar] collectionBehavior=CanJoinAllSpaces|Transient|FullScreenAuxiliary|DisallowsTiling (no MoveToActiveSpace) level={}",
            NSPopUpMenuWindowLevel
        );
    }
}

#[cfg(target_os = "macos")]
fn order_panel_front(window: &WebviewWindow) {
    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }
    use objc2_app_kit::NSWindow;
    unsafe {
        let ns_window = &*(ns_window_ptr as *const NSWindow);
        ns_window.orderFrontRegardless();
    }
}

fn show_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("[meterbar] show_panel: main window missing");
        return;
    };
    #[cfg(target_os = "macos")]
    configure_panel_for_fullscreen_spaces(&window);
    // show 前钉一次；show / orderFront / set_focus 后再强制钉到 status item 屏。
    position_panel_near_tray(app, &window, tray_rect);
    mark_panel_shown();
    let show_res = window.show();
    position_panel_near_tray(app, &window, tray_rect);
    #[cfg(target_os = "macos")]
    {
        order_panel_front(&window);
        correct_panel_frame_after_show(app, &window);
    }
    // 需要 key 窗口才能靠 Focused(false) 点外关闭；Accessory 下通常不抢 Dock。
    let focus_res = window.set_focus();
    #[cfg(target_os = "macos")]
    correct_panel_frame_after_show(app, &window);
    eprintln!(
        "[meterbar] show_panel show={show_res:?} focus={focus_res:?} visible={:?} focused={:?}",
        window.is_visible(),
        window.is_focused()
    );
}

fn hide_panel_if_blurred(window: &WebviewWindow) {
    if !should_hide_on_blur() {
        return;
    }
    eprintln!("[meterbar] hide_panel on blur");
    let _ = window.hide();
}

fn toggle_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("[meterbar] toggle_panel: main window missing");
        return;
    };

    // 全屏 Space 上 is_visible 可能仍为 true（窗口在别的 Space），不能只靠 visible 决定 hide。
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    eprintln!("[meterbar] tray toggle visible={visible} focused={focused}");
    if visible && focused {
        let _ = window.hide();
    } else {
        show_panel(app, tray_rect);
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

            let _tray = TrayIconBuilder::with_id(METERBAR_TRAY_ID)
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
                    configure_panel_for_fullscreen_spaces(&window);
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

#[cfg(test)]
mod panel_position_tests {
    use super::{
        clamp_panel_x, cocoa_frame_center_on_screen, cocoa_panel_top_left, cocoa_point_in_rect,
        point_in_rect,
    };

    #[test]
    fn point_in_rect_accepts_interior() {
        assert!(point_in_rect(100.0, 10.0, 0.0, 0.0, 1512.0, 982.0));
        assert!(!point_in_rect(2000.0, 10.0, 0.0, 0.0, 1512.0, 982.0));
        assert!(point_in_rect(1600.0, 10.0, 1512.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn clamp_panel_x_keeps_panel_inside_monitor() {
        let panel_w = 360.0;
        let ox = 1512.0;
        let ow = 1920.0;
        assert_eq!(clamp_panel_x(1400.0, panel_w, ox, ow), ox);
        assert_eq!(
            clamp_panel_x(3200.0, panel_w, ox, ow),
            ox + ow - panel_w
        );
        assert_eq!(clamp_panel_x(2000.0, panel_w, ox, ow), 2000.0);
    }

    #[test]
    fn cocoa_panel_top_left_anchors_under_menu_bar() {
        // 主屏 visibleFrame 常见：菜单栏下方 y=0 起算时 top = vf_y+vf_h
        let (x, top_y) = cocoa_panel_top_left(1200.0, 360.0, 0.0, 0.0, 1728.0, 1055.0);
        assert_eq!(x, 1200.0 - 180.0);
        assert_eq!(top_y, 1055.0);
        let (x2, _) = cocoa_panel_top_left(50.0, 360.0, 0.0, 0.0, 1728.0, 1055.0);
        assert_eq!(x2, 0.0);
    }

    #[test]
    fn cocoa_frame_center_detects_wrong_screen() {
        let primary = (0.0, 0.0, 1728.0, 1117.0);
        let external = (1728.0, 0.0, 1920.0, 1080.0);
        // 面板落在拓展屏
        assert!(!cocoa_frame_center_on_screen(
            2000.0, 500.0, 360.0, 480.0, primary.0, primary.1, primary.2, primary.3
        ));
        assert!(cocoa_frame_center_on_screen(
            2000.0, 500.0, 360.0, 480.0, external.0, external.1, external.2, external.3
        ));
        assert!(cocoa_point_in_rect(100.0, 50.0, 0.0, 0.0, 1728.0, 1117.0));
    }

    #[test]
    fn retina_physical_to_logical_stays_on_primary_not_external() {
        // tray 在主屏逻辑 x=1400、scale=2 → 物理 2800；若误用 scale=1 当逻辑点会落到拓展屏。
        let primary_scale = 2.0;
        let tray_physical_x = 1400.0 * primary_scale;
        let logical_x = tray_physical_x / primary_scale;
        assert!(point_in_rect(
            logical_x,
            10.0,
            0.0,
            0.0,
            1512.0,
            982.0
        ));
        assert!(!point_in_rect(
            tray_physical_x,
            10.0,
            0.0,
            0.0,
            1512.0,
            982.0
        ));
        assert!(point_in_rect(
            tray_physical_x,
            10.0,
            1512.0,
            0.0,
            1920.0,
            1080.0
        ));
    }

    #[test]
    fn retina_flipped_y_out_of_bounds_still_selects_by_x() {
        // tray-icon 用 pixels_high 翻转 Y：逻辑 y≈1004 超出 982 屏高；选屏只能看 X。
        let primary = (0.0, 0.0, 1512.0, 982.0);
        let external = (1512.0, 0.0, 1920.0, 1080.0);
        let tray_logical_x = 1400.0;
        let tray_logical_y = 1004.0;
        assert!(
            tray_logical_x >= primary.0 && tray_logical_x < primary.0 + primary.2,
            "X 应落在主屏"
        );
        assert!(
            !point_in_rect(
                tray_logical_x,
                tray_logical_y,
                primary.0,
                primary.1,
                primary.2,
                primary.3
            ),
            "含 Y 的命中会误拒主屏"
        );
        let tray_as_physical_on_external = tray_logical_x * 2.0; // 2800
        assert!(
            tray_as_physical_on_external >= external.0
                && tray_as_physical_on_external < external.0 + external.2,
            "若把 Retina 物理 X 当逻辑点会落到拓展屏"
        );
    }

    #[test]
    fn status_item_screen_beats_mouse_when_menubar_focus_mismatches() {
        // 全屏：status item / main（菜单栏焦点）在拓展屏，mouseLocation 仍落在主屏
        // → 必须以菜单栏屏为准（点哪块 tray 锚哪块）。
        let primary: (f64, f64, f64, f64) = (0.0, 0.0, 1728.0, 1117.0);
        let external: (f64, f64, f64, f64) = (-896.0, -516.0, 896.0, 1344.0);
        let mouse: (f64, f64) = (1187.5, 1099.1);
        let main = external; // NSScreen.main = 有菜单栏焦点的全屏屏
        let status_item = external;
        let mouse_on_primary = cocoa_point_in_rect(
            mouse.0, mouse.1, primary.0, primary.1, primary.2, primary.3,
        );
        let main_is_external = (main.0 - external.0).abs() < 0.5;
        let status_is_external = (status_item.0 - external.0).abs() < 0.5;
        assert!(mouse_on_primary && main_is_external && status_is_external);
        let menubar_focus_mismatch = mouse_on_primary && main_is_external;
        let chosen = if status_is_external {
            external
        } else if menubar_focus_mismatch {
            main
        } else {
            primary
        };
        assert_eq!(chosen.0, external.0);
        assert!(cocoa_frame_center_on_screen(
            chosen.0 + 100.0,
            chosen.1 + 100.0,
            360.0,
            320.0,
            external.0,
            external.1,
            external.2,
            external.3,
        ));
    }
}
