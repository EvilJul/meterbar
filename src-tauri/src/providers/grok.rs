//! Grok / SuperGrok 周池用量：本机 `grok login` → cli-chat-proxy billing。
//! 对齐 xai-org/grok-build `extensions/billing.rs`（非公开稳定 API）。

use chrono::Utc;
use serde::Deserialize;

use crate::credentials::grok_session::{self, GrokAuth};
use crate::models::{ProviderStatus, UsageSnapshot, UsageUnit};

use super::UsageProvider;

const DEFAULT_PROXY_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
const TOKEN_AUTH_HEADER: &str = "xai-grok-cli";
const RAW_VERSION: &str = "grok/billing-credits-2026-08";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct GrokProvider;

impl UsageProvider for GrokProvider {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn display_name(&self) -> &'static str {
        "Grok"
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingResponse {
    config: Option<BillingConfig>,
    #[serde(default)]
    subscription_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingConfig {
    credit_usage_percent: Option<f64>,
    current_period: Option<UsagePeriod>,
    monthly_limit: Option<Cent>,
    used: Option<Cent>,
    on_demand_used: Option<Cent>,
    prepaid_balance: Option<Cent>,
    #[serde(default)]
    is_unified_billing_user: Option<bool>,
    billing_period_start: Option<String>,
    billing_period_end: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsagePeriod {
    #[serde(rename = "type")]
    period_type: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Cent {
    #[serde(default)]
    val: i64,
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn proxy_base() -> String {
    std::env::var("USAGES_GROK_PROXY_BASE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PROXY_BASE.to_string())
}

fn snapshot_base(
    status: ProviderStatus,
    message: Option<String>,
    membership: Option<String>,
) -> UsageSnapshot {
    UsageSnapshot {
        provider: "grok".to_string(),
        display_name: "Grok".to_string(),
        membership,
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

/// 将 billing JSON 映射为 UsageSnapshot（可测）。
pub fn map_billing_json(body: &str) -> UsageSnapshot {
    match serde_json::from_str::<BillingResponse>(body) {
        Ok(v) => map_billing(v, None),
        Err(_) => snapshot_base(
            ProviderStatus::ParseError,
            Some("无法解析 Grok billing 响应".into()),
            None,
        ),
    }
}

/// 合并 credits 与 legacy 两份响应：优先 percent；周期优先 credits 的 weekly currentPeriod。
pub fn map_billing_pair(primary: &str, fallback: Option<&str>) -> UsageSnapshot {
    let primary = match serde_json::from_str::<BillingResponse>(primary) {
        Ok(v) => v,
        Err(_) => {
            return snapshot_base(
                ProviderStatus::ParseError,
                Some("无法解析 Grok billing 响应".into()),
                None,
            );
        }
    };
    let fallback = fallback.and_then(|b| serde_json::from_str::<BillingResponse>(b).ok());
    map_billing(primary, fallback)
}

fn percent_from_config(cfg: &BillingConfig) -> Option<f64> {
    if let Some(p) = cfg.credit_usage_percent {
        return Some(p.clamp(0.0, 100.0));
    }
    let limit = cfg.monthly_limit.as_ref().map(|c| c.val).unwrap_or(0);
    let used = cfg.used.as_ref().map(|c| c.val).unwrap_or(0);
    if limit > 0 {
        return Some((used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0));
    }
    None
}

fn map_billing(primary: BillingResponse, fallback: Option<BillingResponse>) -> UsageSnapshot {
    let tier = primary
        .subscription_tier
        .as_deref()
        .or_else(|| {
            fallback
                .as_ref()
                .and_then(|f| f.subscription_tier.as_deref())
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let Some(cfg) = primary.config.as_ref() else {
        if let Some(fb) = fallback {
            return map_billing(fb, None);
        }
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("billing 响应缺少 config".into()),
            tier,
        );
    };

    let fb_cfg = fallback.as_ref().and_then(|f| f.config.as_ref());

    // 实测 format=credits 常省略 percent/used；legacy `/billing` 带 monthlyLimit+used。
    let percent = percent_from_config(cfg)
        .or_else(|| fb_cfg.and_then(percent_from_config))
        // 双源仍无数字时：有周期元数据则按 0%（proto3 省略 0）。
        .or_else(|| {
            let has_period = cfg.current_period.is_some()
                || cfg.billing_period_end.is_some()
                || cfg.is_unified_billing_user == Some(true)
                || fb_cfg.is_some_and(|c| {
                    c.current_period.is_some()
                        || c.billing_period_end.is_some()
                        || c.is_unified_billing_user == Some(true)
                });
            if has_period {
                Some(0.0)
            } else {
                None
            }
        });

    let Some(percent) = percent else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("billing 缺少 creditUsagePercent 与 usable limit".into()),
            tier,
        );
    };

    let period_start = cfg
        .current_period
        .as_ref()
        .and_then(|p| p.start.clone())
        .or_else(|| cfg.billing_period_start.clone())
        .or_else(|| {
            fb_cfg.and_then(|c| {
                c.current_period
                    .as_ref()
                    .and_then(|p| p.start.clone())
                    .or_else(|| c.billing_period_start.clone())
            })
        });
    let period_end = cfg
        .current_period
        .as_ref()
        .and_then(|p| p.end.clone())
        .or_else(|| cfg.billing_period_end.clone())
        .or_else(|| {
            fb_cfg.and_then(|c| {
                c.current_period
                    .as_ref()
                    .and_then(|p| p.end.clone())
                    .or_else(|| c.billing_period_end.clone())
            })
        });

    let on_demand = cfg
        .on_demand_used
        .as_ref()
        .or_else(|| fb_cfg.and_then(|c| c.on_demand_used.as_ref()))
        .map(|c| c.val as f64 / 100.0);

    let mut message_parts = Vec::new();
    let period_type = cfg
        .current_period
        .as_ref()
        .and_then(|p| p.period_type.as_ref())
        .or_else(|| {
            fb_cfg
                .and_then(|c| c.current_period.as_ref())
                .and_then(|p| p.period_type.as_ref())
        });
    if let Some(p) = period_type {
        if p.contains("WEEKLY") {
            message_parts.push("周池".to_string());
        } else if p.contains("MONTHLY") {
            message_parts.push("月池".to_string());
        }
    }
    let prepaid = cfg
        .prepaid_balance
        .as_ref()
        .or_else(|| fb_cfg.and_then(|c| c.prepaid_balance.as_ref()));
    if let Some(prepaid) = prepaid {
        if prepaid.val > 0 {
            message_parts.push(format!("预付 ${:.2}", prepaid.val as f64 / 100.0));
        }
    }
    // 有 cents 额度时附带简要已用/上限（便于核对）
    let limit_cents = cfg
        .monthly_limit
        .as_ref()
        .or_else(|| fb_cfg.and_then(|c| c.monthly_limit.as_ref()))
        .map(|c| c.val);
    let used_cents = cfg
        .used
        .as_ref()
        .or_else(|| fb_cfg.and_then(|c| c.used.as_ref()))
        .map(|c| c.val);
    if let (Some(used), Some(limit)) = (used_cents, limit_cents) {
        if limit > 0 {
            message_parts.push(format!(
                "${:.2} / ${:.2}",
                used as f64 / 100.0,
                limit as f64 / 100.0
            ));
        }
    }
    let message = if message_parts.is_empty() {
        Some("SuperGrok 用量".into())
    } else {
        Some(message_parts.join(" · "))
    };

    UsageSnapshot {
        provider: "grok".to_string(),
        display_name: "Grok".to_string(),
        membership: tier,
        period_start,
        period_end,
        unit: UsageUnit::Unknown,
        used: percent,
        limit: Some(100.0),
        remaining: Some((100.0 - percent).max(0.0)),
        percent_used: Some(percent),
        auto_percent_used: None,
        api_percent_used: None,
        on_demand_used: on_demand,
        status: ProviderStatus::Ok,
        message,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION.to_string()),
    }
}

async fn http_get_billing(auth: &GrokAuth, path_and_query: &str) -> Result<String, UsageSnapshot> {
    let base = proxy_base().trim_end_matches('/').to_string();
    let url = format!("{base}/{path_and_query}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| {
            snapshot_base(
                ProviderStatus::NetworkError,
                Some(format!("HTTP 客户端错误: {e}")),
                None,
            )
        })?;

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", auth.access_token))
        .header("X-XAI-Token-Auth", TOKEN_AUTH_HEADER)
        .header("x-userid", &auth.user_id)
        .header("x-grok-client-version", CLIENT_VERSION)
        .send()
        .await
        .map_err(|e| {
            snapshot_base(
                ProviderStatus::NetworkError,
                Some(format!("Grok billing 请求失败: {e}")),
                None,
            )
        })?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("Grok 登录已失效，请运行 grok login 或打开 grok.com Usage".into()),
            None,
        ));
    }
    if !status.is_success() {
        return Err(snapshot_base(
            ProviderStatus::NetworkError,
            Some(format!("Grok billing HTTP {}", status.as_u16())),
            None,
        ));
    }
    Ok(body)
}

async fn fetch_billing(auth: &GrokAuth) -> UsageSnapshot {
    // format=credits：周池元数据，常省略 percent。
    // 无可用 percent 时再拉 legacy `/billing`（used/monthlyLimit）。
    let credits = match http_get_billing(auth, "billing?format=credits").await {
        Ok(b) => b,
        Err(snap) => return snap,
    };

    let needs_legacy = serde_json::from_str::<BillingResponse>(&credits)
        .ok()
        .and_then(|r| r.config)
        .and_then(|c| percent_from_config(&c))
        .is_none();

    if !needs_legacy {
        return map_billing_json(&credits);
    }

    let legacy = match http_get_billing(auth, "billing").await {
        Ok(b) => Some(b),
        Err(snap) if snap.status == ProviderStatus::NeedsAuth => return snap,
        Err(_) => None,
    };
    map_billing_pair(&credits, legacy.as_deref())
}

/// 刷新 Grok 周池快照。
pub async fn refresh() -> UsageSnapshot {
    let Some(auth) = grok_session::read_auth() else {
        return snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("未找到本机 Grok 登录态；请运行 grok login".into()),
            None,
        );
    };
    fetch_billing(&auth).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_credits_percent_shape() {
        let body = r#"{
          "config": {
            "creditUsagePercent": 42.5,
            "currentPeriod": {
              "type": "USAGE_PERIOD_TYPE_WEEKLY",
              "start": "2026-06-01T00:00:00Z",
              "end": "2026-06-08T00:00:00Z"
            },
            "prepaidBalance": {"val": 1250}
          },
          "subscriptionTier": "SuperGrok"
        }"#;
        let snap = map_billing_json(body);
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.percent_used, Some(42.5));
        assert_eq!(snap.period_end.as_deref(), Some("2026-06-08T00:00:00Z"));
        assert_eq!(snap.membership.as_deref(), Some("SuperGrok"));
        assert!(snap.message.as_deref().unwrap_or("").contains("周池"));
        assert!(snap.message.as_deref().unwrap_or("").contains("12.50"));
    }

    #[test]
    fn maps_legacy_limit_used() {
        let body = r#"{
          "config": {
            "monthlyLimit": {"val": 2000},
            "used": {"val": 500},
            "billingPeriodEnd": "2026-07-01T00:00:00Z"
          }
        }"#;
        let snap = map_billing_json(body);
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.percent_used, Some(25.0));
        assert_eq!(snap.period_end.as_deref(), Some("2026-07-01T00:00:00Z"));
    }

    #[test]
    fn missing_config_is_parse_error() {
        let snap = map_billing_json(r#"{"config":null}"#);
        assert_eq!(snap.status, ProviderStatus::ParseError);
    }

    #[test]
    fn merges_credits_period_with_legacy_used_limit() {
        // 真实账号形态：format=credits 有周池无 percent；/billing 有 used/limit。
        let credits = r#"{
          "config": {
            "currentPeriod": {
              "type": "USAGE_PERIOD_TYPE_WEEKLY",
              "start": "2026-08-07T04:39:19Z",
              "end": "2026-08-14T04:39:19Z"
            },
            "isUnifiedBillingUser": true,
            "prepaidBalance": {"val": 0}
          }
        }"#;
        let legacy = r#"{
          "config": {
            "monthlyLimit": {"val": 15000},
            "used": {"val": 2209},
            "billingPeriodEnd": "2026-09-01T00:00:00Z"
          }
        }"#;
        // 仅 credits、无数字：仍 ok 为 0%（proto 省略），但生产路径会再拉 legacy。
        let alone = map_billing_json(credits);
        assert_eq!(alone.status, ProviderStatus::Ok);
        assert_eq!(alone.percent_used, Some(0.0));

        let merged = map_billing_pair(credits, Some(legacy));
        assert_eq!(merged.status, ProviderStatus::Ok);
        assert!((merged.percent_used.unwrap() - 14.7266).abs() < 0.01);
        assert_eq!(merged.period_end.as_deref(), Some("2026-08-14T04:39:19Z"));
        assert!(merged.message.as_deref().unwrap_or("").contains("周池"));
        assert!(merged.message.as_deref().unwrap_or("").contains("22.09"));
    }
}
