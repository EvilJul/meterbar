//! 本机 Grok Build / `grok login` 会话发现（`~/.grok/auth.json`）。
//! 绝不日志输出 token 明文。
//!
//! OIDC 字段与续期写回对齐 xai-org/grok-build `auth/model.rs` / `auth/oidc/protocol.rs`。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;

/// 与 grok CLI `GROK_AUTH_EARLY_INVALIDATION_SECS` 默认一致：到期前 5 分钟视为需续期。
pub const EARLY_INVALIDATION_SECS: i64 = 300;

/// 账单请求与 OIDC 续期所需材料（仅内存传递）。
#[derive(Debug, Clone)]
pub struct GrokAuth {
    pub access_token: String,
    pub user_id: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub oidc_issuer: Option<String>,
    pub oidc_client_id: Option<String>,
    pub principal_type: Option<String>,
    pub principal_id: Option<String>,
    /// auth.json 顶层 scope 键（写回时定位条目）。
    pub scope: String,
    /// 读取到的 auth.json 路径。
    pub source_path: PathBuf,
}

/// IdP 续期成功后写回 auth.json 的字段。
#[derive(Debug, Clone)]
pub struct RefreshedTokens {
    pub access_token: String,
    /// 若 IdP 未轮换则 `None`，写回时保留原 refresh_token。
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
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
    parse_auth_json_at(&raw, path)
}

/// 从 auth.json 全文解析 GrokAuth（可测；路径占位为空）。
pub fn parse_auth_json(raw: &str) -> Option<GrokAuth> {
    parse_auth_json_at(raw, Path::new(""))
}

fn parse_auth_json_at(raw: &str, path: &Path) -> Option<GrokAuth> {
    let root: Value = serde_json::from_str(raw).ok()?;
    let obj = root.as_object()?;

    // 优先 oauth 范围（key 通常含 auth.x.ai）；跳过纯 api_key 范围若无 user_id。
    let mut best: Option<GrokAuth> = None;
    for (scope, value) in obj {
        let Some(mut entry) = parse_scope_entry(scope, value, path) else {
            continue;
        };
        let prefer = scope.contains("auth.x.ai") || scope.contains("oauth");
        if prefer {
            return Some(entry);
        }
        if best.is_none() {
            entry.scope = scope.clone();
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
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<Value>,
    #[serde(default)]
    oidc_issuer: Option<String>,
    #[serde(default)]
    oidc_client_id: Option<String>,
    #[serde(default)]
    principal_type: Option<String>,
    #[serde(default)]
    principal_id: Option<String>,
}

fn parse_scope_entry(scope: &str, value: &Value, path: &Path) -> Option<GrokAuth> {
    let entry: ScopeEntry = serde_json::from_value(value.clone()).ok()?;
    let token = entry.key?.trim().to_string();
    let user_id = entry.user_id?.trim().to_string();
    if token.is_empty() || user_id.is_empty() {
        return None;
    }
    let _ = entry.auth_mode;
    let refresh_token = entry
        .refresh_token
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let oidc_issuer = entry
        .oidc_issuer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let oidc_client_id = entry
        .oidc_client_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let principal_type = entry
        .principal_type
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let principal_id = entry
        .principal_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Some(GrokAuth {
        access_token: token,
        user_id,
        refresh_token,
        expires_at: entry.expires_at.as_ref().and_then(parse_expires_at),
        oidc_issuer,
        oidc_client_id,
        principal_type,
        principal_id,
        scope: scope.to_string(),
        source_path: path.to_path_buf(),
    })
}

/// 解析 `expires_at`：RFC3339 字符串，或秒/毫秒时间戳。
pub fn parse_expires_at(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::String(s) => parse_expires_at_str(s),
        Value::Number(n) => {
            let n = n.as_f64()?;
            let secs = if n > 1.0e12 { n / 1000.0 } else { n };
            DateTime::from_timestamp(secs.floor() as i64, 0)
        }
        _ => None,
    }
}

fn parse_expires_at_str(s: &str) -> Option<DateTime<Utc>> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Some(dt.with_timezone(&Utc));
    }
    // 部分 auth.json 带超过 6 位小数；截断后再试。
    if let Some((head, rest)) = t.split_once('.') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let suffix: String = rest.chars().skip_while(|c| c.is_ascii_digit()).collect();
        if digits.len() > 6 {
            let truncated = format!("{head}.{}{suffix}", &digits[..6]);
            if let Ok(dt) = DateTime::parse_from_rfc3339(&truncated) {
                return Some(dt.with_timezone(&Utc));
            }
        }
    }
    None
}

