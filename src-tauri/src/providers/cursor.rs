//! Cursor 用量 Provider：本机 accessToken 优先，Cookie fallback → `UsageSnapshot`。

use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::credentials::{self, local_session};
use crate::models::{ProviderStatus, UsageSnapshot, UsageUnit};

use super::UsageProvider;

const USAGE_SUMMARY_CURSOR_URL: &str = "https://cursor.com/api/usage-summary";
const USAGE_SUMMARY_API2_URL: &str = "https://api2.cursor.sh/api/usage/summary";
const GET_CURRENT_PERIOD_USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
const RAW_VERSION: &str = "usage-summary/pro-plus-2026-08";
const RAW_VERSION_DASHBOARD: &str = "dashboard/GetCurrentPeriodUsage-2026-08";

pub struct CursorProvider;

impl UsageProvider for CursorProvider {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn display_name(&self) -> &'static str {
        "Cursor"
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSummaryResponse {
    billing_cycle_start: Option<String>,
    billing_cycle_end: Option<String>,
    membership_type: Option<String>,
    individual_usage: Option<IndividualUsage>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndividualUsage {
    plan: Option<PlanUsage>,
    on_demand: Option<OnDemandUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanUsage {
    used: Option<f64>,
    limit: Option<f64>,
    remaining: Option<f64>,
    auto_percent_used: Option<f64>,
    api_percent_used: Option<f64>,
    total_percent_used: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnDemandUsage {
    enabled: Option<bool>,
    used: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardPeriodResponse {
    billing_cycle_start: Option<Value>,
    billing_cycle_end: Option<Value>,
    membership_type: Option<String>,
    plan_usage: Option<DashboardPlanUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardPlanUsage {
    used: Option<f64>,
    limit: Option<f64>,
    remaining: Option<f64>,
    included_spend: Option<f64>,
    total_spend: Option<f64>,
    auto_percent_used: Option<f64>,
    api_percent_used: Option<f64>,
    total_percent_used: Option<f64>,
}

enum AuthMode<'a> {
    Bearer(&'a str),
    Cookie(&'a str),
}

struct HttpOutcome {
    status: reqwest::StatusCode,
    body: String,
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn snapshot_base(status: ProviderStatus, message: Option<String>) -> UsageSnapshot {
    UsageSnapshot {
        provider: "cursor".to_string(),
        display_name: "Cursor".to_string(),
        membership: None,
        period_start: None,
        period_end: None,
        unit: UsageUnit::Cents,
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

fn looks_unauthenticated(
    status: reqwest::StatusCode,
    body: &str,
    parsed: &UsageSummaryResponse,
) -> bool {
    if status.as_u16() == 401 {
        return true;
    }
    if let Some(err) = &parsed.error {
        let s = err.to_string().to_lowercase();
        if s.contains("not_authenticated") || s.contains("unauthenticated") {
            return true;
        }
    }
    let lower = body.to_lowercase();
    lower.contains("not_authenticated")
}

fn build_client() -> Result<reqwest::Client, UsageSnapshot> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| {
            snapshot_base(
                ProviderStatus::NetworkError,
                Some("无法创建网络客户端".to_string()),
            )
        })
}

async fn http_get(client: &reqwest::Client, url: &str, auth: AuthMode<'_>) -> Result<HttpOutcome, UsageSnapshot> {
    let mut req = client.get(url).header("Accept", "application/json");
    req = match auth {
        AuthMode::Bearer(token) => req.header("Authorization", format!("Bearer {}", token.trim())),
        AuthMode::Cookie(token) => req.header(
            "Cookie",
            format!("WorkosCursorSessionToken={}", token.trim()),
        ),
    };
    let response = req.send().await.map_err(|_| {
        snapshot_base(
            ProviderStatus::NetworkError,
            Some("网络请求失败或超时".to_string()),
        )
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|_| {
        snapshot_base(
            ProviderStatus::NetworkError,
            Some("读取响应失败".to_string()),
        )
    })?;
    Ok(HttpOutcome { status, body })
}

async fn http_post_json(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body_json: &str,
) -> Result<HttpOutcome, UsageSnapshot> {
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token.trim()))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .body(body_json.to_string())
        .send()
        .await
        .map_err(|_| {
            snapshot_base(
                ProviderStatus::NetworkError,
                Some("网络请求失败或超时".to_string()),
            )
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|_| {
        snapshot_base(
            ProviderStatus::NetworkError,
            Some("读取响应失败".to_string()),
        )
    })?;
    Ok(HttpOutcome { status, body })
}

fn try_map_usage_summary(outcome: HttpOutcome) -> Option<UsageSnapshot> {
    let parsed: UsageSummaryResponse = match serde_json::from_str(&outcome.body) {
        Ok(v) => v,
        Err(_) => {
            if outcome.status.as_u16() == 401
                || outcome.body.to_lowercase().contains("not_authenticated")
            {
                return Some(snapshot_base(
                    ProviderStatus::NeedsAuth,
                    Some("会话已失效".to_string()),
                ));
            }
            return None;
        }
    };

    if looks_unauthenticated(outcome.status, &outcome.body, &parsed) {
        return Some(snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("会话已失效".to_string()),
        ));
    }

    if !outcome.status.is_success() {
        return None;
    }

    let snap = map_ok(parsed);
    if snap.status == ProviderStatus::Ok {
        Some(snap)
    } else {
        None
    }
}

fn dashboard_ts_to_iso(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => {
            // Dashboard 常返回毫秒时间戳字符串（如 "1783670222000"）。
            if let Ok(ms) = s.parse::<i64>() {
                return chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339());
            }
            Some(s.clone())
        }
        Value::Number(n) => {
            let ms = n.as_f64()?;
            chrono::DateTime::from_timestamp_millis(ms as i64).map(|dt| dt.to_rfc3339())
        }
        _ => None,
    }
}

fn dashboard_looks_unauthenticated(status: reqwest::StatusCode, body: &str) -> bool {
    if status.as_u16() == 401 {
        return true;
    }
    let lower = body.to_lowercase();
    lower.contains("not_authenticated")
        || lower.contains("\"unauthenticated\"")
        || (lower.contains("unauthenticated") && !status.is_success())
}

fn try_map_dashboard(outcome: HttpOutcome) -> Option<UsageSnapshot> {
    if dashboard_looks_unauthenticated(outcome.status, &outcome.body) {
        return Some(snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("会话已失效".to_string()),
        ));
    }
    if !outcome.status.is_success() {
        return None;
    }

    let parsed: DashboardPeriodResponse = serde_json::from_str(&outcome.body).ok()?;
    let plan = parsed.plan_usage?;

    let limit = plan.limit?;
    let used = plan
        .used
        .or(plan.included_spend)
        .or(plan.total_spend)
        .or_else(|| {
            plan.remaining
                .map(|rem| (limit - rem).max(0.0))
        })?;

    let has_percent = plan.auto_percent_used.is_some()
        || plan.api_percent_used.is_some()
        || plan.total_percent_used.is_some();

    let percent_used = plan.total_percent_used.or_else(|| {
        if limit > 0.0 {
            Some((used / limit) * 100.0)
        } else {
            None
        }
    });

    // 无百分比且无法推导时跳过（让上层继续 fallback）。
    if !has_percent && percent_used.is_none() {
        return None;
    }

    Some(UsageSnapshot {
        provider: "cursor".to_string(),
        display_name: "Cursor".to_string(),
        membership: parsed.membership_type,
        period_start: parsed
            .billing_cycle_start
            .as_ref()
            .and_then(dashboard_ts_to_iso),
        period_end: parsed
            .billing_cycle_end
            .as_ref()
            .and_then(dashboard_ts_to_iso),
        unit: UsageUnit::Cents,
        used,
        limit: Some(limit),
        remaining: plan.remaining,
        percent_used,
        auto_percent_used: plan.auto_percent_used,
        api_percent_used: plan.api_percent_used,
        on_demand_used: None,
        status: ProviderStatus::Ok,
        message: None,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION_DASHBOARD.to_string()),
    })
}

/// Bearer 诊断日志：仅记录 endpoint / HTTP status / 分支名，绝不含 token。
fn log_bearer_branch(endpoint: &str, http_status: Option<u16>, branch: &str) {
    match http_status {
        Some(status) => eprintln!(
            "[usages] cursor bearer endpoint={endpoint} http_status={status} branch={branch}"
        ),
        None => eprintln!("[usages] cursor bearer endpoint={endpoint} branch={branch}"),
    }
}

fn dashboard_transient_snapshot(http_status: Option<u16>) -> UsageSnapshot {
    let message = match http_status {
        Some(status) => format!("Dashboard 瞬时失败（HTTP {status}）"),
        None => "Dashboard 瞬时失败（网络错误）".to_string(),
    };
    snapshot_base(ProviderStatus::NetworkError, Some(message))
}

async fn fetch_with_bearer(client: &reqwest::Client, token: &str) -> Option<UsageSnapshot> {
    // 主路径：Dashboard。仅 Dashboard 自身 401/unauth → NeedsAuth。
    // usage-summary 对本机 token 常 401，视为端点不支持，不得升格 needs_auth。
    let dashboard_transient: Option<UsageSnapshot> =
        match http_post_json(client, GET_CURRENT_PERIOD_USAGE_URL, token, "{}").await {
            Ok(outcome) => {
                let status_code = outcome.status.as_u16();
                match try_map_dashboard(outcome) {
                    Some(snap) if snap.status == ProviderStatus::Ok => {
                        log_bearer_branch("dashboard", Some(status_code), "dashboard_ok");
                        return Some(snap);
                    }
                    Some(snap) if snap.status == ProviderStatus::NeedsAuth => {
                        log_bearer_branch("dashboard", Some(status_code), "dashboard_needs_auth");
                        return Some(snap);
                    }
                    Some(snap) => {
                        log_bearer_branch("dashboard", Some(status_code), "dashboard_transient");
                        Some(snap)
                    }
                    None => {
                        log_bearer_branch("dashboard", Some(status_code), "dashboard_transient");
                        Some(dashboard_transient_snapshot(Some(status_code)))
                    }
                }
            }
            Err(snap) => {
                log_bearer_branch("dashboard", None, "dashboard_transient");
                Some(snap)
            }
        };

    for url in [USAGE_SUMMARY_CURSOR_URL, USAGE_SUMMARY_API2_URL] {
        let endpoint = if url.contains("api2") {
            "usage_summary_api2"
        } else {
            "usage_summary_cursor"
        };
        match http_get(client, url, AuthMode::Bearer(token)).await {
            Ok(outcome) => {
                let status_code = outcome.status.as_u16();
                match try_map_usage_summary(outcome) {
                    Some(snap) if snap.status == ProviderStatus::Ok => {
                        log_bearer_branch(endpoint, Some(status_code), "usage_summary_ok");
                        return Some(snap);
                    }
                    Some(snap) if snap.status == ProviderStatus::NeedsAuth => {
                        // 本机 token 常不被 usage-summary 接受；不置 saw_needs_auth。
                        log_bearer_branch(
                            endpoint,
                            Some(status_code),
                            "usage_summary_unsupported_token",
                        );
                        continue;
                    }
                    Some(snap) => {
                        log_bearer_branch(endpoint, Some(status_code), "usage_summary_other");
                        return Some(snap);
                    }
                    None => {
                        log_bearer_branch(endpoint, Some(status_code), "usage_summary_unmapped");
                    }
                }
            }
            Err(_) => {
                log_bearer_branch(endpoint, None, "usage_summary_network_error");
            }
        }
    }

    dashboard_transient
}

async fn fetch_with_cookie(client: &reqwest::Client, token: &str) -> UsageSnapshot {
    let outcome = match http_get(client, USAGE_SUMMARY_CURSOR_URL, AuthMode::Cookie(token)).await {
        Ok(o) => o,
        Err(snap) => return snap,
    };

    let parsed: UsageSummaryResponse = match serde_json::from_str(&outcome.body) {
        Ok(v) => v,
        Err(_) => {
            if outcome.status.as_u16() == 401
                || outcome.body.to_lowercase().contains("not_authenticated")
            {
                return snapshot_base(
                    ProviderStatus::NeedsAuth,
                    Some("Cookie 已失效，请重新粘贴或确保 Cursor 已登录".to_string()),
                );
            }
            return snapshot_base(
                ProviderStatus::ParseError,
                Some("用量响应无法解析".to_string()),
            );
        }
    };

    if looks_unauthenticated(outcome.status, &outcome.body, &parsed) {
        return snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("Cookie 已失效，请重新粘贴或确保 Cursor 已登录".to_string()),
        );
    }

    if !outcome.status.is_success() {
        return snapshot_base(
            ProviderStatus::NetworkError,
            Some(format!("服务返回 HTTP {}", outcome.status.as_u16())),
        );
    }

    map_ok(parsed)
}

fn no_auth_snapshot(probe: &local_session::LocalSessionProbe) -> UsageSnapshot {
    let message = match probe.failure.as_deref() {
        Some("cursor_db_not_found") => format!(
            "未找到 Cursor 数据库（已尝试 {} 个主目录）。请确认 Cursor 已安装并登录，或在设置中粘贴 Cookie",
            probe.homes_tried.len()
        ),
        Some("cursor_db_not_openable") => {
            "找到 Cursor 数据库但无法只读打开（可能被锁或权限不足）。请重启 Cursor；若仍失败，请在「系统设置 → 隐私与安全性 → 完全磁盘访问权限」中允许 Meterbar"
                .to_string()
        }
        Some("cursor_token_missing") => {
            "Cursor 数据库可读但未找到 accessToken。请重新登录 Cursor，或在设置中粘贴 Cookie".to_string()
        }
        _ => "未检测到 Cursor 登录态。请确保本机已登录 Cursor，或在设置中粘贴 Cookie 作为兜底"
            .to_string(),
    };
    snapshot_base(ProviderStatus::NeedsAuth, Some(message))
}

fn local_session_api_failed_snapshot() -> UsageSnapshot {
    snapshot_base(
        ProviderStatus::NetworkError,
        Some("已检测到 Cursor 登录态，但拉取用量失败，请稍后重试".to_string()),
    )
}

/// 拉取并映射 Cursor 用量；错误以 status 表达，不 panic。
pub async fn refresh() -> UsageSnapshot {
    let client = match build_client() {
        Ok(c) => c,
        Err(snap) => return snap,
    };

    // 单次打开 DB，同时得到 probe 与 token，避免 WAL 下双重打开竞态。
    let session = local_session::probe_and_read();
    let session_probe = session.probe;
    let had_local_session = session_probe.token_len.is_some();
    let mut bearer_needs_auth = false;
    let mut bearer_other_error: Option<UsageSnapshot> = None;

    // 1. 本机 Cursor 登录态
    if had_local_session {
        if let Some(access_token) = session.access_token {
            match fetch_with_bearer(&client, &access_token).await {
                Some(snap) if snap.status == ProviderStatus::Ok => return snap,
                Some(snap) if snap.status == ProviderStatus::NeedsAuth => {
                    // 真 NeedsAuth：仍尝试 Cookie 兜底。
                    bearer_needs_auth = true;
                }
                Some(snap) => bearer_other_error = Some(snap),
                None => {}
            }
        }
    }

    // 2. 已保存 Cookie（Bearer 失效或缺失时的兜底）
    match credentials::get_token() {
        Ok(Some(cookie)) => fetch_with_cookie(&client, &cookie).await,
        Ok(None) if had_local_session => {
            if bearer_needs_auth {
                snapshot_base(
                    ProviderStatus::NeedsAuth,
                    Some(
                        "Cursor 登录会话已失效，请重新登录 Cursor 或在设置中粘贴 Cookie"
                            .to_string(),
                    ),
                )
            } else if let Some(snap) = bearer_other_error {
                snap
            } else {
                local_session_api_failed_snapshot()
            }
        }
        Ok(None) => no_auth_snapshot(&session_probe),
        Err(_) if had_local_session => snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("已检测到 Cursor 登录态，但无法读取 Cookie 兜底凭证".to_string()),
        ),
        Err(_) => snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("无法读取凭证；请确保 Cursor 已登录或粘贴 Cookie".to_string()),
        ),
    }
}

