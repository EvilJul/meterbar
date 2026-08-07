//! 非敏感 AppSettings 的本机 JSON 持久化。
//! 与 credentials 同目录惯例，但独立文件；绝不写入密钥字段。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::{
    AppSettings, AppSettingsUpdate, ProviderVisibility, ProviderVisibilityMode,
};

const SETTINGS_FILE: &str = "settings.json";
const SCHEMA_VERSION: u32 = 1;

/// 磁盘上的 visibility 原始对象（字符串，便于非法枚举回退）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderVisibilityFile {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    codex: Option<String>,
    #[serde(default)]
    deepseek: Option<String>,
    #[serde(default)]
    grok: Option<String>,
}

impl ProviderVisibilityFile {
    fn from_visibility(vis: &ProviderVisibility) -> Self {
        Self {
            cursor: Some(mode_to_str(vis.cursor).to_string()),
            codex: Some(mode_to_str(vis.codex).to_string()),
            deepseek: Some(mode_to_str(vis.deepseek).to_string()),
            grok: Some(mode_to_str(vis.grok).to_string()),
        }
    }

    fn into_visibility(self) -> ProviderVisibility {
        ProviderVisibility {
            cursor: self
                .cursor
                .as_deref()
                .map(AppSettings::parse_visibility_mode)
                .unwrap_or(ProviderVisibilityMode::Auto),
            codex: self
                .codex
                .as_deref()
                .map(AppSettings::parse_visibility_mode)
                .unwrap_or(ProviderVisibilityMode::Auto),
            deepseek: self
                .deepseek
                .as_deref()
                .map(AppSettings::parse_visibility_mode)
                .unwrap_or(ProviderVisibilityMode::Auto),
            grok: self
                .grok
                .as_deref()
                .map(AppSettings::parse_visibility_mode)
                .unwrap_or(ProviderVisibilityMode::Auto),
        }
    }
}

fn mode_to_str(mode: ProviderVisibilityMode) -> &'static str {
    match mode {
        ProviderVisibilityMode::Auto => "auto",
        ProviderVisibilityMode::Always => "always",
        ProviderVisibilityMode::Hidden => "hidden",
    }
}

/// 磁盘 schema：可选 version + 既有四字段 + 显示/排序偏好；禁止密钥字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    cursor_refresh_sec: Option<u64>,
    #[serde(default)]
    cpu_gpu_refresh_sec: Option<u64>,
    #[serde(default)]
    system_refresh_sec: Option<u64>,
    #[serde(default)]
    latency_target: Option<String>,
    #[serde(default)]
    high_latency_ms: Option<u64>,
    #[serde(default)]
    provider_visibility: Option<ProviderVisibilityFile>,
    #[serde(default)]
    provider_order: Option<Vec<String>>,
    #[serde(default)]
    show_system_section: Option<bool>,
    #[serde(default)]
    show_latency_section: Option<bool>,
}

fn default_version() -> u32 {
    SCHEMA_VERSION
}