/// 是否应在拉 billing 前主动续期（已过期或落入提前失效窗口）。
pub fn access_token_needs_refresh(auth: &GrokAuth, now: DateTime<Utc>) -> bool {
    access_token_needs_refresh_with_buffer(auth.expires_at, now, EARLY_INVALIDATION_SECS)
}

/// 可测：显式 buffer（秒）。无 `expires_at` 时不主动续期（避免误伤长期 key）。
pub fn access_token_needs_refresh_with_buffer(
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    buffer_secs: i64,
) -> bool {
    let Some(exp) = expires_at else {
        return false;
    };
    let buffer = Duration::seconds(buffer_secs.max(0));
    now >= (exp - buffer)
}

/// 是否具备 OIDC refresh 所需字段。
pub fn can_refresh_oidc(auth: &GrokAuth) -> bool {
    auth.refresh_token.as_deref().is_some_and(|s| !s.is_empty())
        && auth.oidc_issuer.as_deref().is_some_and(|s| !s.is_empty())
        && auth.oidc_client_id.as_deref().is_some_and(|s| !s.is_empty())
}

/// 将续期结果合并进 auth.json 文本（保留其它字段与 scope，兼容 grok CLI）。
pub fn merge_refreshed_tokens_json(
    raw: &str,
    scope: &str,
    tokens: &RefreshedTokens,
    create_time: DateTime<Utc>,
) -> Result<String, String> {
    let mut root: Value =
        serde_json::from_str(raw).map_err(|e| format!("解析 auth.json 失败: {e}"))?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "auth.json 根节点不是对象".to_string())?;
    let entry = obj
        .get_mut(scope)
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| format!("auth.json 缺少 scope: {scope}"))?;

    entry.insert("key".into(), Value::String(tokens.access_token.clone()));
    entry.insert(
        "create_time".into(),
        Value::String(create_time.to_rfc3339()),
    );
    if let Some(exp) = tokens.expires_at {
        entry.insert("expires_at".into(), Value::String(exp.to_rfc3339()));
    }
    if let Some(rt) = tokens.refresh_token.as_ref() {
        entry.insert("refresh_token".into(), Value::String(rt.clone()));
    }
    serde_json::to_string_pretty(&root).map_err(|e| format!("序列化 auth.json 失败: {e}"))
}