/// 离线解析并映射 `usage-summary` JSON（供单测；不发起网络请求）。
pub(crate) fn map_usage_summary_json(body: &str) -> Result<UsageSnapshot, serde_json::Error> {
    let parsed: UsageSummaryResponse = serde_json::from_str(body)?;
    Ok(map_ok(parsed))
}

fn map_ok(parsed: UsageSummaryResponse) -> UsageSnapshot {
    let Some(individual) = parsed.individual_usage else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("响应缺少 individualUsage".to_string()),
        );
    };
    let Some(plan) = individual.plan else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("响应缺少 individualUsage.plan".to_string()),
        );
    };
    let Some(used) = plan.used else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("响应缺少 plan.used".to_string()),
        );
    };

    if plan.auto_percent_used.is_none()
        && plan.api_percent_used.is_none()
        && plan.total_percent_used.is_none()
    {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("响应缺少 plan 百分比字段".to_string()),
        );
    }

    let on_demand_used = individual
        .on_demand
        .as_ref()
        .filter(|od| od.enabled.unwrap_or(false))
        .and_then(|od| od.used);

    UsageSnapshot {
        provider: "cursor".to_string(),
        display_name: "Cursor".to_string(),
        membership: parsed.membership_type,
        period_start: parsed.billing_cycle_start,
        period_end: parsed.billing_cycle_end,
        unit: UsageUnit::Cents,
        used,
        limit: plan.limit,
        remaining: plan.remaining,
        percent_used: plan.total_percent_used,
        auto_percent_used: plan.auto_percent_used,
        api_percent_used: plan.api_percent_used,
        on_demand_used,
        status: ProviderStatus::Ok,
        message: None,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRO_PLUS_FIXTURE: &str = r#"{
      "billingCycleStart": "2026-07-10T07:57:02.000Z",
      "billingCycleEnd": "2026-08-10T07:57:02.000Z",
      "membershipType": "pro_plus",
      "limitType": "user",
      "isUnlimited": false,
      "individualUsage": {
        "plan": {
          "enabled": true,
          "used": 7000,
          "limit": 7000,
          "remaining": 0,
          "breakdown": { "included": 7000, "bonus": 40603, "total": 47603 },
          "autoPercentUsed": 45.7275,
          "apiPercentUsed": 100,
          "totalPercentUsed": 52.310989010989005
        },
        "onDemand": { "enabled": false, "used": 0, "limit": null, "remaining": null }
      },
      "teamUsage": {}
    }"#;

    #[test]
    fn maps_pro_plus_fixture_offline() {
        let snap = map_usage_summary_json(PRO_PLUS_FIXTURE).expect("fixture 应可解析");

        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.unit, UsageUnit::Cents);
        assert_eq!(snap.used, 7000.0);
        assert_eq!(snap.limit, Some(7000.0));
        assert_eq!(snap.membership.as_deref(), Some("pro_plus"));
        assert_eq!(snap.on_demand_used, None);

        let percent = snap.percent_used.expect("应有 percent_used");
        let expected = 52.310989010989005_f64;
        assert!(
            (percent - expected).abs() < 1e-12,
            "percent_used={percent}, expected≈{expected}"
        );

        let auto = snap.auto_percent_used.expect("应有 auto_percent_used");
        assert!(
            (auto - 45.7275_f64).abs() < 1e-12,
            "auto_percent_used={auto}"
        );
        let api = snap.api_percent_used.expect("应有 api_percent_used");
        assert!((api - 100.0_f64).abs() < 1e-12, "api_percent_used={api}");
    }

    #[test]
    fn maps_dashboard_plan_usage() {
        let body = r#"{
          "billingCycleStart": "2026-07-10T07:57:02.000Z",
          "billingCycleEnd": "2026-08-10T07:57:02.000Z",
          "membershipType": "pro_plus",
          "planUsage": {
            "used": 7000,
            "limit": 7000,
            "remaining": 0,
            "autoPercentUsed": 45.7275,
            "apiPercentUsed": 100,
            "totalPercentUsed": 52.31
          }
        }"#;
        let outcome = HttpOutcome {
            status: reqwest::StatusCode::OK,
            body: body.to_string(),
        };
        let snap = try_map_dashboard(outcome).expect("dashboard 应可映射");
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.used, 7000.0);
        assert_eq!(snap.auto_percent_used, Some(45.7275));
    }

    #[test]
    fn dashboard_401_is_needs_auth() {
        let outcome = HttpOutcome {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: r#"{"error":"unauthenticated"}"#.to_string(),
        };
        let snap = try_map_dashboard(outcome).expect("应映射");
        assert_eq!(snap.status, ProviderStatus::NeedsAuth);
    }

    #[test]
    fn dashboard_429_is_unmapped_transient() {
        let outcome = HttpOutcome {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            body: "rate limited".to_string(),
        };
        assert!(try_map_dashboard(outcome).is_none());
        let transient = dashboard_transient_snapshot(Some(429));
        assert_eq!(transient.status, ProviderStatus::NetworkError);
        assert!(
            transient
                .message
                .as_deref()
                .unwrap_or("")
                .contains("429"),
            "message={}",
            transient.message.unwrap_or_default()
        );
    }

    #[test]
    fn maps_dashboard_real_shape_with_ms_timestamps() {
        // 贴近本机实测响应：无 used/remaining，含 includedSpend / remainingBonus:bool / 毫秒字符串时间戳。
        let body = r#"{
          "billingCycleStart": "1783670222000",
          "billingCycleEnd": "1786348622000",
          "planUsage": {
            "totalSpend": 52657,
            "includedSpend": 7000,
            "bonusSpend": 45657,
            "limit": 7000,
            "remainingBonus": false,
            "autoPercentUsed": 52.045,
            "apiPercentUsed": 100,
            "totalPercentUsed": 57.864835164835156
          }
        }"#;
        let outcome = HttpOutcome {
            status: reqwest::StatusCode::OK,
            body: body.to_string(),
        };
        let snap = try_map_dashboard(outcome).expect("实测形态应可映射");
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.used, 7000.0);
        assert_eq!(snap.limit, Some(7000.0));
        assert!(snap.period_start.is_some());
        assert!(snap.period_end.is_some());
        assert_eq!(snap.api_percent_used, Some(100.0));
    }
}
