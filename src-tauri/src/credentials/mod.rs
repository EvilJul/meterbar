//! 本机凭证存取：Keychain 优先，并双写本地 fallback（0600）。
//! 绝不把 token 明文写入日志。

pub mod grok_session;
pub mod local_session;

use std::fs;
use std::path::PathBuf;

use keyring::Entry;

const SERVICE: &str = "com.usages.app";
const ACCOUNT: &str = "WorkosCursorSessionToken";
const FALLBACK_FILE: &str = "cursor_session_token";
const COOKIE_PREFIX: &str = "WorkosCursorSessionToken=";

const DEEPSEEK_ACCOUNT: &str = "DeepSeekApiKey";
const DEEPSEEK_FALLBACK_FILE: &str = "deepseek_api_key";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("无法访问钥匙串: {e}"))
}

fn deepseek_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, DEEPSEEK_ACCOUNT).map_err(|e| format!("无法访问钥匙串: {e}"))
}

fn fallback_dir() -> Result<PathBuf, String> {
    crate::platform_paths::app_data_dir()
}

fn fallback_path() -> Result<PathBuf, String> {
    Ok(fallback_dir()?.join(FALLBACK_FILE))
}

fn deepseek_fallback_path() -> Result<PathBuf, String> {
    Ok(fallback_dir()?.join(DEEPSEEK_FALLBACK_FILE))
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("设置权限失败: {e}"))
}

fn set_fallback(token: &str) -> Result<(), String> {
    let path = fallback_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建凭证目录失败: {e}"))?;
        #[cfg(unix)]
        set_mode(parent, 0o700)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, token).map_err(|e| format!("写入本地凭证失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, &path).map_err(|e| format!("提交本地凭证失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&path, 0o600)?;
    Ok(())
}

fn get_fallback() -> Result<Option<String>, String> {
    let path = fallback_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path).map_err(|e| format!("读取本地凭证失败: {e}"))?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.trim().to_string()))
    }
}

fn clear_fallback() -> Result<(), String> {
    let path = fallback_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清除本地凭证失败: {e}")),
    }
}

fn set_keychain(token: &str) -> Result<(), String> {
    entry()?
        .set_password(token)
        .map_err(|e| format!("写入钥匙串失败: {e}"))
}

fn get_keychain() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(value) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.trim().to_string()))
            }
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取钥匙串失败: {e}")),
    }
}

fn clear_keychain() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("清除钥匙串失败: {e}")),
    }
}

/// 规范化粘贴内容：去空白/包裹引号，剥离 `WorkosCursorSessionToken=` 前缀。
pub(crate) fn normalize_token(raw: &str) -> Result<String, String> {
    let mut s = raw.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[s.len() - 1] == quote {
            s = s[1..s.len() - 1].trim();
        }
    }
    if let Some(rest) = s.strip_prefix(COOKIE_PREFIX) {
        s = rest.trim();
    }
    if s.is_empty() {
        return Err("会话 token 不能为空".to_string());
    }
    Ok(s.to_string())
}

