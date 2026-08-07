mod commands;
mod credentials;
mod models;
mod network;
mod platform_paths;
mod providers;
mod settings;
mod system;

use commands::AppState;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::Color,
    LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Rect,
    Size, WebviewWindow, WindowEvent,
};
#[cfg(target_os = "macos")]
use tauri::window::{Effect, EffectState, EffectsBuilder};

/// 菜单栏下方锚点（逻辑点）。
const MENU_BAR_OFFSET_LOGICAL: f64 = 28.0;
/// GNOME 等顶栏托盘图标默认在右侧；无 tray 几何时的右边距。
const LINUX_TRAY_RIGHT_MARGIN_LOGICAL: f64 = 12.0;
/// 窗口尚未 layout 时 outer_size 可能为 0，定位用默认逻辑宽（与 tauri.conf / 前端一致）。
const PANEL_DEFAULT_LOGICAL_WIDTH: f64 = 360.0;
const PANEL_DEFAULT_LOGICAL_HEIGHT: f64 = 320.0;

/// 面板最近一次 show 的时间戳（毫秒），用于忽略紧随其后的失焦。
static PANEL_SHOWN_AT_MS: AtomicU64 = AtomicU64::new(0);
/// 全屏 /「点壁纸显示桌面」后 show→focus 抖动更久，grace 放宽避免刚弹出就被 hide。
#[cfg(target_os = "macos")]
const BLUR_GRACE_MS: u64 = 1200;
/// Linux/Wayland：show 后 Focused(false) 更晚更密，grace 加长。
#[cfg(not(target_os = "macos"))]
const BLUR_GRACE_MS: u64 = 2800;
/// 主面板正在拖拽排序时抑制失焦 hide，避免拖到一半窗口被关掉。
static PANEL_DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Linux AppIndicator 常无法在 show 时给出 tray.rect；缓存 Click/Enter/Move 的几何。
#[derive(Clone, Copy, Debug)]
struct CachedTrayGeom {
    pos: PhysicalPosition<f64>,
    size: PhysicalSize<f64>,
}

static CACHED_TRAY_GEOM: Mutex<Option<CachedTrayGeom>> = Mutex::new(None);
/// 最近一次成功计算出的面板逻辑坐标（Linux reanchor 复用，避免又跟鼠标跑）。
static LAST_PANEL_LOGICAL_POS: Mutex<Option<(f64, f64)>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mark_panel_shown() {
    PANEL_SHOWN_AT_MS.store(now_ms(), Ordering::SeqCst);
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
    let scale = window.scale_factor().ok().filter(|s| *s > 0.0).unwrap_or(1.0);
    let size = window.outer_size().ok();
    let logical = size.map(|s| s.to_logical::<f64>(scale));
    // show 前 outer 常为 0×0；用配置默认宽高，否则 clamp 会把 x 当成「光标 x」。
    let w = logical
        .as_ref()
        .map(|s| s.width)
        .filter(|w| *w >= 32.0)
        .unwrap_or(PANEL_DEFAULT_LOGICAL_WIDTH);
    let h = logical
        .as_ref()
        .map(|s| s.height)
        .filter(|h| *h >= 32.0)
        .unwrap_or(PANEL_DEFAULT_LOGICAL_HEIGHT);
    Some(LogicalSize::new(w, h))
}

/// 将面板移到逻辑坐标 (x,y)。Linux/Wayland 上 Logical 常被忽略，优先写 Physical。
fn set_panel_logical_position(window: &WebviewWindow, x: f64, y: f64) {
    if let Ok(mut g) = LAST_PANEL_LOGICAL_POS.lock() {
        *g = Some((x, y));
    }

    let scale = window
        .scale_factor()
        .ok()
        .filter(|s| *s > 0.0)
        .or_else(|| {
            window
                .primary_monitor()
                .ok()
                .flatten()
                .map(|m| m.scale_factor())
                .filter(|s| *s > 0.0)
        })
        .unwrap_or(1.0);

    let px = (x * scale).round() as i32;
    let py = (y * scale).round() as i32;

    // 1) Physical（X11 / 部分 Wayland 合成器更认）
    if let Err(e) = window.set_position(Position::Physical(PhysicalPosition::new(px, py))) {
        eprintln!("[meterbar] set_position physical err={e}");
    }
    let outer_phys = window.outer_position();
    eprintln!(
        "[meterbar] set_pos target_logical=({x:.1},{y:.1}) scale={scale:.2} physical=({px},{py}) outer={outer_phys:?}"
    );

    // 2) 仍在原点则再试 Logical
    let still_origin = matches!(outer_phys, Ok(p) if p.x.abs() < 2 && p.y.abs() < 2)
        && (x > 8.0 || y > 8.0);
    if still_origin {
        if let Err(e) = window.set_position(Position::Logical(LogicalPosition::new(x, y))) {
            eprintln!("[meterbar] set_position logical err={e}");
        }
        eprintln!(
            "[meterbar] set_pos logical retry outer={:?}",
            window.outer_position()
        );
    }
}

