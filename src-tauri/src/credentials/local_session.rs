//! 从 Cursor 本机 SQLite 只读读取登录 accessToken。
//! 绝不写入、持久化或日志输出 token。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// 社区实现（Cursor-Usage-Status 等）使用的 ItemTable key，按优先级排列。
const ACCESS_TOKEN_KEYS: &[&str] = &["cursorAuth/accessToken"];

/// 本机会话探测结果（不含 token 明文）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSessionProbe {
    pub homes_tried: Vec<String>,
    pub db_paths_found: usize,
    pub db_paths_openable: usize,
    pub token_len: Option<usize>,
    pub failure: Option<String>,
}

/// Unix：从 passwd 解析真实主目录，避免 GUI/启动器传入错误 `HOME`。
#[cfg(unix)]
fn passwd_home_dir() -> Option<PathBuf> {
    use std::ffi::CStr;

    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        let dir = CStr::from_ptr((*pw).pw_dir);
        let s = dir.to_str().ok()?;
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    }
}

#[cfg(not(unix))]
fn passwd_home_dir() -> Option<PathBuf> {
    None
}

fn push_unique(homes: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, candidate: PathBuf) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if seen.insert(candidate.clone()) {
        homes.push(candidate);
    }
}

/// 按优先级收集候选主目录：passwd → `home_dir()` → `$HOME`。
pub fn home_dir_candidates() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let mut seen = HashSet::new();
    if let Some(h) = passwd_home_dir() {
        push_unique(&mut homes, &mut seen, h);
    }
    if let Some(h) = std::env::home_dir() {
        push_unique(&mut homes, &mut seen, h);
    }
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        push_unique(&mut homes, &mut seen, h);
    }
    homes
}

/// 凭证模块共用的主目录解析。
pub fn primary_home_dir() -> Option<PathBuf> {
    home_dir_candidates().into_iter().next()
}

/// 平台候选 Cursor globalStorage 路径（含 Insiders / backup）。
fn candidate_state_db_paths(home: &Path) -> Vec<PathBuf> {
    crate::platform_paths::candidate_cursor_state_db_paths(home)
}