/// 将续期结果写回 `auth.source_path`（原子替换）。
pub fn write_refreshed_auth(auth: &GrokAuth, tokens: &RefreshedTokens) -> Result<(), String> {
    if auth.source_path.as_os_str().is_empty() {
        return Err("auth.json 路径未知，无法写回".into());
    }
    let raw = fs::read_to_string(&auth.source_path)
        .map_err(|e| format!("读取 auth.json 失败: {e}"))?;
    let next = merge_refreshed_tokens_json(&raw, &auth.scope, tokens, Utc::now())?;
    atomic_write(&auth.source_path, &next)
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).map_err(|e| format!("写入临时 auth.json 失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| format!("提交 auth.json 失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 将续期结果应用到内存中的 `GrokAuth`。
pub fn apply_refreshed_tokens_in_memory(auth: &mut GrokAuth, tokens: &RefreshedTokens) {
    auth.access_token = tokens.access_token.clone();
    if let Some(rt) = tokens.refresh_token.clone() {
        auth.refresh_token = Some(rt);
    }
    if tokens.expires_at.is_some() {
        auth.expires_at = tokens.expires_at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn sample_auth_json() -> &'static str {
        r#"{
          "https://auth.x.ai::abc-123": {
            "key": "access-token-value",
            "auth_mode": "oidc",
            "user_id": "user-uuid-1",
            "email": "a@b.c",
            "refresh_token": "rt-value",
            "expires_at": "2026-08-07T12:00:00Z",
            "oidc_issuer": "https://auth.x.ai",
            "oidc_client_id": "client-1",
            "principal_type": "User",
            "principal_id": "p-1",
            "create_time": "2026-08-01T00:00:00Z"
          },
          "xai::api_key": {
            "key": "xai-api-key-only"
          }
        }"#
    }

    #[test]
    fn parse_oauth_scope_from_fixture() {
        let auth = parse_auth_json(sample_auth_json()).expect("auth");
        assert_eq!(auth.access_token, "access-token-value");
        assert_eq!(auth.user_id, "user-uuid-1");
        assert_eq!(auth.refresh_token.as_deref(), Some("rt-value"));
        assert_eq!(auth.oidc_issuer.as_deref(), Some("https://auth.x.ai"));
        assert_eq!(auth.oidc_client_id.as_deref(), Some("client-1"));
        assert_eq!(auth.scope, "https://auth.x.ai::abc-123");
        assert!(can_refresh_oidc(&auth));
    }

    #[test]
    fn parse_skips_entries_without_user_id() {
        let json = r#"{ "xai::api_key": { "key": "only-key" } }"#;
        assert!(parse_auth_json(json).is_none());
    }

    #[test]
    fn parse_expires_at_nanoseconds() {
        let v = Value::String("2026-08-07T12:32:29.038932978Z".into());
        let dt = parse_expires_at(&v).expect("parse");
        assert_eq!(dt.timestamp(), 1786105949);
    }

    #[test]
    fn needs_refresh_false_when_far_from_expiry() {
        let exp = DateTime::parse_from_rfc3339("2026-08-07T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-08-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!access_token_needs_refresh_with_buffer(
            Some(exp),
            now,
            EARLY_INVALIDATION_SECS
        ));
    }

    #[test]
    fn needs_refresh_true_inside_early_window() {
        let exp = DateTime::parse_from_rfc3339("2026-08-07T12:04:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-08-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(access_token_needs_refresh_with_buffer(
            Some(exp),
            now,
            EARLY_INVALIDATION_SECS
        ));
    }

    #[test]
    fn needs_refresh_true_when_expired() {
        let exp = DateTime::parse_from_rfc3339("2026-08-07T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-08-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(access_token_needs_refresh_with_buffer(
            Some(exp),
            now,
            EARLY_INVALIDATION_SECS
        ));
    }

    #[test]
    fn needs_refresh_false_without_expires_at() {
        let now = Utc::now();
        assert!(!access_token_needs_refresh_with_buffer(
            None,
            now,
            EARLY_INVALIDATION_SECS
        ));
    }

    #[test]
    fn merge_refreshed_tokens_preserves_other_fields() {
        let tokens = RefreshedTokens {
            access_token: "new-at".into(),
            refresh_token: Some("new-rt".into()),
            expires_at: Some(
                DateTime::parse_from_rfc3339("2026-08-08T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        };
        let create = DateTime::parse_from_rfc3339("2026-08-07T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let out = merge_refreshed_tokens_json(
            sample_auth_json(),
            "https://auth.x.ai::abc-123",
            &tokens,
            create,
        )
        .expect("merge");
        let root: Value = serde_json::from_str(&out).unwrap();
        let entry = root["https://auth.x.ai::abc-123"].as_object().unwrap();
        assert_eq!(entry["key"], "new-at");
        assert_eq!(entry["refresh_token"], "new-rt");
        assert_eq!(entry["email"], "a@b.c");
        assert_eq!(entry["user_id"], "user-uuid-1");
        assert_eq!(entry["oidc_client_id"], "client-1");
        assert!(entry["expires_at"].as_str().unwrap().starts_with("2026-08-08"));
        // 未改动的 api_key scope 仍在
        assert_eq!(root["xai::api_key"]["key"], "xai-api-key-only");
    }

    #[test]
    fn merge_keeps_old_refresh_token_when_not_rotated() {
        let tokens = RefreshedTokens {
            access_token: "new-at".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
        };
        let out = merge_refreshed_tokens_json(
            sample_auth_json(),
            "https://auth.x.ai::abc-123",
            &tokens,
            Utc::now(),
        )
        .unwrap();
        let root: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            root["https://auth.x.ai::abc-123"]["refresh_token"],
            "rt-value"
        );
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
            r#"{"https://auth.x.ai::t":{"key":"tok","user_id":"uid-9","refresh_token":"rt","oidc_issuer":"https://auth.x.ai","oidc_client_id":"c1","expires_at":"2099-01-01T00:00:00Z"}}"#,
        )
        .unwrap();
        std::env::set_var("USAGES_GROK_AUTH_PATH", &path);
        let auth = read_auth().expect("read");
        assert_eq!(auth.user_id, "uid-9");
        assert_eq!(auth.source_path, path);
        assert!(has_local_session());
        std::env::remove_var("USAGES_GROK_AUTH_PATH");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refreshed_auth_roundtrip() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let dir = std::env::temp_dir().join(format!("usages-grok-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        fs::write(&path, sample_auth_json()).unwrap();

        let mut auth = parse_auth_json_at(sample_auth_json(), &path).unwrap();
        let tokens = RefreshedTokens {
            access_token: "written-at".into(),
            refresh_token: Some("written-rt".into()),
            expires_at: Some(Utc::now() + Duration::hours(2)),
        };
        write_refreshed_auth(&auth, &tokens).unwrap();
        apply_refreshed_tokens_in_memory(&mut auth, &tokens);

        let again = read_auth_from_path(&path).unwrap();
        assert_eq!(again.access_token, "written-at");
        assert_eq!(again.refresh_token.as_deref(), Some("written-rt"));
        assert_eq!(auth.access_token, "written-at");

        let _ = fs::remove_dir_all(&dir);
    }
}
