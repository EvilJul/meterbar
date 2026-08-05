//! Codex 本地额度 Provider：短生命周期拉起 `codex app-server`（stdio JSON-RPC），
//! 调用 `account/rateLimits/read`，映射为 `UsageSnapshot`。
//!
//! ## 文档核对（2026-08-05）
//! - 入口：`https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md`
//! - 传输：默认 `stdio://`（`--stdio` / `--listen stdio://`），newline-delimited JSON（JSONL）
//! - 握手：每连接一次 `initialize`（`clientInfo`）→ `initialized` 通知 → 再调业务方法
//! - 方法：`account/rateLimits/read`；主窗口字段 `rateLimits.primary.usedPercent` /
//!   `windowDurationMins` / `resetsAt`（Unix 秒）；次窗口为 `secondary`（首版主进度用 primary）
//! - 未登录：JSON-RPC error（如 `chatgpt authentication required to read rate limits`）→ `needs_auth`
//!
//! 安全：不读取/不落盘 ChatGPT cookie；不写密钥到 settings；日志不打印 token/cookie。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::models::{ProviderStatus, UsageSnapshot, UsageUnit};

use super::UsageProvider;

const RAW_VERSION: &str = "codex/app-server-rateLimits-2026-08-05";
const CLIENT_NAME: &str = "usages";
const CLIENT_TITLE: &str = "Meterbar";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 整体超时（含拉起子进程 + 握手 + RPC）。
const RPC_TIMEOUT: Duration = Duration::from_secs(12);

pub struct CodexProvider;

impl UsageProvider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }
}

#[derive(Debug)]
enum CodexFetchError {
    NotInstalled,
    Timeout,
    Process(String),
    AuthRequired,
    Rpc(String),
    Parse(String),
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn snapshot_base(status: ProviderStatus, message: Option<String>) -> UsageSnapshot {
    UsageSnapshot {
        provider: "codex".to_string(),
        display_name: "Codex".to_string(),
        membership: None,
        period_start: None,
        period_end: None,
        unit: UsageUnit::Unknown,
        used: 0.0,
        limit: None,
        remaining: None,
        percent_used: None,
        auto_percent_used: None,
        api_percent_used: None,
        on_demand_used: None,
        status,
        message,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION.to_string()),
    }
}

fn unavailable(message: &str) -> UsageSnapshot {
    // 失败路径：不写入看起来像成功的假百分比。
    snapshot_base(
        ProviderStatus::NetworkError,
        Some(message.to_string()),
    )
}

/// 解析 `codex` 可执行文件：`USAGES_CODEX_BIN` > PATH > 常见安装位置。
fn resolve_codex_bin() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("USAGES_CODEX_BIN") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    if let Some(from_path) = which_codex() {
        return Some(from_path);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ];
    if let Some(home) = home {
        candidates.push(home.join(".local/bin/codex"));
        candidates.push(home.join(".cargo/bin/codex"));
        candidates.push(home.join("bin/codex"));
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn which_codex() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("codex");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitsReadResult {
    rate_limits: Option<RateLimitSnapshot>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitSnapshot {
    primary: Option<RateLimitWindow>,
    secondary: Option<RateLimitWindow>,
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RateLimitWindow {
    used_percent: Option<f64>,
    window_duration_mins: Option<u64>,
    resets_at: Option<i64>,
}

fn format_window_mins(mins: u64) -> String {
    if mins % (24 * 60) == 0 {
        let days = mins / (24 * 60);
        return format!("{days} 天窗口");
    }
    if mins % 60 == 0 {
        let hours = mins / 60;
        return format!("{hours} 小时窗口");
    }
    format!("{mins} 分钟窗口")
}

fn resets_at_iso(ts: i64) -> Option<String> {
    Utc.timestamp_opt(ts, 0).single().map(|dt| dt.to_rfc3339())
}

fn map_rate_limits(result: RateLimitsReadResult) -> UsageSnapshot {
    let Some(limits) = result.rate_limits else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("本地 Codex 响应缺少 rateLimits".to_string()),
        );
    };

    // 首版主进度：primary；若 primary 无 usedPercent 再尝试 secondary。
    let primary = limits.primary.clone();
    let secondary = limits.secondary.clone();
    let main = match (&primary, &secondary) {
        (Some(p), _) if p.used_percent.is_some() => Some(p),
        (_, Some(s)) if s.used_percent.is_some() => Some(s),
        _ => None,
    };

    let Some(main) = main else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("本地 Codex 协议变更：缺少 usedPercent".to_string()),
        );
    };

    let used_percent = main.used_percent.expect("checked above");
    let period_end = main.resets_at.and_then(resets_at_iso);

    let mut parts: Vec<String> = Vec::new();
    if let Some(mins) = main.window_duration_mins {
        parts.push(format_window_mins(mins));
    }
    if let Some(sec) = &secondary {
        if let Some(pct) = sec.used_percent {
            let mut sec_part = format!("次窗口 {pct:.0}%");
            if let Some(mins) = sec.window_duration_mins {
                sec_part.push_str(&format!("（{}）", format_window_mins(mins)));
            }
            parts.push(sec_part);
        }
    }

    let message = if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    };

    UsageSnapshot {
        provider: "codex".to_string(),
        display_name: "Codex".to_string(),
        membership: limits.plan_type,
        period_start: None,
        period_end,
        unit: UsageUnit::Unknown,
        used: used_percent,
        limit: Some(100.0),
        remaining: Some((100.0 - used_percent).max(0.0)),
        percent_used: Some(used_percent),
        auto_percent_used: None,
        api_percent_used: None,
        on_demand_used: None,
        status: ProviderStatus::Ok,
        message,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION.to_string()),
    }
}