fn last_panel_logical_pos() -> Option<(f64, f64)> {
    LAST_PANEL_LOGICAL_POS.lock().ok().and_then(|g| *g)
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
    let logical = cursor.to_logical::<f64>(primary_scale);
    // Wayland 上未移动指针时常回报 (0,0)，不能当真。
    if !cursor_logical_is_usable(logical) {
        return None;
    }
    Some(logical)
}

fn cursor_logical_is_usable(c: LogicalPosition<f64>) -> bool {
    // 原点附近视为无效；真实点击托盘时 x/y 至少有一侧明显离开 0。
    !(c.x.abs() < 1.5 && c.y.abs() < 1.5)
}

fn remember_tray_rect(rect: &Rect) {
    let (x, y) = match rect.position {
        Position::Physical(p) => (p.x as f64, p.y as f64),
        Position::Logical(p) => {
            // 缓存时先按 1.0 存逻辑，取出时再当 physical 用会偏；尽量只缓存 physical。
            // 若只有 logical，按 scale=1 记，position_panel 路径会再 to_physical。
            (p.x, p.y)
        }
    };
    let (w, h) = match rect.size {
        Size::Physical(s) => (s.width as f64, s.height as f64),
        Size::Logical(s) => (s.width, s.height),
    };
    if w <= 0.5 || h <= 0.5 {
        return;
    }
    if let Ok(mut g) = CACHED_TRAY_GEOM.lock() {
        *g = Some(CachedTrayGeom {
            pos: PhysicalPosition { x, y },
            size: PhysicalSize {
                width: w,
                height: h,
            },
        });
        eprintln!("[meterbar] cached tray geom phys=({x:.1},{y:.1})x({w:.1}x{h:.1})");
    }
}

fn cached_tray_rect() -> Option<Rect> {
    let g = CACHED_TRAY_GEOM.lock().ok()?;
    let c = (*g)?;
    Some(Rect {
        position: Position::Physical(PhysicalPosition {
            x: c.pos.x as i32,
            y: c.pos.y as i32,
        }),
        size: Size::Physical(PhysicalSize {
            width: c.size.width as u32,
            height: c.size.height as u32,
        }),
    })
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
    let Some((_ox, oy, _ow, oh)) = monitor_logical_bounds(monitor) else {
        return MENU_BAR_OFFSET_LOGICAL;
    };
    if let Some((pos, size)) = tray_logical {
        let bottom = pos.y + size.height + 4.0;
        // 顶栏托盘（macOS 菜单栏 / GNOME 顶栏）：图标在屏顶附近。
        if pos.y >= oy && pos.y <= oy + MENU_BAR_BAND_LOGICAL {
            return bottom.max(oy + MENU_BAR_OFFSET_LOGICAL);
        }
        // Linux 底栏 / 侧栏：图标在屏下方或其它位置时，仍尽量贴在图标外侧。
        #[cfg(not(target_os = "macos"))]
        {
            if size.height > 0.0 && size.height < 80.0 {
                // 靠近底边：面板开在图标上方会更合理，但当前 UI 假定向下展开；
                // 先贴图标下方并夹到可见区内。
                let max_y = oy + oh - 40.0;
                return bottom.clamp(oy + 4.0, max_y);
            }
        }
        let _ = oh;
    }
    oy + MENU_BAR_OFFSET_LOGICAL
}

/// 无 tray 时：缓存 tray →（macOS）鼠标 → 主屏顶栏右侧（GNOME 指示器区）。
/// Linux 上 **不要** 用屏幕中部的 cursor 当锚点（点托盘时指针常不在顶栏）。
fn position_panel_fallback(window: &WebviewWindow) {
    let Some(panel) = panel_logical_size(window) else {
        return;
    };

    // 若有缓存托盘几何，走 tao 主路径（比「瞎猜右上」准）。
    if let Some(rect) = cached_tray_rect() {
        eprintln!("[meterbar] fallback using cached tray rect");
        position_panel_near_tray_tao(window, Some(rect));
        return;
    }

    let cursor = cursor_logical_position(window);
    let monitor = {
        #[cfg(target_os = "macos")]
        {
            cursor
                .as_ref()
                .and_then(|c| monitor_containing_logical_x(window, c.x))
                .or_else(|| window.primary_monitor().ok().flatten())
                .or_else(|| window.current_monitor().ok().flatten())
        }
        #[cfg(not(target_os = "macos"))]
        {
            window
                .primary_monitor()
                .ok()
                .flatten()
                .or_else(|| window.current_monitor().ok().flatten())
        }
    };

    let Some(monitor) = monitor else {
        eprintln!("[meterbar] fallback: no monitor");
        return;
    };
    let Some((ox, oy, ow, _oh)) = monitor_logical_bounds(&monitor) else {
        return;
    };

    #[cfg(target_os = "macos")]
    let (x, y, via) = if let Some(c) = cursor {
        (
            clamp_panel_x(c.x - panel.width / 2.0, panel.width, ox, ow),
            panel_y_on_monitor(&monitor, None),
            "cursor",
        )
    } else {
        let x = clamp_panel_x(
            ox + ow - panel.width - LINUX_TRAY_RIGHT_MARGIN_LOGICAL,
            panel.width,
            ox,
            ow,
        );
        (x, oy + MENU_BAR_OFFSET_LOGICAL, "top_right")
    };

    // Linux/GNOME：指示器在顶栏右侧；AppIndicator 无 rect 时固定锚右上。
    #[cfg(not(target_os = "macos"))]
    let (x, y, via) = {
        let x = clamp_panel_x(
            ox + ow - panel.width - LINUX_TRAY_RIGHT_MARGIN_LOGICAL,
            panel.width,
            ox,
            ow,
        );
        let y = oy + MENU_BAR_OFFSET_LOGICAL;
        (x, y, "top_right")
    };

    eprintln!(
        "[meterbar] fallback via={via} mon=({ox:.0},{oy:.0},{ow:.0}) scale={:.2} panel=({x:.1},{y:.1}) size=({:.0}x{:.0}) cursor={:?}",
        monitor.scale_factor(),
        panel.width,
        panel.height,
        cursor.map(|c| (c.x, c.y))
    );
    set_panel_logical_position(window, x, y);
}