fn resolve_state_db_paths() -> Vec<PathBuf> {
    if let Ok(override_path) = std::env::var("USAGES_CURSOR_STATE_DB") {
        let p = PathBuf::from(override_path);
        if p.exists() {
            return vec![p];
        }
        return Vec::new();
    }

    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for home in home_dir_candidates() {
        for path in candidate_state_db_paths(&home) {
            if path.exists() && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths
}

fn open_readonly(path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if let Ok(conn) = Connection::open_with_flags(path, flags) {
        return Some(conn);
    }
    // Cursor 持有 WAL 锁时，尝试 URI 只读 + nolock / immutable。
    let encoded = path.to_string_lossy().replace('\'', "%27");
    for query in ["mode=ro&nolock=1", "mode=ro&nolock=1&immutable=1"] {
        let uri = format!("file:{encoded}?{query}");
        if let Ok(conn) = Connection::open_with_flags(&uri, flags | OpenFlags::SQLITE_OPEN_URI) {
            return Some(conn);
        }
    }
    None
}

fn read_token_from_conn(conn: &Connection) -> Option<String> {
    for key in ACCESS_TOKEN_KEYS {
        let mut stmt = conn
            .prepare("SELECT value FROM ItemTable WHERE key = ?1 LIMIT 1")
            .ok()?;
        if let Ok(token) = stmt.query_row([key], |row| row.get::<_, String>(0)) {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// 单次打开结果：probe 元数据 + 可选 token（token 仅内存传递，不入日志）。
#[derive(Debug)]
pub struct LocalSessionRead {
    pub probe: LocalSessionProbe,
    pub access_token: Option<String>,
}

fn build_probe(
    homes_tried: Vec<String>,
    db_paths_found: usize,
    db_paths_openable: usize,
    token_len: Option<usize>,
) -> LocalSessionProbe {
    let failure = if db_paths_found == 0 {
        Some("cursor_db_not_found".into())
    } else if db_paths_openable == 0 {
        Some("cursor_db_not_openable".into())
    } else if token_len.is_none() {
        Some("cursor_token_missing".into())
    } else {
        None
    };

    LocalSessionProbe {
        homes_tried,
        db_paths_found,
        db_paths_openable,
        token_len,
        failure,
    }
}

/// 每候选路径至多只读打开一次，同时得到 probe 字段与 accessToken。
pub fn probe_and_read() -> LocalSessionRead {
    let homes_tried: Vec<String> = home_dir_candidates()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();
    let paths = resolve_state_db_paths();
    let db_paths_found = paths.len();
    let mut db_paths_openable = 0;
    let mut access_token = None;

    for path in &paths {
        let Some(conn) = open_readonly(path) else {
            continue;
        };
        db_paths_openable += 1;
        if access_token.is_none() {
            access_token = read_token_from_conn(&conn);
        }
    }

    let token_len = access_token.as_ref().map(|t| t.len());
    let probe = build_probe(homes_tried, db_paths_found, db_paths_openable, token_len);
    LocalSessionRead {
        probe,
        access_token,
    }
}

/// 探测本机 Cursor 登录态（不返回 token 明文）。
pub fn probe() -> LocalSessionProbe {
    probe_and_read().probe
}

/// 从 Cursor `state.vscdb` 读取 accessToken；不可读时返回 `None`（不 panic）。
/// 刷新路径请优先用 [`probe_and_read`]，避免与 probe 双重打开。
#[allow(dead_code)] // 公开薄封装；生产刷新走 probe_and_read
pub fn read_access_token() -> Option<String> {
    probe_and_read().access_token
}

/// 是否检测到本机 Cursor 登录态（不验证 token 有效性）。
pub fn has_local_session() -> bool {
    probe_and_read().probe.token_len.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_test_db(dir: &std::path::Path, token: &str) -> PathBuf {
        let db_path = dir.join("state.vscdb");
        let conn = Connection::open(&db_path).expect("create test db");
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .expect("create table");
        conn.execute(
            "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
            [ACCESS_TOKEN_KEYS[0], token],
        )
        .expect("insert token");
        db_path
    }

    #[test]
    fn reads_token_from_test_db() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-local-session-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");

        let db_path = write_test_db(&dir, "test-access-token-value");
        std::env::set_var("USAGES_CURSOR_STATE_DB", &db_path);

        let got = read_access_token().expect("should read token");
        assert_eq!(got, "test-access-token-value");
        assert!(has_local_session());

        std::env::remove_var("USAGES_CURSOR_STATE_DB");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_and_read_temp_db_single_pass() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-local-session-probe-read-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");

        let db_path = write_test_db(&dir, "single-pass-token");
        std::env::set_var("USAGES_CURSOR_STATE_DB", &db_path);

        let session = probe_and_read();
        assert_eq!(session.probe.db_paths_found, 1);
        assert_eq!(session.probe.db_paths_openable, 1);
        assert_eq!(session.probe.token_len, Some("single-pass-token".len()));
        assert!(session.probe.failure.is_none());
        assert_eq!(session.access_token.as_deref(), Some("single-pass-token"));

        // 薄封装与合并 API 一致
        assert_eq!(probe().token_len, session.probe.token_len);
        assert_eq!(read_access_token().as_deref(), Some("single-pass-token"));

        std::env::remove_var("USAGES_CURSOR_STATE_DB");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_db_returns_none() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::set_var(
            "USAGES_CURSOR_STATE_DB",
            "/tmp/usages-nonexistent-state-vscdb-xyz",
        );
        assert!(read_access_token().is_none());
        let session = probe_and_read();
        assert_eq!(session.probe.db_paths_found, 0);
        assert_eq!(
            session.probe.failure.as_deref(),
            Some("cursor_db_not_found")
        );
        assert!(session.access_token.is_none());
        std::env::remove_var("USAGES_CURSOR_STATE_DB");
    }

    #[test]
    fn empty_token_returns_none() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "usages-local-session-empty-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");

        let db_path = write_test_db(&dir, "   ");
        std::env::set_var("USAGES_CURSOR_STATE_DB", &db_path);

        assert!(read_access_token().is_none());

        std::env::remove_var("USAGES_CURSOR_STATE_DB");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_home_without_home_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("HOME");
        std::env::remove_var("USAGES_CURSOR_STATE_DB");
        let _ = read_access_token();
    }

    #[cfg(unix)]
    #[test]
    fn wrong_home_env_still_resolves_via_passwd() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::set_var("HOME", "/var/empty");
        std::env::remove_var("USAGES_CURSOR_STATE_DB");

        let passwd_home = passwd_home_dir().expect("passwd home");
        let homes = home_dir_candidates();
        assert!(
            homes.first() == Some(&passwd_home),
            "passwd home should be first: {homes:?}"
        );
        assert_ne!(passwd_home, PathBuf::from("/var/empty"));

        let p = probe();
        if p.db_paths_found > 0 {
            assert!(
                p.token_len.is_some(),
                "expected token with wrong HOME, probe={p:?}"
            );
        }

        std::env::remove_var("HOME");
    }

    #[test]
    fn probe_real_cursor_db() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("USAGES_CURSOR_STATE_DB");

        let p = probe();
        eprintln!("probe={p:?}");
        eprintln!("HOME env={:?}", std::env::var_os("HOME"));

        if p.db_paths_found > 0 {
            assert!(p.db_paths_openable > 0, "db should be openable: {p:?}");
            assert!(p.token_len.is_some(), "token should exist: {p:?}");
        }
    }
}