/// 双写逻辑（可注入），保证钥匙串 set-ok/get-none 时仍保留 fallback 副本。
///
/// - 始终写入 fallback（成功路径绝不 `clear_fallback`）
/// - 钥匙串尽力写入；回读优先钥匙串，空/失败则 fallback
/// - 末尾校验回读非空且与写入一致（trim 后）
fn persist_dual_write(
    normalized: &str,
    set_kc: impl FnOnce(&str) -> Result<(), String>,
    get_kc: impl FnOnce() -> Result<Option<String>, String>,
    set_fb: impl FnOnce(&str) -> Result<(), String>,
    get_combined: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<(), String> {
    let kc_write_err = set_kc(normalized).err();

    // 关键：无论钥匙串是否「成功」，都必须写入 fallback；成功路径不得清除 fallback。
    if let Err(fb_err) = set_fb(normalized) {
        // 若钥匙串已可回读，仍可视为可用；否则失败。
        match get_kc() {
            Ok(Some(v)) if v.trim() == normalized => {
                // 钥匙串可用，本地备份失败只作为警告性错误信息的一部分——此处仍成功。
                let _ = kc_write_err;
                let _ = fb_err;
                return Ok(());
            }
            Ok(Some(_)) | Ok(None) => {
                return Err(match kc_write_err {
                    Some(kc) => format!("钥匙串写入失败且本地备份失败: {kc}; {fb_err}"),
                    None => format!("钥匙串回读为空，已尝试本地备份失败: {fb_err}"),
                });
            }
            Err(kc_err) => {
                return Err(format!("钥匙串不可用且本地备份失败: {kc_err}; {fb_err}"));
            }
        }
    }

    match get_combined() {
        Ok(Some(stored)) if stored.trim() == normalized => Ok(()),
        Ok(Some(_)) => Err("保存后回读内容与写入不一致".to_string()),
        Ok(None) => Err(match kc_write_err {
            Some(kc) => format!("钥匙串写入失败，本地备份回读亦为空: {kc}"),
            None => "钥匙串回读为空，本地备份回读亦为空".to_string(),
        }),
        Err(e) => Err(format!("保存后回读失败: {e}")),
    }
}

/// 将会话 token 双写 Keychain + 本地受保护文件。不记录 token 内容。
pub fn set_token(token: &str) -> Result<(), String> {
    let normalized = normalize_token(token)?;
    persist_dual_write(
        &normalized,
        set_keychain,
        get_keychain,
        set_fallback,
        || get_token_raw(),
    )
}

fn get_token_raw() -> Result<Option<String>, String> {
    match get_keychain() {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => get_fallback(),
        Err(keychain_err) => match get_fallback() {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Err(keychain_err),
            Err(_) => Err(keychain_err),
        },
    }
}

/// 读取会话 token；Keychain 优先，其次本地 fallback；无条目时返回 `None`。
pub fn get_token() -> Result<Option<String>, String> {
    get_token_raw()
}

/// 本机 Cursor 登录态或已保存 Cookie 任一可用。
pub fn has_any_auth() -> bool {
    local_session::has_local_session()
        || get_token()
            .ok()
            .flatten()
            .is_some_and(|t| !t.trim().is_empty())
}

/// 清除已存 token（Keychain + fallback）；无条目视为成功。
pub fn clear_token() -> Result<(), String> {
    let kc = clear_keychain();
    let fb = clear_fallback();
    match (kc, fb) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

fn set_deepseek_fallback(key: &str) -> Result<(), String> {
    let path = deepseek_fallback_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建凭证目录失败: {e}"))?;
        #[cfg(unix)]
        set_mode(parent, 0o700)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, key).map_err(|e| format!("写入本地凭证失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, &path).map_err(|e| format!("提交本地凭证失败: {e}"))?;
    #[cfg(unix)]
    set_mode(&path, 0o600)?;
    Ok(())
}

fn get_deepseek_fallback() -> Result<Option<String>, String> {
    let path = deepseek_fallback_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path).map_err(|e| format!("读取本地凭证失败: {e}"))?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.trim().to_string()))
    }
}

fn clear_deepseek_fallback() -> Result<(), String> {
    let path = deepseek_fallback_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清除本地凭证失败: {e}")),
    }
}

fn set_deepseek_keychain(key: &str) -> Result<(), String> {
    deepseek_entry()?
        .set_password(key)
        .map_err(|e| format!("写入钥匙串失败: {e}"))
}

fn get_deepseek_keychain() -> Result<Option<String>, String> {
    match deepseek_entry()?.get_password() {
        Ok(value) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.trim().to_string()))
            }
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取钥匙串失败: {e}")),
    }
}

fn clear_deepseek_keychain() -> Result<(), String> {
    match deepseek_entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("清除钥匙串失败: {e}")),
    }
}

fn get_deepseek_key_raw() -> Result<Option<String>, String> {
    match get_deepseek_keychain() {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => get_deepseek_fallback(),
        Err(keychain_err) => match get_deepseek_fallback() {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Err(keychain_err),
            Err(_) => Err(keychain_err),
        },
    }
}

fn normalize_api_key(raw: &str) -> Result<String, String> {
    let mut s = raw.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[s.len() - 1] == quote {
            s = s[1..s.len() - 1].trim();
        }
    }
    if let Some(rest) = s.strip_prefix("Bearer ") {
        s = rest.trim();
    }
    if s.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    Ok(s.to_string())
}