/// 托盘 id：macOS 上用于取出 NSStatusItem → button.window.screen。
const METERBAR_TRAY_ID: &str = "meterbar";

/// 从 tray 图标查询几何（菜单 Open / 启动 show 时常无 click rect）。
fn tray_icon_rect(app: &tauri::AppHandle) -> Option<Rect> {
    if let Some(tray) = app.tray_by_id(METERBAR_TRAY_ID) {
        match tray.rect() {
            Ok(Some(r)) => {
                let size = physical_tray_size(&r, 1.0);
                if size.width > 0.5 && size.height > 0.5 {
                    remember_tray_rect(&r);
                    return Some(r);
                }
            }
            Ok(None) => {
                eprintln!("[meterbar] tray.rect() = None");
            }
            Err(e) => {
                eprintln!("[meterbar] tray.rect() err={e}");
            }
        }
    }
    cached_tray_rect()
}

/// 每次 show/toggle 重算面板位置。
///
/// macOS：以「被点击的 status item 窗口 / click tray_rect」定屏为不变量；
/// 禁止用全局唯一 `NSStatusItem.button.window`（多屏镜像菜单栏时常锚到主屏那份）。
/// 无 click 信息时：全屏 menubar 焦点屏 → mouse →（弱）status item。
/// 非 macOS 或 AppKit 失败时退回 tao 逻辑；Linux 优先 click rect，否则 `tray.rect()`。
fn position_panel_near_tray(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    tray_rect: Option<Rect>,
) {
    let tray_rect = tray_rect.or_else(|| tray_icon_rect(app));

    #[cfg(target_os = "macos")]
    {
        if position_panel_via_appkit(app, window, tray_rect) {
            return;
        }
        eprintln!("[meterbar] appkit position unavailable, falling back to tao");
    }
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

/// 点击事件里的 tray 物理矩形（tray-icon 按 **被点击窗口** 的 backingScaleFactor 编码）。
#[cfg(target_os = "macos")]
fn tray_event_physical(tray_rect: &Rect) -> Option<(f64, f64, f64, f64)> {
    let (x, y) = match tray_rect.position {
        Position::Physical(p) => (p.x as f64, p.y as f64),
        Position::Logical(_) => return None,
    };
    let (w, h) = match tray_rect.size {
        Size::Physical(s) => (s.width as f64, s.height as f64),
        Size::Logical(_) => return None,
    };
    if w <= 1.0 || h <= 1.0 {
        return None;
    }
    Some((x, y, w, h))
}

/// 用候选屏 scale 把 tray 物理坐标还原为 Cocoa 逻辑点，仅校验中心 X + 图标高度。
/// 返回 (anchor_x, score)；score 越小越像 22pt 菜单栏图标。
#[cfg_attr(not(test), allow(dead_code))]
fn cocoa_tray_hit_on_screen(
    screen_x: f64,
    screen_w: f64,
    scale: f64,
    tray_phys_x: f64,
    tray_phys_w: f64,
    tray_phys_h: f64,
) -> Option<(f64, f64)> {
    if scale <= 0.0 {
        return None;
    }
    let logical_h = tray_phys_h / scale;
    if logical_h < 8.0 || logical_h > 48.0 {
        return None;
    }
    let logical_x = tray_phys_x / scale;
    let logical_w = tray_phys_w / scale;
    let cx = logical_x + logical_w / 2.0;
    if cx >= screen_x && cx < screen_x + screen_w {
        let score = (logical_h - TRAY_ICON_LOGICAL_HEIGHT).abs();
        Some((cx, score))
    } else {
        None
    }
}

/// status item window 几何（Send）：仅作弱回退；多屏时常是主屏那一份镜像。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct StatusItemGeom {
    anchor_x: f64,
    screen_x: f64,
    screen_y: f64,
    screen_w: f64,
    screen_h: f64,
}

/// 从 tray-icon 的**唯一** NSStatusItem 取 button.window.screen。
/// 注意：多屏「每屏菜单栏各一份」时，这通常只反映主屏/全局那份，不能代表点击目标。
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

