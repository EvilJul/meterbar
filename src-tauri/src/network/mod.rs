//! 延迟探测与出口区域查询（§5）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::models::{AppSettings, LatencySnapshot, LatencyStatus};

/// 探测超时（秒）。
const PROBE_TIMEOUT_SECS: u64 = 5;
/// 区域缓存 TTL，避免频繁调用免费 API。
const REGION_CACHE_TTL: Duration = Duration::from_secs(300);
/// 请求头：部分免费 API 对无 UA 的客户端会 403 / 挑战页。
const USER_AGENT: &str = "Usages/0.1 (macOS; AI Usage Monitor)";
/// 主用：ipwho.is（无需 key，国内可达性优于 ipapi.co）。
const REGION_API_IPWHO: &str = "https://ipwho.is/";
/// 备用：ipinfo.io（无需 key；国家为 ISO 码）。
const REGION_API_IPINFO: &str = "https://ipinfo.io/json";
/// 区域查询失败时的短提示（避免面板裸「—」）。
const REGION_UNAVAILABLE: &str = "出口暂不可用";

static REGION_CACHE: Mutex<Option<RegionCache>> = Mutex::new(None);

struct RegionCache {
    label: String,
    fetched_at: Instant,
}

#[derive(Debug, Deserialize)]
struct IpWhoIsResponse {
    success: Option<bool>,
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IpInfoResponse {
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
}

/// 对目标发起 HTTPS 探测，返回延迟与状态；并附带出口区域（与延迟成败解耦）。
pub async fn probe(target: &str) -> LatencySnapshot {
    let fetched_at = Utc::now().to_rfc3339();
    let url = normalize_target(target);

    let client = match Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return latency_snapshot(
                url,
                None,
                LatencyStatus::Error,
                fetched_at,
                Some(REGION_UNAVAILABLE.to_string()),
            );
        }
    };

    let start = Instant::now();
    let (latency_ms, status) = match send_probe(&client, &url).await {
        ProbeOutcome::Ok => (Some(elapsed_ms(start)), LatencyStatus::Ok),
        ProbeOutcome::Timeout => (None, LatencyStatus::Timeout),
        ProbeOutcome::Error => (None, LatencyStatus::Error),
    };

    // 出口区域与延迟成败解耦：有缓存用缓存，否则始终尝试查询。
    let region_label = probe_region(&client).await;

    latency_snapshot(url, latency_ms, status, fetched_at, region_label)
}

fn latency_snapshot(
    target: String,
    latency_ms: Option<u64>,
    status: LatencyStatus,
    fetched_at: String,
    region_label: Option<String>,
) -> LatencySnapshot {
    LatencySnapshot {
        target,
        latency_ms,
        status,
        fetched_at,
        region_label,
    }
}

enum ProbeOutcome {
    Ok,
    Timeout,
    Error,
}

async fn send_probe(client: &Client, url: &str) -> ProbeOutcome {
    match client.head(url).send().await {
        Ok(resp) if resp.status() == StatusCode::METHOD_NOT_ALLOWED => {
            map_result(client.get(url).send().await)
        }
        Ok(_) => ProbeOutcome::Ok,
        Err(err) if err.is_timeout() => ProbeOutcome::Timeout,
        Err(_) => map_result(client.get(url).send().await),
    }
}

fn map_result(result: Result<reqwest::Response, reqwest::Error>) -> ProbeOutcome {
    match result {
        Ok(_) => ProbeOutcome::Ok,
        Err(err) if err.is_timeout() => ProbeOutcome::Timeout,
        Err(_) => ProbeOutcome::Error,
    }
}

/// 查询当前出口 IP 所在区域；失败返回短提示，成功结果缓存 5 分钟。
async fn probe_region(client: &Client) -> Option<String> {
    if let Some(label) = cached_region_label() {
        return Some(label);
    }

    if let Some(label) = fetch_region_ipwho(client).await {
        store_region_cache(&label);
        return Some(label);
    }

    if let Some(label) = fetch_region_ipinfo(client).await {
        store_region_cache(&label);
        return Some(label);
    }

    Some(REGION_UNAVAILABLE.to_string())
}

async fn fetch_region_ipwho(client: &Client) -> Option<String> {
    let resp = match client.get(REGION_API_IPWHO).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return None,
    };
    let body: IpWhoIsResponse = resp.json().await.ok()?;
    if body.success == Some(false) {
        return None;
    }
    let label = compose_region_label(
        body.city.as_deref(),
        body.region.as_deref(),
        body.country
            .as_deref()
            .or(body.country_code.as_deref()),
    );
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

async fn fetch_region_ipinfo(client: &Client) -> Option<String> {
    let resp = match client.get(REGION_API_IPINFO).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return None,
    };
    let body: IpInfoResponse = resp.json().await.ok()?;
    let label = compose_region_label(
        body.city.as_deref(),
        body.region.as_deref(),
        body.country.as_deref(),
    );
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

fn store_region_cache(label: &str) {
    if let Ok(mut guard) = REGION_CACHE.lock() {
        *guard = Some(RegionCache {
            label: label.to_string(),
            fetched_at: Instant::now(),
        });
    }
}

fn cached_region_label() -> Option<String> {
    let guard = REGION_CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    if cache.fetched_at.elapsed() <= REGION_CACHE_TTL {
        Some(cache.label.clone())
    } else {
        None
    }
}

fn compose_region_label(city: Option<&str>, region: Option<&str>, country: Option<&str>) -> String {
    let city = city.unwrap_or("").trim();
    let region = region.unwrap_or("").trim();
    let country = country.unwrap_or("").trim();

    match (city.is_empty(), region.is_empty(), country.is_empty()) {
        (false, _, false) if city.eq_ignore_ascii_case(region) || region.is_empty() => {
            format!("{city}, {country}")
        }
        (false, false, false) => format!("{city}, {region}, {country}"),
        (false, _, true) => city.to_string(),
        (true, false, false) => format!("{region}, {country}"),
        (true, true, false) => country.to_string(),
        _ => String::new(),
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn normalize_target(raw: &str) -> String {
    AppSettings::normalize_latency_target(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_city_country() {
        assert_eq!(
            compose_region_label(Some("Hong Kong"), Some("Hong Kong"), Some("HK")),
            "Hong Kong, HK"
        );
    }

    #[test]
    fn compose_city_region_country() {
        assert_eq!(
            compose_region_label(Some("Shenzhen"), Some("Guangdong"), Some("China")),
            "Shenzhen, Guangdong, China"
        );
    }

    #[test]
    fn compose_country_only() {
        assert_eq!(compose_region_label(None, None, Some("JP")), "JP");
    }
}