fn is_auth_required_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("authentication required")
        || lower.contains("not authenticated")
        || lower.contains("not logged in")
        || lower.contains("unauthorized")
        || lower.contains("chatgpt authentication")
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn send_line(stdin: &mut impl Write, value: &Value) -> Result<(), CodexFetchError> {
    let line = serde_json::to_string(value).map_err(|e| CodexFetchError::Process(e.to_string()))?;
    writeln!(stdin, "{line}").map_err(|e| CodexFetchError::Process(e.to_string()))?;
    stdin
        .flush()
        .map_err(|e| CodexFetchError::Process(e.to_string()))
}

/// 读取 stdout，跳过通知，直到匹配 `id` 的 result/error。
fn read_response(
    reader: &mut impl BufRead,
    expect_id: u64,
) -> Result<Value, CodexFetchError> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| CodexFetchError::Process(e.to_string()))?;
        if n == 0 {
            return Err(CodexFetchError::Process(
                "app-server 意外结束".to_string(),
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(trimmed)
            .map_err(|e| CodexFetchError::Parse(format!("JSON-RPC 解析失败: {e}")))?;

        // 通知（无 id）跳过。
        if msg.get("id").is_none() {
            continue;
        }
        let id = msg
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| CodexFetchError::Parse("JSON-RPC id 无效".to_string()))?;
        if id != expect_id {
            continue;
        }

        if let Some(err) = msg.get("error") {
            let message = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            if is_auth_required_message(message) {
                return Err(CodexFetchError::AuthRequired);
            }
            return Err(CodexFetchError::Rpc(message.to_string()));
        }

        return msg
            .get("result")
            .cloned()
            .ok_or_else(|| CodexFetchError::Parse("JSON-RPC 缺少 result".to_string()));
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

fn error_to_snapshot(err: CodexFetchError) -> UsageSnapshot {
    match err {
        CodexFetchError::NotInstalled => unavailable("本地 Codex 不可用：未找到 codex 命令"),
        CodexFetchError::Timeout => unavailable("本地 Codex 不可用：读取超时"),
        CodexFetchError::Process(_) => unavailable("本地 Codex 不可用"),
        CodexFetchError::AuthRequired => snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("请先在本机 Codex / CLI 登录 ChatGPT".to_string()),
        ),
        CodexFetchError::Rpc(msg) => {
            // 不回显可能含敏感信息的长错误；仅短提示。
            let _ = msg;
            unavailable("本地 Codex 不可用")
        }
        CodexFetchError::Parse(msg) => snapshot_base(ProviderStatus::ParseError, Some(msg)),
    }
}

fn fetch_with_timeout(codex_bin: PathBuf) -> UsageSnapshot {
    let (tx, rx) = mpsc::channel();
    let (pid_tx, pid_rx) = mpsc::channel::<u32>();
    thread::spawn(move || {
        let mut child = match Command::new(&codex_bin)
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let err = if e.kind() == std::io::ErrorKind::NotFound {
                    CodexFetchError::NotInstalled
                } else {
                    CodexFetchError::Process(e.to_string())
                };
                let _ = tx.send(error_to_snapshot(err));
                return;
            }
        };
        let _ = pid_tx.send(child.id());

        let result = (|| {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| CodexFetchError::Process("无法打开 stdin".to_string()))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| CodexFetchError::Process("无法打开 stdout".to_string()))?;
            let mut reader = BufReader::new(stdout);

            send_line(
                &mut stdin,
                &json!({
                    "method": "initialize",
                    "id": 0,
                    "params": {
                        "clientInfo": {
                            "name": CLIENT_NAME,
                            "title": CLIENT_TITLE,
                            "version": CLIENT_VERSION
                        }
                    }
                }),
            )?;
            let _init = read_response(&mut reader, 0)?;

            send_line(
                &mut stdin,
                &json!({
                    "method": "initialized",
                    "params": {}
                }),
            )?;

            thread::sleep(Duration::from_millis(400));

            send_line(
                &mut stdin,
                &json!({
                    "method": "account/rateLimits/read",
                    "id": 1,
                    "params": {}
                }),
            )?;
            let result_val = read_response(&mut reader, 1)?;
            let parsed: RateLimitsReadResult = serde_json::from_value(result_val)
                .map_err(|e| CodexFetchError::Parse(format!("rateLimits schema 不匹配: {e}")))?;
            Ok(map_rate_limits(parsed))
        })();

        drop(child.stdin.take());
        terminate_child(&mut child);

        let snap = match result {
            Ok(s) => s,
            Err(e) => error_to_snapshot(e),
        };
        let _ = tx.send(snap);
    });

    match rx.recv_timeout(RPC_TIMEOUT) {
        Ok(snap) => snap,
        Err(_) => {
            if let Ok(pid) = pid_rx.try_recv() {
                kill_pid(pid);
            }
            unavailable("本地 Codex 不可用：读取超时")
        }
    }
}