/// 点击当下的 NSEvent.window：tray-icon 用它生成 rect，多屏时应是被点的那份 status item 窗。
#[cfg(target_os = "macos")]
fn appkit_click_event_status_window(
    mtm: objc2::MainThreadMarker,
) -> Option<(objc2::rc::Retained<objc2_app_kit::NSScreen>, f64)> {
    use objc2_app_kit::NSApplication;

    let app = NSApplication::sharedApplication(mtm);
    let event = app.currentEvent()?;
    let window = event.window(mtm)?;
    let screen = window.screen()?;
    let wf = window.frame();
    // status item 窗高度通常很小；排除我们自己的面板窗误命中。
    if wf.size.height > 80.0 {
        return None;
    }
    let anchor_x = wf.origin.x + wf.size.width / 2.0;
    Some((screen, anchor_x))
}

/// 用 click tray_rect 的物理坐标 + 各屏 scale 选屏（只信 X；Y 经 pixels_high 翻转不可靠）。
#[cfg(target_os = "macos")]
fn appkit_screen_from_tray_rect(
    mtm: objc2::MainThreadMarker,
    tray_rect: &Rect,
) -> Option<(objc2::rc::Retained<objc2_app_kit::NSScreen>, f64)> {
    use objc2_app_kit::NSScreen;

    let (px, _py, pw, ph) = tray_event_physical(tray_rect)?;
    let screens = NSScreen::screens(mtm);
    let mut best: Option<(f64, objc2::rc::Retained<objc2_app_kit::NSScreen>, f64)> = None;
    let count = screens.count();
    for i in 0..count {
        let screen = screens.objectAtIndex(i);
        let f = screen.frame();
        let scale = screen.backingScaleFactor();
        if let Some((anchor_x, score)) =
            cocoa_tray_hit_on_screen(f.origin.x, f.size.width, scale, px, pw, ph)
        {
            let replace = best
                .as_ref()
                .is_none_or(|(best_score, ..)| score < *best_score);
            if replace {
                eprintln!(
                    "[meterbar] tray_rect_hit screen={} scale={:.2} phys=({:.1},{:.1})x({:.1}x{:.1}) anchor_x={:.1} score={:.2}",
                    screen.localizedName(),
                    scale,
                    px,
                    _py,
                    pw,
                    ph,
                    anchor_x,
                    score
                );
                best = Some((score, screen, anchor_x));
            }
        }
    }
    best.map(|(_, screen, anchor_x)| (screen, anchor_x))
}