/// 将 DeepSeek API Key 双写 Keychain + 本地受保护文件。不记录 key 内容。
pub fn set_deepseek_key(key: &str) -> Result<(), String> {
    let normalized = normalize_api_key(key)?;
    persist_dual_write(
        &normalized,
        set_deepseek_keychain,
        get_deepseek_keychain,
        set_deepseek_fallback,
        || get_deepseek_key_raw(),
    )
}

/// 读取 DeepSeek API Key；无条目时返回 `None`。
pub fn get_deepseek_key() -> Result<Option<String>, String> {
    get_deepseek_key_raw()
}

/// 是否已配置 DeepSeek API Key。
pub fn has_deepseek_key() -> bool {
    get_deepseek_key()
        .ok()
        .flatten()
        .is_some_and(|k| !k.trim().is_empty())
}

/// 清除 DeepSeek API Key。
pub fn clear_deepseek_key() -> Result<(), String> {
    let kc = clear_deepseek_keychain();
    let fb = clear_deepseek_fallback();
    match (kc, fb) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clear_fallback, fallback_path, get_fallback, normalize_token, persist_dual_write,
        set_fallback,
    };
    use std::cell::RefCell;
    use std::sync::Mutex;

    // 串行化环境变量测试，避免并行污染。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn normalize_strips_prefix_quotes_and_whitespace() {
        assert_eq!(
            normalize_token("  WorkosCursorSessionToken=abc.def  ").unwrap(),
            "abc.def"
        );
        assert_eq!(
            normalize_token("\"WorkosCursorSessionToken=xyz\"").unwrap(),
            "xyz"
        );
        assert_eq!(normalize_token("'plain-token'").unwrap(), "plain-token");
        assert_eq!(normalize_token("  plain  ").unwrap(), "plain");
        assert!(normalize_token("   ").is_err());
        assert!(normalize_token("WorkosCursorSessionToken=").is_err());
    }

    #[test]
    fn fallback_roundtrip_in_temp_dir() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-cred-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("USAGES_CREDENTIALS_DIR", &dir);

        let token = "test-token-value-not-for-log";
        set_fallback(token).expect("write fallback");
        let got = get_fallback().expect("read fallback");
        assert_eq!(got.as_deref(), Some(token));
        let path = fallback_path().expect("path");
        assert!(path.starts_with(&dir));
        clear_fallback().expect("clear fallback");
        assert!(get_fallback().expect("read after clear").is_none());

        std::env::remove_var("USAGES_CREDENTIALS_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keychain_set_ok_get_none_still_reads_fallback() {
        let fb = RefCell::new(None::<String>);
        let token = "survives-when-keychain-lies";

        persist_dual_write(
            token,
            |_t| Ok(()), // 模拟钥匙串 set 成功
            || Ok(None),  // 模拟随后 get 为 NoEntry/空
            |t| {
                *fb.borrow_mut() = Some(t.to_string());
                Ok(())
            },
            || {
                // 与 get_token_raw 一致：钥匙串空则读 fallback
                Ok(fb.borrow().clone())
            },
        )
        .expect("dual-write should succeed via fallback");

        assert_eq!(fb.borrow().as_deref(), Some(token));
    }

    #[test]
    fn never_clears_fallback_on_keychain_phantom_success() {
        // 若旧逻辑在 set_kc Ok 后 clear_fallback，此用例会失败。
        let fb = RefCell::new(Some("stale".to_string()));
        let token = "fresh-token";

        persist_dual_write(
            token,
            |_t| Ok(()),
            || Ok(None),
            |t| {
                *fb.borrow_mut() = Some(t.to_string());
                Ok(())
            },
            || Ok(fb.borrow().clone()),
        )
        .expect("persist");

        assert_eq!(fb.borrow().as_deref(), Some(token));
    }

    #[test]
    fn both_stores_fail_returns_specific_error_without_token() {
        let token = "secret-must-not-appear-in-error";
        let err = persist_dual_write(
            token,
            |_t| Err("kc boom".into()),
            || Ok(None),
            |_t| Err("fb boom".into()),
            || Ok(None),
        )
        .expect_err("should fail");

        assert!(err.contains("本地备份失败"));
        assert!(!err.contains(token));
    }
}
