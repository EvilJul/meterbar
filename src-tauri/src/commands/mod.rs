//! Tauri invoke 命令面。

use std::sync::Mutex;

use tauri::State;

use crate::credentials;
use crate::models::{
    AppSettings, AppSettingsUpdate, LatencySnapshot, LatencyStatus, PanelState, SystemSnapshot,
    UsageSnapshot,
};
use crate::network;
use crate::providers::{codex, cursor, deepseek};
use crate::settings;
use crate::system;

/// 进程内设置与最近快照缓存。
pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub last_usages: Mutex<Vec<UsageSnapshot>>,
    pub last_system: Mutex<Option<SystemSnapshot>>,
    pub last_latency: Mutex<Option<LatencySnapshot>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            settings: Mutex::new(settings::load()),
            last_usages: Mutex::new(Vec::new()),
            last_system: Mutex::new(None),
            last_latency: Mutex::new(None),
        }
    }
}

fn has_cursor_token() -> bool {
    credentials::has_any_auth()
}

fn has_deepseek_key() -> bool {
    credentials::has_deepseek_key()
}

fn placeholder_system() -> SystemSnapshot {
    SystemSnapshot {
        cpu_percent: 0.0,
        cpu_temp_c: None,
        gpu_percent: None,
        gpu_temp_c: None,
        mem_used_bytes: 0,
        mem_total_bytes: 0,
        disk_used_bytes: None,
        disk_available_bytes: None,
        vpn_ip: None,
        fetched_at: String::new(),
    }
}

fn placeholder_latency(target: &str) -> LatencySnapshot {
    LatencySnapshot {
        target: target.to_string(),
        latency_ms: None,
        status: LatencyStatus::Error,
        fetched_at: String::new(),
        region_label: None,
        egress_ip: None,
    }
}

fn build_panel_state(state: &AppState) -> Result<PanelState, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "读取设置失败".to_string())?
        .clone();
    let usages = state
        .last_usages
        .lock()
        .map_err(|_| "读取用量缓存失败".to_string())?
        .clone();
    let system = state
        .last_system
        .lock()
        .map_err(|_| "读取系统缓存失败".to_string())?
        .clone()
        .unwrap_or_else(placeholder_system);
    let latency = state
        .last_latency
        .lock()
        .map_err(|_| "读取延迟缓存失败".to_string())?
        .clone()
        .unwrap_or_else(|| placeholder_latency(&settings.latency_target));

    Ok(PanelState {
        usages,
        system,
        latency,
        auto_refresh_sec: settings.cursor_refresh_sec,
        cpu_gpu_refresh_sec: settings.cpu_gpu_refresh_sec,
        system_refresh_sec: settings.system_refresh_sec,
        high_latency_ms: settings.high_latency_ms,
        has_cursor_token: has_cursor_token(),
        has_deepseek_key: has_deepseek_key(),
    })
}

fn store_usage(state: &AppState, snap: &UsageSnapshot) {
    if let Ok(mut guard) = state.last_usages.lock() {
        if let Some(existing) = guard.iter_mut().find(|u| u.provider == snap.provider) {
            *existing = snap.clone();
        } else {
            // Cursor 置顶，其余按插入顺序跟在后面。
            if snap.provider == "cursor" {
                guard.insert(0, snap.clone());
            } else if let Some(idx) = guard.iter().position(|u| u.provider == "cursor") {
                guard.insert(idx + 1, snap.clone());
            } else {
                guard.push(snap.clone());
            }
        }
    }
}

/// 将 Cursor 会话 token 双写本机（Keychain + 本地备份）。
/// `set_token` 内已回读校验；此处再确认一次。不回显 token。
/// 兼容 invoke 参数名 `token` / `sessionToken` / `session_token`。
#[tauri::command]
pub fn set_cursor_session_token(
    token: Option<String>,
    session_token: Option<String>,
    #[allow(non_snake_case)] sessionToken: Option<String>,
) -> Result<(), String> {
    let token = token
        .or(session_token)
        .or(sessionToken)
        .ok_or_else(|| "缺少 token 参数".to_string())?;
    credentials::set_token(&token)?;
    match credentials::get_token()? {
        Some(stored) if !stored.trim().is_empty() => Ok(()),
        Some(_) => Err("保存后回读为空，Cookie 可能未写入".to_string()),
        None => Err("保存后回读失败：钥匙串与本地备份均无有效 Cookie".to_string()),
    }
}

/// 清除本机 Keychain 中的 Cursor 会话 token。
#[tauri::command]
pub fn clear_cursor_session_token() -> Result<(), String> {
    credentials::clear_token()
}

/// 保存 DeepSeek API Key（Keychain + 本地备份）。不回显 key。
#[tauri::command]
pub fn set_deepseek_api_key(key: Option<String>) -> Result<(), String> {
    let key = key.ok_or_else(|| "缺少 key 参数".to_string())?;
    credentials::set_deepseek_key(&key)?;
    match credentials::get_deepseek_key()? {
        Some(stored) if !stored.trim().is_empty() => Ok(()),
        Some(_) => Err("保存后回读为空，API Key 可能未写入".to_string()),
        None => Err("保存后回读失败：钥匙串与本地备份均无有效 API Key".to_string()),
    }
}

/// 清除 DeepSeek API Key。
#[tauri::command]
pub fn clear_deepseek_api_key() -> Result<(), String> {
    credentials::clear_deepseek_key()
}

/// 后端请求 Cursor usage-summary 并返回归一化快照。
#[tauri::command]
pub async fn refresh_cursor(state: State<'_, AppState>) -> Result<UsageSnapshot, String> {
    let snap = cursor::refresh().await;
    store_usage(&state, &snap);
    Ok(snap)
}