/// 选锚屏（不变量：点哪块 status item，面板就在哪块物理屏菜单栏下）。
///
/// 优先级：
/// 1. click `tray_rect` 按各屏 scale 解码（tray-icon 在 mouseUp 时用 **event.window** 编码，最可靠）
/// 2. `NSApp.currentEvent.window`（同一次事件循环内可能仍是点击窗；post_show 时通常已失效）
/// 3. 全屏 menubar 焦点屏（mouse≠main）
/// 4. mouse 所在屏
/// 5. 全局 NSStatusItem（弱回退；多屏镜像菜单栏时常是主屏那份）
/// 6. main
#[cfg(target_os = "macos")]
fn appkit_resolve_anchor_screen(
    app: &tauri::AppHandle,
    mtm: objc2::MainThreadMarker,
    tray_rect: Option<Rect>,
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
    let click_event = appkit_click_event_status_window(mtm);
    let click_rect = tray_rect
        .as_ref()
        .and_then(|r| appkit_screen_from_tray_rect(mtm, r));

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
    let click_event_name = click_event
        .as_ref()
        .map(|(s, _)| s.localizedName().to_string())
        .unwrap_or_else(|| "-".into());
    let click_rect_name = click_rect
        .as_ref()
        .map(|(s, _)| s.localizedName().to_string())
        .unwrap_or_else(|| "-".into());
    let menubar_focus_mismatch = match (&main, &mouse_screen) {
        (Some(m), Some(ms)) => !nsscreen_same(m, ms),
        _ => false,
    };

    // 诊断：全局 status item 与点击目标不一致时，旧逻辑会把副屏点击锚到主屏。
    if let (Some((status_s, _)), Some((click_s, _))) = (&status_screen, &click_event) {
        if !nsscreen_same(status_s, click_s) {
            eprintln!(
                "[meterbar] status_item_mismatch status={} click_event={} (ignoring global NSStatusItem)",
                status_s.localizedName(),
                click_s.localizedName()
            );
        }
    } else if let (Some((status_s, _)), Some((click_s, _))) = (&status_screen, &click_rect) {
        if !nsscreen_same(status_s, click_s) {
            eprintln!(
                "[meterbar] status_item_mismatch status={} click_rect={} (ignoring global NSStatusItem)",
                status_s.localizedName(),
                click_s.localizedName()
            );
        }
    }

    eprintln!(
        "[meterbar] screen_pick mouse=({:.1},{:.1}) mouse_screen={mouse_name} main={main_name} status_item={status_name} click_event={click_event_name} click_rect={click_rect_name} menubar_focus_mismatch={menubar_focus_mismatch} has_tray_rect={}",
        mouse.x,
        mouse.y,
        tray_rect.is_some()
    );

    if let Some((screen, anchor_x)) = click_rect {
        return Some((screen, anchor_x, "click_tray_rect"));
    }
    if let Some((screen, anchor_x)) = click_event {
        return Some((screen, anchor_x, "click_event_window"));
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

    if let Some((screen, anchor_x)) = status_screen {
        eprintln!(
            "[meterbar] weak_fallback status_item_screen={}",
            screen.localizedName()
        );
        return Some((screen, anchor_x, "status_item_screen"));
    }

    main.map(|s| (s, mouse.x, "main_screen_fallback"))
}

/// AppKit：按点击 status item 所在屏的 visibleFrame 顶边锚面板。
/// 返回 true 表示已成功设置 frame。
#[cfg(target_os = "macos")]
fn position_panel_via_appkit(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    tray_rect: Option<Rect>,
) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSPoint, NSRect};

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
    let Some((screen, anchor_x, via)) =
        appkit_resolve_anchor_screen(app, mtm, tray_rect, true)
    else {
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

/// show / orderFront / set_focus 之后：强制按**同一次点击**的锚屏再钉一次 frame。
///
/// 注意：`NSWindow.screen` / frame 中心只能证明坐标落在哪块物理屏，**不能**证明
/// Space 是否正确。必须传入原始 `tray_rect`，否则 post_show 会丢掉点击信息、
/// 退回全局 NSStatusItem 并再次锚错屏。
#[cfg(target_os = "macos")]
fn correct_panel_frame_after_show(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    tray_rect: Option<Rect>,
) {
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

    let Some((target, _, via)) = appkit_resolve_anchor_screen(app, mtm, tray_rect, false) else {
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
    let _ = position_panel_via_appkit(app, window, tray_rect);
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
/// Apple DTS 完整配方含真正的 `NSPanel` + `NonactivatingPanel`。Tauri/Tao 主窗
/// 是 `NSWindow`：`object_setClass` 会因实例大小不同 abort；把 contentView 迁走
/// 会让 Tao 在 `contentView().unwrap()` 处 panic。因此这里落地 DTS 中可在
/// **现有 NSWindow** 上生效的部分；真正 NSPanel 需在窗口创建期替换（后续大改）。
///
/// 当前组合：
/// - Accessory / LSUIElement（进程早设 + Info.plist）
/// - `CanJoinAllSpaces | CanJoinAllApplications | FullScreenAuxiliary | Stationary`
/// - level = `NSScreenSaverWindowLevel`（PopUpMenu/Floating 会被原生全屏压住）
/// - `orderFrontRegardless`；show 后重钉 level（防 alwaysOnTop 打回 Floating）
///
/// **禁止 `MoveToActiveSpace`**：多屏错 Space。tiling 互斥位必须恰好其一。
#[cfg(target_os = "macos")]
fn configure_panel_for_fullscreen_spaces(window: &WebviewWindow) {
    let Ok(ns_window_ptr) = window.ns_window() else {
        return;
    };
    if ns_window_ptr.is_null() {
        return;
    }

    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindow, NSWindowCollectionBehavior};

    unsafe {
        let ns_window = &*(ns_window_ptr as *const NSWindow);

        let mut behavior = ns_window.collectionBehavior();
        behavior.remove(NSWindowCollectionBehavior::MoveToActiveSpace);
        behavior.insert(NSWindowCollectionBehavior::CanJoinAllSpaces);
        // 跨 App 全屏 Space（仅 CanJoinAllSpaces 不够，见 Apple DTS）。
        behavior.insert(NSWindowCollectionBehavior::CanJoinAllApplications);
        // Managed / Transient / Stationary 三选一。
        // Stationary：不受 Exposé /「显示桌面」推开；进全屏 Space 靠
        // CanJoinAllApplications + FullScreenAuxiliary，不靠 MoveToActiveSpace。
        behavior.remove(NSWindowCollectionBehavior::Managed);
        behavior.remove(NSWindowCollectionBehavior::Transient);
        behavior.insert(NSWindowCollectionBehavior::Stationary);
        behavior.remove(NSWindowCollectionBehavior::ParticipatesInCycle);
        behavior.insert(NSWindowCollectionBehavior::IgnoresCycle);
        behavior.remove(NSWindowCollectionBehavior::FullScreenPrimary);
        behavior.remove(NSWindowCollectionBehavior::FullScreenNone);
        behavior.insert(NSWindowCollectionBehavior::FullScreenAuxiliary);
        behavior.remove(NSWindowCollectionBehavior::FullScreenAllowsTiling);
        behavior.insert(NSWindowCollectionBehavior::FullScreenDisallowsTiling);
        ns_window.setCollectionBehavior(behavior);

        // PopUpMenu(101) 仍可能被原生全屏压住；DTS 验证用 ScreenSaver(1000)。
        ns_window.setLevel(NSScreenSaverWindowLevel);
        ns_window.setHidesOnDeactivate(false);
        eprintln!(
            "[meterbar] collectionBehavior=CanJoinAllSpaces|CanJoinAllApplications|Stationary|FullScreenAuxiliary|DisallowsTiling (no MoveToActiveSpace) level={} (NSWindow path)",
            NSScreenSaverWindowLevel
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
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSWindow};
    unsafe {
        let ns_window = &*(ns_window_ptr as *const NSWindow);
        // show/set_focus 后 Tao alwaysOnTop 可能把 level 打回 Floating(3)。
        ns_window.setLevel(NSScreenSaverWindowLevel);
        ns_window.orderFrontRegardless();
        ns_window.makeKeyAndOrderFront(None);
    }
}

/// 面板是否「真正在前台可交互」：不能只信 tauri is_visible。
/// 「点壁纸显示桌面」后窗口常仍 is_visible=true，但 occlusion 已不可见 / 非 key。
#[cfg(target_os = "macos")]
fn panel_is_effectively_shown(window: &WebviewWindow) -> bool {
    let focused = window.is_focused().unwrap_or(false);
    let Ok(ns_window_ptr) = window.ns_window() else {
        return window.is_visible().unwrap_or(false) && focused;
    };
    if ns_window_ptr.is_null() {
        return window.is_visible().unwrap_or(false) && focused;
    }
    use objc2_app_kit::{NSWindow, NSWindowOcclusionState};
    unsafe {
        let ns_window = &*(ns_window_ptr as *const NSWindow);
        let visible = ns_window.isVisible();
        let occlusion_visible = ns_window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible);
        let key = ns_window.isKeyWindow();
        eprintln!(
            "[meterbar] panel_effective visible={visible} occlusion_visible={occlusion_visible} key={key} focused={focused}"
        );
        visible && occlusion_visible && (key || focused)
    }
}

#[cfg(not(target_os = "macos"))]
fn panel_is_effectively_shown(window: &WebviewWindow) -> bool {
    // Wayland/GNOME 下无边框窗常 is_focused=false，不能与 macOS 一样强依赖 focus。
    window.is_visible().unwrap_or(false)
}

fn show_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("[meterbar] show_panel: main window missing");
        return;
    };
    if let Some(ref r) = tray_rect {
        remember_tray_rect(r);
    }
    let tray_rect = tray_rect.or_else(|| tray_icon_rect(app));

    #[cfg(target_os = "macos")]
    configure_panel_for_fullscreen_spaces(&window);

    // 尽早开 grace，覆盖 show→focus 期间的 Focused(false)。
    mark_panel_shown();

    // Linux：先 show 再定位（X11/Wayland 在 map 前 set_position 常被丢掉 → outer 一直 0,0）。
    #[cfg(not(target_os = "macos"))]
    {
        let show_res = window.show();
        let _ = window.unminimize();
        position_panel_near_tray(app, &window, tray_rect);
        let focus_res = window.set_focus();
        // 再钉一次：show 后 outer_size 才可靠，且 WM 已 map 窗口。
        position_panel_near_tray(app, &window, tray_rect);
        mark_panel_shown();

        let app2 = app.clone();
        let _ = window.run_on_main_thread(move || {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(80));
                if let Some(w) = app2.get_webview_window("main") {
                    let _ = w.show();
                    // 优先复用上次逻辑坐标，避免 reanchor 时又去读鼠标。
                    if let Some((x, y)) = last_panel_logical_pos() {
                        set_panel_logical_position(&w, x, y);
                    } else {
                        position_panel_near_tray(&app2, &w, tray_icon_rect(&app2));
                    }
                    let _ = w.set_focus();
                    eprintln!(
                        "[meterbar] linux reanchor +80ms visible={:?} outer={:?}",
                        w.is_visible(),
                        w.outer_position()
                    );
                }
            });
        });

        eprintln!(
            "[meterbar] show_panel(linux) show={show_res:?} focus={focus_res:?} visible={:?} outer={:?}",
            window.is_visible(),
            window.outer_position(),
        );
        return;
    }

    #[cfg(target_os = "macos")]
    {
        position_panel_near_tray(app, &window, tray_rect);
        let show_res = window.show();
        position_panel_near_tray(app, &window, tray_rect);
        order_panel_front(&window);
        correct_panel_frame_after_show(app, &window, tray_rect);
        let focus_res = window.set_focus();
        order_panel_front(&window);
        correct_panel_frame_after_show(app, &window, tray_rect);
        mark_panel_shown();
        eprintln!(
            "[meterbar] show_panel show={show_res:?} focus={focus_res:?} visible={:?} focused={:?} outer={:?}",
            window.is_visible(),
            window.is_focused(),
            window.outer_position(),
        );
    }
}

