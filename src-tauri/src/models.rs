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
    /// 启动盘已用字节（total − 实际剩余）；采集失败时为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_used_bytes: Option<u64>,
    /// 启动盘实际剩余字节（statfs 可用空间，不含可清除虚高）；采集失败时为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_available_bytes: Option<u64>,
    /// 本机 VPN/隧道网卡 IPv4（如 utun）；未连接时为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpn_ip: Option<String>,
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
    /// 当前公网出口 IP；查询失败时为 `None`（与区域同源探测）。
    pub egress_ip: Option<String>,
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
    /// CPU/GPU 自动刷新间隔（秒）。
    pub cpu_gpu_refresh_sec: u64,
    /// 其余系统指标与延迟自动刷新间隔（秒）：内存/磁盘/VPN/Latency。
    pub system_refresh_sec: u64,
    /// 高延迟阈值（毫秒）；UI 标红。
    pub high_latency_ms: u64,
    /// Keychain 中是否已配置 Cursor 会话 token（不回显内容）。
    pub has_cursor_token: bool,
    /// 是否已配置 DeepSeek API Key（不回显内容）。
    pub has_deepseek_key: bool,
    /// 是否发现本机 Grok / SuperGrok 登录态（不回显 token）。
    pub has_grok_auth: bool,
}

/// 看板供应商显示三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderVisibilityMode {
    Auto,
    Always,
    Hidden,
}

impl Default for ProviderVisibilityMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// 各模型供应商的看板显示偏好（非密钥）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderVisibility {
    pub cursor: ProviderVisibilityMode,
    pub codex: ProviderVisibilityMode,
    pub deepseek: ProviderVisibilityMode,
    pub grok: ProviderVisibilityMode,
}

impl Default for ProviderVisibility {
    fn default() -> Self {
        Self {
            cursor: ProviderVisibilityMode::Auto,
            codex: ProviderVisibilityMode::Auto,
            deepseek: ProviderVisibilityMode::Auto,
            grok: ProviderVisibilityMode::Auto,
        }
    }
}

/// 应用设置（启动自磁盘加载；`update_settings` 成功后持久化）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Cursor 用量自动刷新间隔（秒），默认 300，最小 60。
    pub cursor_refresh_sec: u64,
    /// CPU/GPU 刷新间隔（秒），默认 2，范围 1–10。
    pub cpu_gpu_refresh_sec: u64,
    /// 其余系统指标刷新间隔（秒）：内存/磁盘/VPN/Latency；默认 10，范围 5–60。
    pub system_refresh_sec: u64,
    /// 延迟探测目标 URL，默认 `https://cursor.com`。
    pub latency_target: String,
    /// 高延迟阈值（毫秒）；UI 标红（§6）使用此字段。
    pub high_latency_ms: u64,
    /// 各模型供应商看板显示模式；默认全 `auto`。
    pub provider_visibility: ProviderVisibility,
    /// 模型供应商看板顺序；仅含 cursor/codex/deepseek/grok。
    pub provider_order: Vec<String>,
    /// 是否显示 System 卡片；默认 true。
    pub show_system_section: bool,
    /// 是否显示 Latency 卡片；默认 true。
    pub show_latency_section: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cursor_refresh_sec: Self::DEFAULT_CURSOR_REFRESH_SEC,
            cpu_gpu_refresh_sec: Self::DEFAULT_CPU_GPU_REFRESH_SEC,
            system_refresh_sec: Self::DEFAULT_SYSTEM_REFRESH_SEC,
            latency_target: Self::DEFAULT_LATENCY_TARGET.to_string(),
            high_latency_ms: Self::DEFAULT_HIGH_LATENCY_MS,
            provider_visibility: ProviderVisibility::default(),
            provider_order: Self::default_provider_order(),
            show_system_section: true,
            show_latency_section: true,
        }
    }
}

impl AppSettings {
    pub const MIN_CURSOR_REFRESH_SEC: u64 = 60;
    pub const DEFAULT_CURSOR_REFRESH_SEC: u64 = 300;

    pub const MIN_CPU_GPU_REFRESH_SEC: u64 = 1;
    pub const MAX_CPU_GPU_REFRESH_SEC: u64 = 10;
    pub const DEFAULT_CPU_GPU_REFRESH_SEC: u64 = 2;

    pub const MIN_SYSTEM_REFRESH_SEC: u64 = 5;
    pub const MAX_SYSTEM_REFRESH_SEC: u64 = 60;
    pub const DEFAULT_SYSTEM_REFRESH_SEC: u64 = 10;

    pub const DEFAULT_LATENCY_TARGET: &'static str = "https://cursor.com";
    pub const DEFAULT_HIGH_LATENCY_MS: u64 = 500;
    pub const MIN_HIGH_LATENCY_MS: u64 = 1;

    pub const PROVIDER_IDS: [&'static str; 4] = ["cursor", "codex", "deepseek", "grok"];

    pub fn default_provider_order() -> Vec<String> {
        Self::PROVIDER_IDS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    pub fn clamp_cursor_refresh_sec(sec: u64) -> u64 {
        sec.max(Self::MIN_CURSOR_REFRESH_SEC)
    }

    pub fn clamp_cpu_gpu_refresh_sec(sec: u64) -> u64 {
        sec.clamp(
            Self::MIN_CPU_GPU_REFRESH_SEC,
            Self::MAX_CPU_GPU_REFRESH_SEC,
        )
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

    /// 非法 / 未知枚举回退为 `auto`。
    pub fn parse_visibility_mode(raw: &str) -> ProviderVisibilityMode {
        match raw.trim().to_ascii_lowercase().as_str() {
            "always" => ProviderVisibilityMode::Always,
            "hidden" => ProviderVisibilityMode::Hidden,
            "auto" => ProviderVisibilityMode::Auto,
            _ => ProviderVisibilityMode::Auto,
        }
    }

    pub fn normalize_provider_visibility(vis: ProviderVisibility) -> ProviderVisibility {
        // 已是枚举；经磁盘字符串解析后再走此路径时模式已合法。
        vis
    }

    /// 过滤未知 id、去重（首现优先）、按默认相对顺序补齐缺失项。
    pub fn normalize_provider_order(order: &[String]) -> Vec<String> {
        let mut result: Vec<String> = Vec::with_capacity(Self::PROVIDER_IDS.len());
        for id in order {
            let normalized = id.trim().to_ascii_lowercase();
            if Self::PROVIDER_IDS.contains(&normalized.as_str())
                && !result.iter().any(|x| x == &normalized)
            {
                result.push(normalized);
            }
        }
        for &id in &Self::PROVIDER_IDS {
            if !result.iter().any(|x| x == id) {
                result.push(id.to_string());
            }
        }
        result
    }

    pub fn mode_for_provider(&self, provider: &str) -> ProviderVisibilityMode {
        match provider {
            "cursor" => self.provider_visibility.cursor,
            "codex" => self.provider_visibility.codex,
            "deepseek" => self.provider_visibility.deepseek,
            "grok" => self.provider_visibility.grok,
            _ => ProviderVisibilityMode::Auto,
        }
    }
}

/// 设置部分更新。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsUpdate {
    pub cursor_refresh_sec: Option<u64>,
    pub cpu_gpu_refresh_sec: Option<u64>,
    pub system_refresh_sec: Option<u64>,
    pub latency_target: Option<String>,
    pub high_latency_ms: Option<u64>,
    pub provider_visibility: Option<ProviderVisibility>,
    pub provider_order: Option<Vec<String>>,
    pub show_system_section: Option<bool>,
    pub show_latency_section: Option<bool>,
}
