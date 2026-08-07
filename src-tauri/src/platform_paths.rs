//! 平台相关路径：App 数据目录、Cursor state.vscdb 候选。
//! 环境变量 `USAGES_CREDENTIALS_DIR` / `USAGES_CURSOR_STATE_DB` 优先于默认解析。

use std::path::{Path, PathBuf};

/// 与凭证 / settings 共用的 service 目录名（历史 identifier）。
pub const APP_SERVICE_DIR: &str = "com.usages.app";

/// 解析应用数据目录（凭证 fallback、默认 settings 父目录）。
///
/// 优先级：`USAGES_CREDENTIALS_DIR` → 平台默认（macOS Application Support /
/// Linux XDG config）。
pub fn app_data_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("USAGES_CREDENTIALS_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    default_app_data_dir()
}

/// 不含环境变量覆盖的平台默认 App 数据目录（可测）。
pub fn default_app_data_dir_for_home(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support").join(APP_SERVICE_DIR)
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let trimmed = xdg.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_SERVICE_DIR);
            }
        }
        home.join(".config").join(APP_SERVICE_DIR)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        home.join(".config").join(APP_SERVICE_DIR)
    }
}

fn default_app_data_dir() -> Result<PathBuf, String> {
    let home = crate::credentials::local_session::primary_home_dir()
        .ok_or_else(|| "无法定位用户主目录".to_string())?;
    Ok(default_app_data_dir_for_home(&home))
}

/// 某 home 下 Cursor `state.vscdb`（含 Insiders / backup）候选路径。
pub fn candidate_cursor_state_db_paths(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app_support = home.join("Library/Application Support");
        expand_cursor_bases(&[
            app_support.join("Cursor/User/globalStorage"),
            app_support.join("Cursor - Insiders/User/globalStorage"),
        ])
    }
    #[cfg(target_os = "linux")]
    {
        let config = linux_config_home(home);
        expand_cursor_bases(&[
            config.join("Cursor/User/globalStorage"),
            config.join("Cursor - Insiders/User/globalStorage"),
        ])
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = home;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn linux_config_home(home: &Path) -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    home.join(".config")
}

fn expand_cursor_bases(bases: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(bases.len() * 2);
    for base in bases {
        paths.push(base.join("state.vscdb"));
        paths.push(base.join("state.vscdb.backup"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn app_data_dir_prefers_usages_credentials_dir() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "meterbar-app-data-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("USAGES_CREDENTIALS_DIR", &dir);
        let got = app_data_dir().expect("path");
        assert_eq!(got, dir);
        std::env::remove_var("USAGES_CREDENTIALS_DIR");
    }

    #[test]
    fn default_app_data_dir_layout_matches_platform() {
        let home = PathBuf::from("/home/tester");
        #[cfg(target_os = "macos")]
        {
            let path = default_app_data_dir_for_home(&home);
            assert_eq!(
                path,
                PathBuf::from("/home/tester/Library/Application Support/com.usages.app")
            );
        }
        #[cfg(target_os = "linux")]
        {
            let _guard = ENV_LOCK.lock().expect("env lock");
            std::env::remove_var("XDG_CONFIG_HOME");
            let path = default_app_data_dir_for_home(&home);
            assert_eq!(
                path,
                PathBuf::from("/home/tester/.config/com.usages.app")
            );
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-test");
            let path_xdg = default_app_data_dir_for_home(&home);
            assert_eq!(
                path_xdg,
                PathBuf::from("/tmp/xdg-config-test/com.usages.app")
            );
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn cursor_state_db_candidates_include_platform_paths() {
        let home = PathBuf::from("/home/tester");
        #[cfg(target_os = "macos")]
        {
            let paths = candidate_cursor_state_db_paths(&home);
            assert!(paths.iter().any(|p| p.ends_with(
                "Library/Application Support/Cursor/User/globalStorage/state.vscdb"
            )));
            assert!(paths.iter().any(|p| p.ends_with("state.vscdb.backup")));
        }
        #[cfg(target_os = "linux")]
        {
            let _guard = ENV_LOCK.lock().expect("env lock");
            std::env::remove_var("XDG_CONFIG_HOME");
            let paths = candidate_cursor_state_db_paths(&home);
            assert!(paths.iter().any(|p| {
                p == &PathBuf::from(
                    "/home/tester/.config/Cursor/User/globalStorage/state.vscdb",
                )
            }));
            assert!(paths.iter().any(|p| p.ends_with(
                "Cursor - Insiders/User/globalStorage/state.vscdb"
            )));
        }
    }
}