fn hide_panel_if_blurred(window: &WebviewWindow) {
    // Wayland/GNOME：Focused(false) 不可靠，且 show 后常立即失焦；
    // 关闭只靠托盘左键 toggle，避免「闪一下就没了」。
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        eprintln!("[meterbar] hide_panel skipped (linux: tray-toggle only, no blur-hide)");
        return;
    }
    #[cfg(target_os = "macos")]
    {
        if !should_hide_on_blur() {
            eprintln!("[meterbar] hide_panel skipped (blur grace / drag)");
            return;
        }
        eprintln!("[meterbar] hide_panel on blur");
        let _ = window.hide();
    }
}

fn toggle_panel(app: &tauri::AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("[meterbar] toggle_panel: main window missing");
        return;
    };

    // 显示桌面 / 全屏 Space：is_visible 可能仍为 true 但实际不可见 → 必须走 show。
    let effectively_shown = panel_is_effectively_shown(&window);
    eprintln!(
        "[meterbar] tray toggle effectively_shown={effectively_shown} visible={:?} focused={:?}",
        window.is_visible(),
        window.is_focused()
    );
    if effectively_shown {
        let _ = window.hide();
    } else {
        show_panel(app, tray_rect);
    }
}

/// 在 Tauri 建窗之前把激活策略打成 Accessory。
/// setup() 里再设往往已晚（窗口已按 Regular 创建）。Info.plist LSUIElement 覆盖 .app 启动。
#[cfg(target_os = "macos")]
fn apply_accessory_activation_policy_early() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[meterbar] early Accessory policy skipped (not main thread)");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let ok = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    eprintln!("[meterbar] early NSApplicationActivationPolicyAccessory ok={ok}");
}