impl SettingsFile {
    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            version: SCHEMA_VERSION,
            cursor_refresh_sec: Some(settings.cursor_refresh_sec),
            cpu_gpu_refresh_sec: Some(settings.cpu_gpu_refresh_sec),
            system_refresh_sec: Some(settings.system_refresh_sec),
            latency_target: Some(settings.latency_target.clone()),
            high_latency_ms: Some(settings.high_latency_ms),
            provider_visibility: Some(ProviderVisibilityFile::from_visibility(
                &settings.provider_visibility,
            )),
            provider_order: Some(settings.provider_order.clone()),
            show_system_section: Some(settings.show_system_section),
            show_latency_section: Some(settings.show_latency_section),
        }
    }

    fn into_app_settings(self) -> AppSettings {
        let defaults = AppSettings::default();
        // 旧 settings.json 仅有 showSystemSection 时：两者同值；均缺省则为 true。
        let legacy_system = self.show_system_section;
        let show_system_section = legacy_system.unwrap_or(defaults.show_system_section);
        let show_latency_section = self
            .show_latency_section
            .unwrap_or_else(|| legacy_system.unwrap_or(defaults.show_latency_section));

        // 旧文件仅有 systemRefreshSec（单一全量间隔）：迁到 CPU/GPU，其余用新默认。
        let legacy_single_interval =
            self.cpu_gpu_refresh_sec.is_none() && self.system_refresh_sec.is_some();
        let cpu_gpu_refresh_sec = AppSettings::clamp_cpu_gpu_refresh_sec(
            self.cpu_gpu_refresh_sec.unwrap_or_else(|| {
                if legacy_single_interval {
                    self.system_refresh_sec.unwrap_or(defaults.cpu_gpu_refresh_sec)
                } else {
                    defaults.cpu_gpu_refresh_sec
                }
            }),
        );
        let system_refresh_sec = if legacy_single_interval {
            defaults.system_refresh_sec
        } else {
            AppSettings::clamp_system_refresh_sec(
                self.system_refresh_sec
                    .unwrap_or(defaults.system_refresh_sec),
            )
        };

        AppSettings {
            cursor_refresh_sec: AppSettings::clamp_cursor_refresh_sec(
                self.cursor_refresh_sec
                    .unwrap_or(defaults.cursor_refresh_sec),
            ),
            cpu_gpu_refresh_sec,
            system_refresh_sec,
            latency_target: AppSettings::normalize_latency_target(
                self.latency_target
                    .as_deref()
                    .unwrap_or(&defaults.latency_target),
            ),
            high_latency_ms: AppSettings::clamp_high_latency_ms(
                self.high_latency_ms.unwrap_or(defaults.high_latency_ms),
            ),
            provider_visibility: self
                .provider_visibility
                .unwrap_or_default()
                .into_visibility(),
            provider_order: AppSettings::normalize_provider_order(
                self.provider_order
                    .as_deref()
                    .unwrap_or(&defaults.provider_order),
            ),
            show_system_section,
            show_latency_section,
        }
    }
}

/// 解析设置文件路径：`USAGES_SETTINGS_PATH` > 平台 app 数据目录 / `settings.json`
/// （`USAGES_CREDENTIALS_DIR` 由 `platform_paths::app_data_dir` 优先处理）。
pub fn settings_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("USAGES_SETTINGS_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(crate::platform_paths::app_data_dir()?.join(SETTINGS_FILE))
}

/// 启动加载：缺文件 / 坏 JSON → 默认值；成功则反序列化并 clamp/normalize。
/// 缺文件时不写盘（懒写入，首次 `update_settings` 再写）。
pub fn load() -> AppSettings {
    let path = match settings_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[usages] 解析设置路径失败，使用默认设置: {e}");
            return AppSettings::default();
        }
    };

    if !path.exists() {
        return AppSettings::default();
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[usages] 读取设置文件失败，使用默认设置: {e}");
            return AppSettings::default();
        }
    };

    match serde_json::from_str::<SettingsFile>(&content) {
        Ok(file) => file.into_app_settings(),
        Err(e) => {
            eprintln!("[usages] 设置文件无效，使用默认设置: {e}");
            AppSettings::default()
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("设置权限失败: {e}"))
}

/// 原子写盘：目录 `0700`、文件 `0600`、tmp+rename。
pub fn save(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建设置目录失败: {e}"))?;
        #[cfg(unix)]
        set_mode(parent, 0o700)?;
    }

    let file = SettingsFile::from_settings(settings);
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("序列化设置失败: {e}"))?;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json.as_bytes()).map_err(|e| format!("写入设置临时文件失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, &path).map_err(|e| format!("提交设置文件失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&path, 0o600)?;
    Ok(())
}

