//! DeepSeek 余额 Provider：API Key → `GET /user/balance`。
//! 官方文档：https://api-docs.deepseek.com/api/get-user-balance

use chrono::Utc;
use serde::Deserialize;

use crate::credentials;
use crate::models::{ProviderStatus, UsageSnapshot, UsageUnit};

use super::UsageProvider;

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
const RAW_VERSION: &str = "deepseek/user-balance-2026-08";

pub struct DeepSeekProvider;

impl UsageProvider for DeepSeekProvider {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn display_name(&self) -> &'static str {
        "DeepSeek"
    }
}

#[derive(Debug, Deserialize)]
struct BalanceResponse {
    is_available: Option<bool>,
    balance_infos: Option<Vec<BalanceInfo>>,
}

#[derive(Debug, Deserialize)]
struct BalanceInfo {
    currency: Option<String>,
    total_balance: Option<String>,
    granted_balance: Option<String>,
    topped_up_balance: Option<String>,
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn snapshot_base(status: ProviderStatus, message: Option<String>) -> UsageSnapshot {
    UsageSnapshot {
        provider: "deepseek".to_string(),
        display_name: "DeepSeek".to_string(),
        membership: None,
        period_start: None,
        period_end: None,
        unit: UsageUnit::Balance,
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

fn parse_amount(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok()
}

fn pick_balance_info(infos: &[BalanceInfo]) -> Option<&BalanceInfo> {
    // 优先 CNY，其次 USD，否则取第一条有效余额。
    infos
        .iter()
        .find(|i| {
            i.currency
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case("CNY"))
        })
        .or_else(|| {
            infos.iter().find(|i| {
                i.currency
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case("USD"))
            })
        })
        .or_else(|| infos.first())
}

fn map_balance(parsed: BalanceResponse) -> UsageSnapshot {
    let Some(infos) = parsed.balance_infos.filter(|v| !v.is_empty()) else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("余额响应缺少 balance_infos".to_string()),
        );
    };
    let Some(info) = pick_balance_info(&infos) else {
        return snapshot_base(
            ProviderStatus::ParseError,
            Some("余额响应无法解析".to_string()),
        );
    };

    let total = info
        .total_balance
        .as_deref()
        .and_then(parse_amount)
        .unwrap_or(0.0);
    let granted = info
        .granted_balance
        .as_deref()
        .and_then(parse_amount)
        .unwrap_or(0.0);
    let topped = info
        .topped_up_balance
        .as_deref()
        .and_then(parse_amount)
        .unwrap_or(0.0);

    let available = parsed.is_available.unwrap_or(true);
    let message = if !available {
        Some("余额不足以调用 API".to_string())
    } else {
        None
    };

    UsageSnapshot {
        provider: "deepseek".to_string(),
        display_name: "DeepSeek".to_string(),
        membership: info.currency.clone(),
        period_start: None,
        period_end: None,
        unit: UsageUnit::Balance,
        // used = 已充值部分展示辅助；remaining = 可用总额。
        used: topped,
        limit: None,
        remaining: Some(total),
        percent_used: None,
        auto_percent_used: None,
        api_percent_used: None,
        on_demand_used: Some(granted),
        status: ProviderStatus::Ok,
        message,
        fetched_at: now_iso(),
        raw_version: Some(RAW_VERSION.to_string()),
    }
}

/// 拉取 DeepSeek 余额；无 key 时返回 `needs_auth`（前端可隐藏卡片）。
pub async fn refresh() -> UsageSnapshot {
    let key = match credentials::get_deepseek_key() {
        Ok(Some(k)) if !k.trim().is_empty() => k,
        Ok(_) => {
            return snapshot_base(
                ProviderStatus::NeedsAuth,
                Some("请在设置中配置 DeepSeek API Key".to_string()),
            );
        }
        Err(_) => {
            return snapshot_base(
                ProviderStatus::NeedsAuth,
                Some("无法读取 DeepSeek API Key".to_string()),
            );
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return snapshot_base(
                ProviderStatus::NetworkError,
                Some("无法创建网络客户端".to_string()),
            );
        }
    };

    let response = match client
        .get(BALANCE_URL)
        .header("Authorization", format!("Bearer {}", key.trim()))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return snapshot_base(
                ProviderStatus::NetworkError,
                Some("网络请求失败或超时".to_string()),
            );
        }
    };

    let status = response.status();
    let body = match response.text().await {
        Ok(b) => b,
        Err(_) => {
            return snapshot_base(
                ProviderStatus::NetworkError,
                Some("读取响应失败".to_string()),
            );
        }
    };

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return snapshot_base(
            ProviderStatus::NeedsAuth,
            Some("API Key 无效或已失效".to_string()),
        );
    }

    if !status.is_success() {
        return snapshot_base(
            ProviderStatus::NetworkError,
            Some(format!("服务返回 HTTP {}", status.as_u16())),
        );
    }

    match serde_json::from_str::<BalanceResponse>(&body) {
        Ok(parsed) => map_balance(parsed),
        Err(_) => snapshot_base(
            ProviderStatus::ParseError,
            Some("余额响应无法解析".to_string()),
        ),
    }
}

/// 离线映射（单测）。
pub(crate) fn map_balance_json(body: &str) -> Result<UsageSnapshot, serde_json::Error> {
    let parsed: BalanceResponse = serde_json::from_str(body)?;
    Ok(map_balance(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_official_balance_shape() {
        let body = r#"{
          "is_available": true,
          "balance_infos": [
            {
              "currency": "CNY",
              "total_balance": "12.50",
              "granted_balance": "2.50",
              "topped_up_balance": "10.00"
            }
          ]
        }"#;
        let snap = map_balance_json(body).expect("应可解析");
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.unit, UsageUnit::Balance);
        assert_eq!(snap.membership.as_deref(), Some("CNY"));
        assert_eq!(snap.remaining, Some(12.5));
        assert_eq!(snap.used, 10.0);
        assert_eq!(snap.on_demand_used, Some(2.5));
    }

    #[test]
    fn prefers_cny_over_usd() {
        let body = r#"{
          "is_available": true,
          "balance_infos": [
            {
              "currency": "USD",
              "total_balance": "1.00",
              "granted_balance": "0",
              "topped_up_balance": "1.00"
            },
            {
              "currency": "CNY",
              "total_balance": "9.99",
              "granted_balance": "0",
              "topped_up_balance": "9.99"
            }
          ]
        }"#;
        let snap = map_balance_json(body).expect("应可解析");
        assert_eq!(snap.membership.as_deref(), Some("CNY"));
        assert_eq!(snap.remaining, Some(9.99));
    }
}