/// 拉取 DeepSeek 余额快照。
#[tauri::command]
pub async fn refresh_deepseek(state: State<'_, AppState>) -> Result<UsageSnapshot, String> {
    let snap = deepseek::refresh().await;
    store_usage(&state, &snap);
    Ok(snap)
}

/// 经本机 Codex app-server 拉取 rate limits 快照（失败返回错误态，不拖垮其它卡）。
#[tauri::command]
pub async fn refresh_codex(state: State<'_, AppState>) -> Result<UsageSnapshot, String> {
    let snap = codex::refresh().await;
    store_usage(&state, &snap);
    Ok(snap)
}

/// 快拍：仅刷新 CPU/GPU，保留缓存中的内存/磁盘/VPN。
#[tauri::command]
pub fn refresh_system_fast(state: State<'_, AppState>) -> SystemSnapshot {
    let fast = system::sample_fast();
    let snap = if let Ok(mut guard) = state.last_system.lock() {
        let merged = match guard.as_ref() {
            Some(prev) => SystemSnapshot {
                cpu_percent: fast.cpu_percent,
                cpu_temp_c: fast.cpu_temp_c,
                gpu_percent: fast.gpu_percent,
                gpu_temp_c: fast.gpu_temp_c,
                mem_used_bytes: prev.mem_used_bytes,
                mem_total_bytes: prev.mem_total_bytes,
                disk_used_bytes: prev.disk_used_bytes,
                disk_available_bytes: prev.disk_available_bytes,
                vpn_ip: prev.vpn_ip.clone(),
                fetched_at: fast.fetched_at,
            },
            None => fast,
        };
        *guard = Some(merged.clone());
        merged
    } else {
        fast
    };
    snap
}

/// 慢拍：刷新内存/磁盘/VPN，保留缓存中的 CPU/GPU。
#[tauri::command]
pub fn refresh_system(state: State<'_, AppState>) -> SystemSnapshot {
    let slow = system::sample_slow();
    let snap = if let Ok(mut guard) = state.last_system.lock() {
        let merged = match guard.as_ref() {
            Some(prev) => SystemSnapshot {
                cpu_percent: prev.cpu_percent,
                cpu_temp_c: prev.cpu_temp_c,
                gpu_percent: prev.gpu_percent,
                gpu_temp_c: prev.gpu_temp_c,
                mem_used_bytes: slow.mem_used_bytes,
                mem_total_bytes: slow.mem_total_bytes,
                disk_used_bytes: slow.disk_used_bytes,
                disk_available_bytes: slow.disk_available_bytes,
                vpn_ip: slow.vpn_ip.clone(),
                fetched_at: slow.fetched_at,
            },
            None => slow,
        };
        *guard = Some(merged.clone());
        merged
    } else {
        slow
    };
    snap
}

/// 按当前设置目标探测延迟；超时/失败返回非 ok 状态且不崩溃。
#[tauri::command]
pub async fn refresh_latency(state: State<'_, AppState>) -> Result<LatencySnapshot, String> {
    let target = state
        .settings
        .lock()
        .map(|g| g.latency_target.clone())
        .map_err(|_| "读取设置失败".to_string())?;
    let snap = network::probe(&target).await;
    if let Ok(mut guard) = state.last_latency.lock() {
        *guard = Some(snap.clone());
    }
    Ok(snap)
}

/// 读取当前设置。
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .settings
        .lock()
        .map(|g| g.clone())
        .map_err(|_| "读取设置失败".to_string())
}

/// 更新设置；各字段按约束钳位后写盘。写盘失败则严格回滚内存。
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    patch: AppSettingsUpdate,
) -> Result<AppSettings, String> {
    let mut guard = state
        .settings
        .lock()
        .map_err(|_| "更新设置失败".to_string())?;
    settings::apply_update(&mut guard, patch)
}

/// 返回最近快照缓存 + 当前刷新间隔（不强制网络请求）。
#[tauri::command]
pub fn get_panel_state(state: State<'_, AppState>) -> Result<PanelState, String> {
    build_panel_state(&state)
}

/// 诊断本机 Cursor 登录态探测（不含 token 明文）。
#[tauri::command]
pub fn diagnose_local_session() -> credentials::local_session::LocalSessionProbe {
    credentials::local_session::probe()
}

/// 刷新 Cursor / Codex / DeepSeek / System / Latency 后返回聚合面板状态。
#[tauri::command]
pub async fn refresh_all(state: State<'_, AppState>) -> Result<PanelState, String> {
    let target = state
        .settings
        .lock()
        .map(|g| g.latency_target.clone())
        .map_err(|_| "读取设置失败".to_string())?;

    // 顺序刷新：各 provider 失败互不拖垮（各自返回错误态快照）。
    let cursor_snap = cursor::refresh().await;
    store_usage(&state, &cursor_snap);

    // Codex best-effort：失败仅写入错误态快照，不影响后续 Cursor/System/Latency。
    let codex_snap = codex::refresh().await;
    store_usage(&state, &codex_snap);

    let deepseek_snap = deepseek::refresh().await;
    // 无 key 时不占卡片缓存（前端靠 has_deepseek_key 隐藏）；有 key 则始终更新。
    if credentials::has_deepseek_key() || deepseek_snap.status != crate::models::ProviderStatus::NeedsAuth
    {
        store_usage(&state, &deepseek_snap);
    } else if let Ok(mut guard) = state.last_usages.lock() {
        guard.retain(|u| u.provider != "deepseek");
    }

    let system_snap = system::sample();
    if let Ok(mut guard) = state.last_system.lock() {
        *guard = Some(system_snap);
    }

    let latency_snap = network::probe(&target).await;
    if let Ok(mut guard) = state.last_latency.lock() {
        *guard = Some(latency_snap);
    }

    build_panel_state(&state)
}