/// 将 patch 钳位后写入内存并持久化；写盘失败则严格回滚内存。
pub fn apply_update(
    current: &mut AppSettings,
    patch: AppSettingsUpdate,
) -> Result<AppSettings, String> {
    let snapshot = current.clone();

    if let Some(sec) = patch.cursor_refresh_sec {
        current.cursor_refresh_sec = AppSettings::clamp_cursor_refresh_sec(sec);
    }
    if let Some(sec) = patch.cpu_gpu_refresh_sec {
        current.cpu_gpu_refresh_sec = AppSettings::clamp_cpu_gpu_refresh_sec(sec);
    }
    if let Some(sec) = patch.system_refresh_sec {
        current.system_refresh_sec = AppSettings::clamp_system_refresh_sec(sec);
    }
    if let Some(target) = patch.latency_target {
        current.latency_target = AppSettings::normalize_latency_target(&target);
    }
    if let Some(ms) = patch.high_latency_ms {
        current.high_latency_ms = AppSettings::clamp_high_latency_ms(ms);
    }
    if let Some(vis) = patch.provider_visibility {
        current.provider_visibility = AppSettings::normalize_provider_visibility(vis);
    }
    if let Some(order) = patch.provider_order {
        current.provider_order = AppSettings::normalize_provider_order(&order);
    }
    if let Some(show) = patch.show_system_section {
        current.show_system_section = show;
    }
    if let Some(show) = patch.show_latency_section {
        current.show_latency_section = show;
    }

    match save(current) {
        Ok(()) => Ok(current.clone()),
        Err(e) => {
            *current = snapshot;
            Err(format!("保存设置到磁盘失败: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_settings_path<F>(f: F)
    where
        F: FnOnce(&std::path::Path),
    {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(SETTINGS_FILE);
        std::env::set_var("USAGES_SETTINGS_PATH", &path);
        std::env::remove_var("USAGES_CREDENTIALS_DIR");

        f(&path);

        std::env::remove_var("USAGES_SETTINGS_PATH");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_file_returns_defaults_without_writing() {
        with_temp_settings_path(|path| {
            assert!(!path.exists());
            let loaded = load();
            assert_eq!(loaded.cursor_refresh_sec, AppSettings::DEFAULT_CURSOR_REFRESH_SEC);
            assert_eq!(
                loaded.cpu_gpu_refresh_sec,
                AppSettings::DEFAULT_CPU_GPU_REFRESH_SEC
            );
            assert_eq!(loaded.system_refresh_sec, AppSettings::DEFAULT_SYSTEM_REFRESH_SEC);
            assert_eq!(loaded.latency_target, AppSettings::DEFAULT_LATENCY_TARGET);
            assert_eq!(loaded.high_latency_ms, AppSettings::DEFAULT_HIGH_LATENCY_MS);
            assert_eq!(loaded.provider_visibility, ProviderVisibility::default());
            assert_eq!(loaded.provider_order, AppSettings::default_provider_order());
            assert!(loaded.show_system_section);
            assert!(loaded.show_latency_section);
            assert!(!path.exists(), "缺文件时不应写盘");
        });
    }

    #[test]
    fn load_bad_json_returns_defaults() {
        with_temp_settings_path(|path| {
            fs::write(path, "{not valid json").expect("write bad json");
            let loaded = load();
            assert_eq!(loaded, AppSettings::default());
        });
    }

    #[test]
    fn load_clamps_out_of_range_values() {
        with_temp_settings_path(|path| {
            fs::write(
                path,
                r#"{
                  "version": 1,
                  "cursorRefreshSec": 10,
                  "cpuGpuRefreshSec": 999,
                  "systemRefreshSec": 999,
                  "latencyTarget": "  ",
                  "highLatencyMs": 0
                }"#,
            )
            .expect("write");
            let loaded = load();
            assert_eq!(loaded.cursor_refresh_sec, 60);
            assert_eq!(loaded.cpu_gpu_refresh_sec, 10);
            assert_eq!(loaded.system_refresh_sec, 60);
            assert_eq!(loaded.latency_target, AppSettings::DEFAULT_LATENCY_TARGET);
            assert_eq!(loaded.high_latency_ms, 1);
            // 旧四字段文件：新字段用默认
            assert_eq!(loaded.provider_visibility, ProviderVisibility::default());
            assert_eq!(loaded.provider_order, AppSettings::default_provider_order());
            assert!(loaded.show_system_section);
            assert!(loaded.show_latency_section);
        });
    }

    #[test]
    fn load_migrates_legacy_single_system_interval_to_cpu_gpu() {
        with_temp_settings_path(|path| {
            fs::write(
                path,
                r#"{
                  "version": 1,
                  "systemRefreshSec": 3
                }"#,
            )
            .expect("write");
            let loaded = load();
            assert_eq!(loaded.cpu_gpu_refresh_sec, 3);
            assert_eq!(
                loaded.system_refresh_sec,
                AppSettings::DEFAULT_SYSTEM_REFRESH_SEC
            );
        });
    }

    #[test]
    fn load_normalizes_invalid_visibility_and_order() {
        with_temp_settings_path(|path| {
            fs::write(
                path,
                r#"{
                  "providerVisibility": {
                    "cursor": "always",
                    "codex": "nope",
                    "deepseek": "hidden"
                  },
                  "providerOrder": ["codex", "codex", "acme", "cursor"],
                  "showSystemSection": false
                }"#,
            )
            .expect("write");
            let loaded = load();
            assert_eq!(loaded.provider_visibility.cursor, ProviderVisibilityMode::Always);
            assert_eq!(loaded.provider_visibility.codex, ProviderVisibilityMode::Auto);
            assert_eq!(
                loaded.provider_visibility.deepseek,
                ProviderVisibilityMode::Hidden
            );
            assert_eq!(
                loaded.provider_order,
                vec![
                    "codex".to_string(),
                    "cursor".to_string(),
                    "deepseek".to_string(),
                    "grok".to_string(),
                ]
            );
            assert_eq!(
                loaded.provider_visibility.grok,
                ProviderVisibilityMode::Auto
            );
            // 旧字段：System / Latency 同值迁移
            assert!(!loaded.show_system_section);
            assert!(!loaded.show_latency_section);
        });
    }

    #[test]
    fn load_migrates_legacy_show_system_section_to_both() {
        with_temp_settings_path(|path| {
            fs::write(path, r#"{"showSystemSection": false}"#).expect("write");
            let loaded = load();
            assert!(!loaded.show_system_section);
            assert!(!loaded.show_latency_section);
        });
    }

    #[test]
    fn load_independent_system_and_latency_visibility() {
        with_temp_settings_path(|path| {
            fs::write(
                path,
                r#"{
                  "showSystemSection": true,
                  "showLatencySection": false
                }"#,
            )
            .expect("write");
            let loaded = load();
            assert!(loaded.show_system_section);
            assert!(!loaded.show_latency_section);
        });
    }

    #[test]
    fn normalize_provider_order_unit_cases() {
        assert_eq!(
            AppSettings::normalize_provider_order(&[
                "cursor".into(),
                "acme".into(),
                "deepseek".into()
            ]),
            vec!["cursor", "deepseek", "codex", "grok"]
        );
        assert_eq!(
            AppSettings::normalize_provider_order(&["codex".into(), "codex".into(), "cursor".into()]),
            vec!["codex", "cursor", "deepseek", "grok"]
        );
        assert_eq!(
            AppSettings::normalize_provider_order(&["deepseek".into()]),
            vec!["deepseek", "cursor", "codex", "grok"]
        );
        assert_eq!(
            AppSettings::normalize_provider_order(&[
                "cursor".into(),
                "codex".into(),
                "deepseek".into()
            ]),
            vec!["cursor", "codex", "deepseek", "grok"]
        );
    }

    #[test]
    fn save_load_roundtrip() {
        with_temp_settings_path(|path| {
            let original = AppSettings {
                cursor_refresh_sec: 120,
                cpu_gpu_refresh_sec: 3,
                system_refresh_sec: 15,
                latency_target: "https://example.com".to_string(),
                high_latency_ms: 800,
                provider_visibility: ProviderVisibility {
                    cursor: ProviderVisibilityMode::Hidden,
                    codex: ProviderVisibilityMode::Always,
                    deepseek: ProviderVisibilityMode::Auto,
                    grok: ProviderVisibilityMode::Always,
                },
                provider_order: vec![
                    "deepseek".into(),
                    "cursor".into(),
                    "codex".into(),
                    "grok".into(),
                ],
                show_system_section: false,
                show_latency_section: true,
            };
            save(&original).expect("save");
            assert!(path.exists());
            let loaded = load();
            assert_eq!(loaded.cursor_refresh_sec, 120);
            assert_eq!(loaded.cpu_gpu_refresh_sec, 3);
            assert_eq!(loaded.system_refresh_sec, 15);
            assert_eq!(loaded.latency_target, "https://example.com");
            assert_eq!(loaded.high_latency_ms, 800);
            assert_eq!(loaded.provider_visibility, original.provider_visibility);
            assert_eq!(loaded.provider_order, original.provider_order);
            assert!(!loaded.show_system_section);
            assert!(loaded.show_latency_section);

            let raw = fs::read_to_string(path).expect("read raw");
            assert!(raw.contains("cursorRefreshSec"));
            assert!(raw.contains("cpuGpuRefreshSec"));
            assert!(raw.contains("systemRefreshSec"));
            assert!(raw.contains("providerVisibility"));
            assert!(raw.contains("providerOrder"));
            assert!(raw.contains("showSystemSection"));
            assert!(raw.contains("showLatencySection"));
            assert!(!raw.to_lowercase().contains("token"));
            assert!(!raw.to_lowercase().contains("api_key"));
            assert!(!raw.to_lowercase().contains("apikey"));
            assert!(!raw.to_lowercase().contains("cookie"));
        });
    }

    #[test]
    fn apply_update_rolls_back_memory_on_save_failure() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-settings-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        // 用普通文件占位「目录」，使 create_dir_all / 写入失败。
        let blocker = dir.join("not-a-directory");
        fs::write(&blocker, b"x").expect("blocker");
        let bad_path = blocker.join(SETTINGS_FILE);
        std::env::set_var("USAGES_SETTINGS_PATH", &bad_path);
        std::env::remove_var("USAGES_CREDENTIALS_DIR");

        let mut current = AppSettings::default();
        let before = current.clone();
        let err = apply_update(
            &mut current,
            AppSettingsUpdate {
                cursor_refresh_sec: Some(180),
                cpu_gpu_refresh_sec: Some(4),
                system_refresh_sec: Some(20),
                latency_target: Some("https://rolled-back.example".into()),
                high_latency_ms: Some(900),
                ..Default::default()
            },
        )
        .expect_err("save should fail");

        assert!(err.contains("保存设置到磁盘失败"));
        assert_eq!(current, before);

        std::env::remove_var("USAGES_SETTINGS_PATH");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_path_prefers_usages_settings_path() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let custom = "/tmp/custom-usages-settings.json";
        std::env::set_var("USAGES_SETTINGS_PATH", custom);
        std::env::set_var("USAGES_CREDENTIALS_DIR", "/tmp/cred-dir-should-not-win");
        let path = settings_path().expect("path");
        assert_eq!(path, PathBuf::from(custom));
        std::env::remove_var("USAGES_SETTINGS_PATH");
        std::env::remove_var("USAGES_CREDENTIALS_DIR");
    }

    #[test]
    fn normalize_latency_target_adds_https_scheme() {
        with_temp_settings_path(|path| {
            fs::write(
                path,
                r#"{"latencyTarget":"cursor.com"}"#,
            )
            .expect("write");
            let loaded = load();
            assert_eq!(loaded.latency_target, "https://cursor.com");
        });
    }
}