/// 清理旧版 LaunchAgent（指向 Contents/MacOS 裸二进制），避免登录项显示 exec/usages。
/// 返回是否删除了至少一个 plist（调用方可迁移为 AppleScript 登录项）。
#[cfg(target_os = "macos")]
fn remove_legacy_autostart_launch_agents() -> bool {
    let Ok(home) = std::env::var("HOME") else {
        return false;
    };
    let agents = std::path::PathBuf::from(home).join("Library/LaunchAgents");
    let mut removed = false;
    for name in ["Meterbar.plist", "usages.plist"] {
        let path = agents.join(name);
        if path.is_file() {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    eprintln!("[meterbar] removed legacy LaunchAgent {}", path.display());
                    removed = true;
                }
                Err(e) => {
                    eprintln!(
                        "[meterbar] failed to remove legacy LaunchAgent {}: {e}",
                        path.display()
                    );
                }
            }
        }
    }
    removed
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    apply_accessory_activation_policy_early();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // macOS：必须用 AppleScript 注册 .app bundle；LaunchAgent 会注册裸二进制，
        // 导致「登录项 / 允许在后台」显示 exec 图标与可执行文件名（如 usages）。
        .plugin({
            #[cfg(target_os = "macos")]
            {
                tauri_plugin_autostart::Builder::new()
                    .app_name("Meterbar")
                    .macos_launcher(tauri_plugin_autostart::MacosLauncher::AppleScript)
                    .build()
            }
            #[cfg(not(target_os = "macos"))]
            {
                tauri_plugin_autostart::Builder::new()
                    .app_name("Meterbar")
                    .build()
            }
        })
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                // 双保险：建窗后仍保持 Accessory。
                let _ = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);

                // 旧 LaunchAgent → 迁移为 AppleScript 登录项（显示 Meterbar.app 名称/图标）。
                if remove_legacy_autostart_launch_agents() {
                    use tauri_plugin_autostart::ManagerExt;
                    if let Err(e) = app.autolaunch().enable() {
                        eprintln!("[meterbar] migrate autostart to login item failed: {e}");
                    }
                }
            }

            let open = MenuItem::with_id(app, "open", "Open Panel", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Meterbar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            // macOS：黑字透明 template；Linux：彩色 app 图标（黑 template 在深色顶栏几乎不可见）。
            #[cfg(target_os = "macos")]
            let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
                .expect("failed to load tray icon");
            #[cfg(not(target_os = "macos"))]
            let icon = Image::from_bytes(include_bytes!("../icons/tray-icon-linux.png"))
                .expect("failed to load tray icon");

            let _tray = TrayIconBuilder::with_id(METERBAR_TRAY_ID)
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .menu(&menu)
                // 左键 toggle 面板（带 tray rect 锚点）；右键菜单。勿在 Linux 上左键只出菜单，否则无法锚到图标。
                .show_menu_on_left_click(false)
                .tooltip("Meterbar — 左键打开面板，右键菜单")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_panel(app, tray_icon_rect(app)),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            rect,
                            ..
                        } => {
                            remember_tray_rect(&rect);
                            toggle_panel(tray.app_handle(), Some(rect));
                        }
                        TrayIconEvent::Click {
                            button: MouseButton::Right,
                            button_state: MouseButtonState::Up,
                            rect,
                            ..
                        } => {
                            // 右键出菜单前也缓存几何，便于菜单「Open」锚点。
                            remember_tray_rect(&rect);
                        }
                        TrayIconEvent::Enter { rect, .. }
                        | TrayIconEvent::Move { rect, .. } => {
                            remember_tray_rect(&rect);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            eprintln!(
                "[meterbar] tray ready — left-click panel under icon; right-click menu; Open uses cached tray geom"
            );

            let app_handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                {
                    // 窗口 + WKWebView 均需透明，否则 NSVisualEffectView 被不透明白底盖住
                    if let Err(err) = window.set_background_color(Some(Color(0, 0, 0, 0))) {
                        eprintln!("set_background_color failed: {err}");
                    }
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
                #[cfg(not(target_os = "macos"))]
                {
                    // 不透明底 + 进任务栏；Wayland 下 transparent 窗常无法定位/显示。
                    if let Err(err) = window.set_background_color(Some(Color(248, 249, 251, 255))) {
                        eprintln!("set_background_color failed: {err}");
                    }
                    let _ = window.set_skip_taskbar(false);
                    let _ = window.set_always_on_top(true);
                    // 不在启动时强制 show：tray.rect/monitor 常未就绪，且会触发 blur 竞态。
                    // 用户左键托盘后再弹出（此时定位与显示更稳）。
                    eprintln!(
                        "[meterbar] linux ready — click tray icon to open panel (no auto-popup)"
                    );
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
            commands::refresh_grok,
            commands::refresh_system,
            commands::refresh_system_fast,
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
    fn click_tray_rect_beats_global_status_item_on_external() {
        // 副屏点击：tray-icon 按副屏 window scale=1 编码；全局 NSStatusItem 仍报主屏。
        // 不变量：必须以 click_tray_rect 命中副屏，不能锚主屏。
        use super::cocoa_tray_hit_on_screen;

        let primary = (0.0, 1728.0, 2.0); // x, w, scale
        let external = (1728.0, 1920.0, 1.0);
        // 副屏图标 cocoa x=2000、h=22 → phys (2000, 22) @ scale 1
        let tray_phys_x = 2000.0;
        let tray_phys_w = 30.0;
        let tray_phys_h = 22.0;

        let hit_primary = cocoa_tray_hit_on_screen(
            primary.0, primary.1, primary.2, tray_phys_x, tray_phys_w, tray_phys_h,
        );
        let hit_external = cocoa_tray_hit_on_screen(
            external.0, external.1, external.2, tray_phys_x, tray_phys_w, tray_phys_h,
        );
        assert!(hit_external.is_some(), "副屏 scale 应命中");
        // 误用主屏 scale=2 时 X 落到主屏，但图标逻辑高度偏离 22 → score 更差或仍可能命中；
        // 择优取 score 更小者必须是副屏。
        let chosen = match (hit_primary, hit_external) {
            (Some((_, sp)), Some((ax, se))) if se <= sp => ("external", ax),
            (None, Some((ax, _))) => ("external", ax),
            (Some((ax, _)), None) => ("primary", ax),
            (Some((ax, sp)), Some((_, se))) if sp < se => ("primary", ax),
            _ => panic!("no hit"),
        };
        assert_eq!(chosen.0, "external");
        assert!((chosen.1 - 2015.0).abs() < 0.1); // 2000 + 30/2
    }

    #[test]
    fn click_tray_rect_on_retina_primary_not_external() {
        use super::cocoa_tray_hit_on_screen;

        let primary = (0.0, 1512.0, 2.0);
        let external = (1512.0, 1920.0, 1.0);
        // 主屏逻辑 x=1400 h=22 → phys 2800 / 44 @ scale 2
        let tray_phys_x = 1400.0 * 2.0;
        let tray_phys_w = 30.0 * 2.0;
        let tray_phys_h = 22.0 * 2.0;

        let hit_primary = cocoa_tray_hit_on_screen(
            primary.0, primary.1, primary.2, tray_phys_x, tray_phys_w, tray_phys_h,
        );
        let hit_external = cocoa_tray_hit_on_screen(
            external.0, external.1, external.2, tray_phys_x, tray_phys_w, tray_phys_h,
        );
        assert!(hit_primary.is_some());
        // 物理 X=2800 若按 scale=1 会落在拓展屏，但高度 44 的 score 差于主屏的 0
        let (name, _) = match (hit_primary, hit_external) {
            (Some((ax, sp)), Some((_, se))) if sp <= se => ("primary", ax),
            (Some((ax, _)), None) => ("primary", ax),
            (None, Some((ax, _))) => ("external", ax),
            (Some((_, sp)), Some((ax, se))) if se < sp => ("external", ax),
            _ => panic!("no hit"),
        };
        assert_eq!(name, "primary");
    }

    #[test]
    fn without_click_menubar_focus_beats_mouse_on_content_screen() {
        // 无 tray_rect 时：全屏 mouse 在主屏、main 在副屏 → 跟菜单栏焦点屏。
        let primary: (f64, f64, f64, f64) = (0.0, 0.0, 1728.0, 1117.0);
        let external: (f64, f64, f64, f64) = (-896.0, -516.0, 896.0, 1344.0);
        let mouse: (f64, f64) = (1187.5, 1099.1);
        let main = external;
        let mouse_on_primary = cocoa_point_in_rect(
            mouse.0, mouse.1, primary.0, primary.1, primary.2, primary.3,
        );
        let menubar_focus_mismatch = mouse_on_primary;
        let status_item_is_primary = true; // 全局 NSStatusItem 常锚错到主屏
        // 新决策：无 click 时 menubar mismatch → main，不再让错误的 status_item 抢先。
        let chosen = if menubar_focus_mismatch {
            main
        } else if status_item_is_primary {
            primary
        } else {
            external
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
