//! 本机 CPU / 内存 / GPU 采集（§4）。

use chrono::Utc;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::models::SystemSnapshot;

/// 采集一次系统指标。GPU 尽力而为，不可用时为 `None`。
pub fn sample() -> SystemSnapshot {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );

    // sysinfo 需要两次 refresh 才能得到有意义的 CPU 利用率。
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let (gpu_percent, gpu_temp_c) = try_gpu_metrics();

    SystemSnapshot {
        cpu_percent: f64::from(sys.global_cpu_usage()),
        cpu_temp_c: None,
        gpu_percent,
        gpu_temp_c,
        mem_used_bytes: sys.used_memory(),
        mem_total_bytes: sys.total_memory(),
        fetched_at: Utc::now().to_rfc3339(),
    }
}

/// 尽力采集 GPU；解析失败时返回 `None`。
fn try_gpu_metrics() -> (Option<f64>, Option<f64>) {
    #[cfg(target_os = "macos")]
    {
        (macos_gpu_percent(), None)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (None, None)
    }
}

#[cfg(target_os = "macos")]
fn macos_gpu_percent() -> Option<f64> {
    let output = std::process::Command::new("ioreg")
        .args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_device_utilization(&text)
}

/// 从 `ioreg` 文本中解析 `"Device Utilization %"`；多设备时取最大值。
pub fn parse_device_utilization(text: &str) -> Option<f64> {
    const KEY: &str = "\"Device Utilization %\"";
    let mut best: Option<f64> = None;
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(KEY) {
        let idx = search_from + rel + KEY.len();
        let rest = text[idx..].trim_start();
        let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(v) = num.parse::<f64>() {
            best = Some(best.map_or(v, |b| b.max(v)));
        }
        search_from = idx;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::parse_device_utilization;

    #[test]
    fn parses_device_utilization_from_fixture() {
        let fixture = r#"
+-o AGXAcceleratorG16X
    {
      "PerformanceStatistics" = {"Tiler Utilization %"=12,"Renderer Utilization %"=8,"Device Utilization %"=44,"Allocated PB Size"=1}
      "model" = "Apple M4 Max"
    }
"#;
        assert_eq!(parse_device_utilization(fixture), Some(44.0));
    }

    #[test]
    fn takes_max_when_multiple_devices() {
        let fixture = r#""Device Utilization %"=10
"Device Utilization %"=55
"Device Utilization %"=3"#;
        assert_eq!(parse_device_utilization(fixture), Some(55.0));
    }

    #[test]
    fn returns_none_when_missing() {
        assert_eq!(parse_device_utilization("no gpu here"), None);
    }
}