/// 拉取 Codex 本地 rate limits 快照。
pub async fn refresh() -> UsageSnapshot {
    let Some(bin) = resolve_codex_bin() else {
        return unavailable("本地 Codex 不可用：未安装或未在 PATH 中");
    };

    // RPC 在独立线程 + 超时内完成，避免长时间卡住调用方。
    fetch_with_timeout(bin)
}

/// 离线映射（单测）：解析 `account/rateLimits/read` 的 `result` JSON。
pub(crate) fn map_rate_limits_json(body: &str) -> Result<UsageSnapshot, serde_json::Error> {
    let parsed: RateLimitsReadResult = serde_json::from_str(body)?;
    Ok(map_rate_limits(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_primary_used_percent() {
        let body = r#"{
          "rateLimits": {
            "primary": { "usedPercent": 25, "windowDurationMins": 300, "resetsAt": 1730947200 },
            "secondary": { "usedPercent": 40, "windowDurationMins": 10080, "resetsAt": 1731552000 },
            "planType": "pro"
          }
        }"#;
        let snap = map_rate_limits_json(body).expect("应可解析");
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.provider, "codex");
        assert_eq!(snap.percent_used, Some(25.0));
        assert_eq!(snap.membership.as_deref(), Some("pro"));
        assert!(snap.period_end.is_some());
        assert!(snap.message.as_deref().unwrap_or("").contains("小时窗口"));
        // 失败态才禁假数据；成功态必须有真实百分比。
        assert!(snap.percent_used.is_some());
    }

    #[test]
    fn missing_used_percent_is_parse_error_without_fake_bar() {
        let body = r#"{ "rateLimits": { "primary": {}, "secondary": null } }"#;
        let snap = map_rate_limits_json(body).expect("应可解析外壳");
        assert_eq!(snap.status, ProviderStatus::ParseError);
        assert!(snap.percent_used.is_none());
    }

    #[test]
    fn auth_message_detection() {
        assert!(is_auth_required_message(
            "chatgpt authentication required to read rate limits"
        ));
        assert!(!is_auth_required_message("server overloaded"));
    }

    #[test]
    fn failure_snapshot_has_no_fake_percent() {
        let snap = unavailable("本地 Codex 不可用");
        assert_eq!(snap.status, ProviderStatus::NetworkError);
        assert!(snap.percent_used.is_none());
        assert_eq!(snap.used, 0.0);
    }
}
