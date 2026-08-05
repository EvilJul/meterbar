//! 面板与 Provider 共享 DTO（骨架；字段按 design 预留）。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// 用量单位。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UsageUnit {
    Cents,
    Requests,
    Tokens,
    /// 账户余额（如 DeepSeek `total_balance`）。
    Balance,
    Unknown,
}

/// Provider 取数状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Ok,
    NeedsAuth,
    ParseError,
    NetworkError,
    Unsupported,
}

/// 归一化用量快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub provider: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub membership: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_end: Option<String>,
    pub unit: UsageUnit,
    pub used: f64,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    /// 总量已用百分比（如 Cursor `totalPercentUsed`）。
    pub percent_used: Option<f64>,
    /// Cursor Auto 额度已用百分比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_percent_used: Option<f64>,
    /// Cursor API 额度已用百分比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_percent_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_demand_used: Option<f64>,
    pub status: ProviderStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_version: Option<String>,
}

/// 本机系统指标快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub cpu_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_temp_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_temp_c: Option<f64>,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub fetched_at: String,
}

/// 延迟探测状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatencyStatus {
    Ok,
    Timeout,
    Error,
}

/// 延迟探测快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySnapshot {
    pub target: String,
    pub latency_ms: Option<u64>,
    pub status: LatencyStatus,
    pub fetched_at: String,
    /// 当前出口 IP 对应的城市/国家文案；查询失败时为 `None`。
    pub region_label: Option<String>,
}

/// 面板聚合状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelState {
    pub usages: Vec<UsageSnapshot>,
    pub system: SystemSnapshot,
    pub latency: LatencySnapshot,
    /// Cursor 用量自动刷新间隔（秒），默认建议 300。
    pub auto_refresh_sec: u64,
    /// 系统与延迟自动刷新间隔（秒）。
    pub system_refresh_sec: u64,
    /// 高延迟阈值（毫秒）；UI 标红。
    pub high_latency_ms: u64,
    /// Keychain 中是否已配置 Cursor 会话 token（不回显内容）。
    pub has_cursor_token: bool,
    /// 是否已配置 DeepSeek API Key（不回显内容）。
    pub has_deepseek_key: bool,
}

/// 应用设置（内存态；完整持久化留给后续任务）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Cursor 用量自动刷新间隔（秒），默认 300，最小 60。
    pub cursor_refresh_sec: u64,
    /// 系统指标刷新间隔（秒），默认 15，范围 10–30。
    pub system_refresh_sec: u64,
    /// 延迟探测目标 URL，默认 `https://cursor.com`。
    pub latency_target: String,
    /// 高延迟阈值（毫秒）；UI 标红（§6）使用此字段。
    pub high_latency_ms: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cursor_refresh_sec: Self::DEFAULT_CURSOR_REFRESH_SEC,
            system_refresh_sec: Self::DEFAULT_SYSTEM_REFRESH_SEC,
            latency_target: Self::DEFAULT_LATENCY_TARGET.to_string(),
            high_latency_ms: Self::DEFAULT_HIGH_LATENCY_MS,
        }
    }
}

impl AppSettings {
    pub const MIN_CURSOR_REFRESH_SEC: u64 = 60;
    pub const DEFAULT_CURSOR_REFRESH_SEC: u64 = 300;

    pub const MIN_SYSTEM_REFRESH_SEC: u64 = 10;
    pub const MAX_SYSTEM_REFRESH_SEC: u64 = 30;
    pub const DEFAULT_SYSTEM_REFRESH_SEC: u64 = 15;

    pub const DEFAULT_LATENCY_TARGET: &'static str = "https://cursor.com";
    pub const DEFAULT_HIGH_LATENCY_MS: u64 = 500;
    pub const MIN_HIGH_LATENCY_MS: u64 = 1;

    pub fn clamp_cursor_refresh_sec(sec: u64) -> u64 {
        sec.max(Self::MIN_CURSOR_REFRESH_SEC)
    }

    pub fn clamp_system_refresh_sec(sec: u64) -> u64 {
        sec.clamp(Self::MIN_SYSTEM_REFRESH_SEC, Self::MAX_SYSTEM_REFRESH_SEC)
    }

    pub fn clamp_high_latency_ms(ms: u64) -> u64 {
        ms.max(Self::MIN_HIGH_LATENCY_MS)
    }

    pub fn normalize_latency_target(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::DEFAULT_LATENCY_TARGET.to_string();
        }
        if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        }
    }
}

/// 设置部分更新。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsUpdate {
    pub cursor_refresh_sec: Option<u64>,
    pub system_refresh_sec: Option<u64>,
    pub latency_target: Option<String>,
    pub high_latency_ms: Option<u64>,
}
