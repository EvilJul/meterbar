//! 本机 Grok Build / `grok login` 会话发现（`~/.grok/auth.json`）。
//! 绝不日志输出 token 明文。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// 账单请求所需材料（仅内存传递）。
#[derive(Debug, Clone)]
pub struct GrokAuth {
    pub access_token: String,
    pub user_id: String,
}

/// 候选 grok 主目录：`USAGES_GROK_HOME` → `GROK_HOME` → `~/.grok`。
pub fn grok_home_candidates() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.as_os_str().is_empty() && !homes.iter().any(|h| h == &p) {
            homes.push(p);
        }
    };
    if let Ok(p) = std::env::var("USAGES_GROK_HOME") {
        let t = p.trim();
        if !t.is_empty() {
            push(PathBuf::from(t));
        }
    }
    if let Ok(p) = std::env::var("GROK_HOME") {
        let t = p.trim();
        if !t.is_empty() {
            push(PathBuf::from(t));
        }
    }
    if let Some(home) = crate::credentials::local_session::primary_home_dir() {
        push(home.join(".grok"));
    }
    homes
}

/// 候选 `auth.json` 路径；`USAGES_GROK_AUTH_PATH` 优先。
pub fn auth_json_candidates() -> Vec<PathBuf> {
    if let Ok(p) = std::env::var("USAGES_GROK_AUTH_PATH") {
        let t = p.trim();
        if !t.is_empty() {
            return vec![PathBuf::from(t)];
        }
    }
    grok_home_candidates()
        .into_iter()
        .map(|h| h.join("auth.json"))
        .collect()
}

/// 是否存在可用 Grok 登录态（不验证 token 是否仍有效）。
pub fn has_local_session() -> bool {
    read_auth().is_some()
}

/// 读取首个可用 OAuth 范围（含 access token + user_id）。
pub fn read_auth() -> Option<GrokAuth> {
    for path in auth_json_candidates() {
        if let Some(auth) = read_auth_from_path(&path) {
            return Some(auth);
        }
    }
    None
}

fn read_auth_from_path(path: &Path) -> Option<GrokAuth> {
    if !path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    parse_auth_json(&raw)
}

/// 从 auth.json 全文解析 GrokAuth（可测）。
pub fn parse_auth_json(raw: &str) -> Option<GrokAuth> {
    let root: Value = serde_json::from_str(raw).ok()?;
    let obj = root.as_object()?;

    // 优先 oauth 范围（key 通常含 auth.x.ai）；跳过纯 api_key 范围若无 user_id。
    let mut best: Option<GrokAuth> = None;
    for (scope, value) in obj {
        let Some(entry) = parse_scope_entry(value) else {
            continue;
        };
        let prefer = scope.contains("auth.x.ai") || scope.contains("oauth");
        if prefer {
            return Some(entry);
        }
        if best.is_none() {
            best = Some(entry);
        }
    }
    best
}

#[derive(Debug, Deserialize)]
struct ScopeEntry {
    key: Option<String>,
    user_id: Option<String>,
    #[serde(default)]
    auth_mode: Option<String>,
}

fn parse_scope_entry(value: &Value) -> Option<GrokAuth> {
    let entry: ScopeEntry = serde_json::from_value(value.clone()).ok()?;
    let token = entry.key?.trim().to_string();
    let user_id = entry.user_id?.trim().to_string();
    if token.is_empty() || user_id.is_empty() {
        return None;
    }
    // 跳过明显是 API key 且无 user 的形态（已用 user_id 过滤）。
    let _ = entry.auth_mode;
    Some(GrokAuth {
        access_token: token,
        user_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_oauth_scope_from_fixture() {
        let json = r#"{
          "https://auth.x.ai::abc-123": {
            "key": "access-token-value",
            "auth_mode": "user",
            "user_id": "user-uuid-1",
            "email": "a@b.c"
          },
          "xai::api_key": {
            "key": "xai-api-key-only"
          }
        }"#;
        let auth = parse_auth_json(json).expect("auth");
        assert_eq!(auth.access_token, "access-token-value");
        assert_eq!(auth.user_id, "user-uuid-1");
    }

    #[test]
    fn parse_skips_entries_without_user_id() {
        let json = r#"{ "xai::api_key": { "key": "only-key" } }"#;
        assert!(parse_auth_json(json).is_none());
    }

    #[test]
    fn auth_path_override_env() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let dir = std::env::temp_dir().join(format!("usages-grok-auth-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        fs::write(
            &path,
            r#"{"https://auth.x.ai::t":{"key":"tok","user_id":"uid-9"}}"#,
        )
        .unwrap();
        std::env::set_var("USAGES_GROK_AUTH_PATH", &path);
        let auth = read_auth().expect("read");
        assert_eq!(auth.user_id, "uid-9");
        assert!(has_local_session());
        std::env::remove_var("USAGES_GROK_AUTH_PATH");
        let _ = fs::remove_dir_all(&dir);
    }
}
