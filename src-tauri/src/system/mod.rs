//! 本机 CPU / 内存 / 磁盘 / GPU / VPN 采集（§4）。

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};

use crate::models::SystemSnapshot;

/// 采集一次完整系统指标（CPU/GPU + 内存/磁盘/VPN）。
pub fn sample() -> SystemSnapshot {
    let (cpu_percent, cpu_temp_c, gpu_percent, gpu_temp_c) = sample_cpu_gpu_fields();
    let (mem_used_bytes, mem_total_bytes, disk_used_bytes, disk_available_bytes, vpn_ip) =
        sample_slow_fields();

    SystemSnapshot {
        cpu_percent,
        cpu_temp_c,
        gpu_percent,
        gpu_temp_c,
        mem_used_bytes,
        mem_total_bytes,
        disk_used_bytes,
        disk_available_bytes,
        vpn_ip,
        fetched_at: Utc::now().to_rfc3339(),
    }
}

/// 快拍：仅 CPU + GPU（避免磁盘/ioreg 全量以外的慢路径；GPU 仍走 ioreg）。
pub fn sample_fast() -> SystemSnapshot {
    let (cpu_percent, cpu_temp_c, gpu_percent, gpu_temp_c) = sample_cpu_gpu_fields();
    SystemSnapshot {
        cpu_percent,
        cpu_temp_c,
        gpu_percent,
        gpu_temp_c,
        mem_used_bytes: 0,
        mem_total_bytes: 0,
        disk_used_bytes: None,
        disk_available_bytes: None,
        vpn_ip: None,
        fetched_at: Utc::now().to_rfc3339(),
    }
}

/// 慢拍：内存 / 磁盘 / VPN（不含 CPU 双采样与 GPU ioreg）。
pub fn sample_slow() -> SystemSnapshot {
    let (mem_used_bytes, mem_total_bytes, disk_used_bytes, disk_available_bytes, vpn_ip) =
        sample_slow_fields();
    SystemSnapshot {
        cpu_percent: 0.0,
        cpu_temp_c: None,
        gpu_percent: None,
        gpu_temp_c: None,
        mem_used_bytes,
        mem_total_bytes,
        disk_used_bytes,
        disk_available_bytes,
        vpn_ip,
        fetched_at: Utc::now().to_rfc3339(),
    }
}

fn sample_cpu_gpu_fields() -> (f64, Option<f64>, Option<f64>, Option<f64>) {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
    );

    // sysinfo 需要两次 refresh 才能得到有意义的 CPU 利用率。
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();

    let (gpu_percent, gpu_temp_c) = try_gpu_metrics();
    (
        f64::from(sys.global_cpu_usage()),
        None,
        gpu_percent,
        gpu_temp_c,
    )
}

fn sample_slow_fields() -> (u64, u64, Option<u64>, Option<u64>, Option<String>) {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_memory();

    let (disk_used_bytes, disk_available_bytes) = sample_root_disk();
    let vpn_ip = detect_vpn_ip();

    // macOS 上 sysinfo 的 used_memory 与 available_memory 并不互补（used+available≠total），
    // 且 used 偶发大于 total。展示「已用/剩余」时与磁盘一致：剩余=available，已用=total-available。
    let (mem_used, mem_total) =
        normalize_memory_bytes(sys.total_memory(), sys.available_memory());

    (
        mem_used,
        mem_total,
        disk_used_bytes,
        disk_available_bytes,
        vpn_ip,
    )
}

/// 将 sysinfo 的 total/available 规范为互补的（已用, 总量）。
/// 剩余语义取 available（钳制到 ≤ total）；已用 = total − available。
pub fn normalize_memory_bytes(total: u64, available: u64) -> (u64, u64) {
    if total == 0 {
        return (0, 0);
    }
    let available = available.min(total);
    let used = total.saturating_sub(available);
    (used, total)
}

/// 将磁盘 total/available 规范为互补的（已用, 剩余）。
/// 剩余 = available（钳制到 ≤ total）；已用 = total − available，保证已用+剩余=总量。
pub fn normalize_disk_bytes(total: u64, available: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let available = available.min(total);
    let used = total.saturating_sub(available);
    Some((used, available))
}

/// 启动盘候选挂载点（按优先级）。
/// macOS APFS：优先 Data 卷（用户数据/接近「关于本机 → 存储」），再回退 `/`。
pub fn preferred_disk_mounts() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &["/System/Volumes/Data", "/"]
    }
    #[cfg(not(target_os = "macos"))]
    {
        &["/"]
    }
}

/// 从候选盘中按优先级选主盘挂载点。
/// `mounts`：(挂载路径, 总字节, 是否可移除)。
pub fn select_primary_disk_mount<'a>(
    mounts: &[(&'a Path, u64, bool)],
) -> Option<&'a Path> {
    for pref in preferred_disk_mounts() {
        let pref = Path::new(pref);
        if let Some((mount, _, _)) = mounts
            .iter()
            .find(|(m, total, _)| *m == pref && *total > 0)
        {
            return Some(*mount);
        }
    }
    mounts
        .iter()
        .filter(|(_, total, removable)| *total > 0 && !*removable)
        .max_by_key(|(_, total, _)| *total)
        .or_else(|| {
            mounts
                .iter()
                .filter(|(_, total, _)| *total > 0)
                .max_by_key(|(_, total, _)| *total)
        })
        .map(|(m, _, _)| *m)
}

/// 用 statfs 读取**实际**可用空间（f_bavail），避免 sysinfo 在 macOS 上优先
/// `AvailableCapacityForImportantUsage`（把可清除缓存算进剩余）导致已用偏小。
#[cfg(unix)]
fn statfs_space(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
    let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    let buf = unsafe { buf.assume_init() };
    #[cfg(target_os = "macos")]
    let block = buf.f_bsize as u64;
    #[cfg(not(target_os = "macos"))]
    let block = {
        let fr = buf.f_frsize as u64;
        if fr > 0 {
            fr
        } else {
            buf.f_bsize as u64
        }
    };
    if block == 0 {
        return None;
    }
    let total = (buf.f_blocks as u64).saturating_mul(block);
    let available = (buf.f_bavail as u64).saturating_mul(block);
    Some((total, available))
}

/// 采集启动盘已用 / 实际剩余；失败时两者均为 `None`。
fn sample_root_disk() -> (Option<u64>, Option<u64>) {
    let mount = resolve_primary_disk_mount();
    let Some(mount) = mount else {
        return (None, None);
    };

    #[cfg(unix)]
    if let Some((total, available)) = statfs_space(&mount) {
        if let Some((used, available)) = normalize_disk_bytes(total, available) {
            return (Some(used), Some(available));
        }
    }

    // statfs 不可用时回退 sysinfo（注意：macOS 上 available 可能含 ImportantUsage）。
    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .find(|d| d.mount_point() == mount.as_path())
        .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()));
    let Some(disk) = disk else {
        return (None, None);
    };
    match normalize_disk_bytes(disk.total_space(), disk.available_space()) {
        Some((used, available)) => (Some(used), Some(available)),
        None => (None, None),
    }
}

fn resolve_primary_disk_mount() -> Option<PathBuf> {
    let disks = Disks::new_with_refreshed_list();
    let mounts: Vec<(&Path, u64, bool)> = disks
        .list()
        .iter()
        .map(|d| (d.mount_point(), d.total_space(), d.is_removable()))
        .collect();
    select_primary_disk_mount(&mounts).map(|p| p.to_path_buf()).or_else(|| {
        // sysinfo 未列出时，仍尝试平台优先路径（statfs 可直接读）。
        preferred_disk_mounts()
            .iter()
            .map(|p| PathBuf::from(p))
            .find(|p| p.exists())
    })
}

/// 从 utun / ipsec / ppp / tun / wg 等接口取首个可用 IPv4。
fn detect_vpn_ip() -> Option<String> {
    let networks = Networks::new_with_refreshed_list();
    let mut candidates: Vec<(u8, String)> = Vec::new();

    for (name, data) in networks.list() {
        let lower = name.to_ascii_lowercase();
        let priority = vpn_iface_priority(&lower)?;
        for net in data.ip_networks() {
            if let IpAddr::V4(v4) = net.addr {
                if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() {
                    continue;
                }
                candidates.push((priority, v4.to_string()));
            }
        }
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().map(|(_, ip)| ip).next()
}

fn vpn_iface_priority(name: &str) -> Option<u8> {
    if name.starts_with("utun") {
        Some(0)
    } else if name.starts_with("ipsec") {
        Some(1)
    } else if name.starts_with("ppp") {
        Some(2)
    } else if name.starts_with("wg") {
        Some(3)
    } else if name.starts_with("tun") {
        Some(4)
    } else if name.contains("vpn") {
        Some(5)
    } else {
        None
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
    use std::path::Path;

    use super::{
        normalize_disk_bytes, normalize_memory_bytes, parse_device_utilization,
        preferred_disk_mounts, select_primary_disk_mount, vpn_iface_priority,
    };

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn memory_used_plus_available_equals_total() {
        let total = 128 * GIB;
        let available = 44 * GIB;
        let (used, out_total) = normalize_memory_bytes(total, available);
        assert_eq!(out_total, total);
        assert_eq!(used, 84 * GIB);
        assert_eq!(used + available, total);
    }

    #[test]
    fn memory_available_clamped_when_above_total() {
        let total = 16 * GIB;
        let (used, out_total) = normalize_memory_bytes(total, 20 * GIB);
        assert_eq!(out_total, total);
        assert_eq!(used, 0);
        // available 被钳为 total → 剩余=total，已用=0
        assert_eq!(used + out_total, total);
    }

    #[test]
    fn memory_zero_total_is_zero() {
        assert_eq!(normalize_memory_bytes(0, 8 * GIB), (0, 0));
    }

    #[test]
    fn disk_used_plus_available_equals_total() {
        let total = 1858 * GIB;
        // ImportantUsage 式虚高剩余会压低已用；这里用实际剩余语义
        let available = 1177 * GIB;
        let (used, rem) = normalize_disk_bytes(total, available).expect("total>0");
        assert_eq!(used, total - available);
        assert_eq!(rem, available);
        assert_eq!(used + rem, total);
    }

    #[test]
    fn disk_available_clamped_when_above_total() {
        let total = 100 * GIB;
        let (used, rem) = normalize_disk_bytes(total, 150 * GIB).expect("total>0");
        assert_eq!(used, 0);
        assert_eq!(rem, total);
    }

    #[test]
    fn disk_zero_total_is_none() {
        assert_eq!(normalize_disk_bytes(0, 8 * GIB), None);
    }

    #[test]
    fn selects_data_volume_before_root_on_macos() {
        let data = Path::new("/System/Volumes/Data");
        let root = Path::new("/");
        let tiny = Path::new("/Volumes/USB");
        let mounts = [
            (root, 1858 * GIB, false),
            (tiny, 16 * GIB, true),
            (data, 1858 * GIB, false),
        ];
        let picked = select_primary_disk_mount(&mounts).expect("should pick");
        if preferred_disk_mounts()
            .iter()
            .any(|p| *p == "/System/Volumes/Data")
        {
            assert_eq!(picked, data);
        } else {
            assert_eq!(picked, root);
        }
    }

    #[test]
    fn selects_largest_internal_when_preferred_missing() {
        let a = Path::new("/mnt/a");
        let b = Path::new("/mnt/b");
        let usb = Path::new("/mnt/usb");
        let mounts = [
            (a, 100 * GIB, false),
            (b, 500 * GIB, false),
            (usb, 2000 * GIB, true),
        ];
        assert_eq!(select_primary_disk_mount(&mounts), Some(b));
    }

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

    #[test]
    fn vpn_iface_priority_matches_tunnel_names() {
        assert_eq!(vpn_iface_priority("utun3"), Some(0));
        assert_eq!(vpn_iface_priority("ipsec0"), Some(1));
        assert_eq!(vpn_iface_priority("en0"), None);
    }

    #[test]
    fn live_memory_sample_used_le_total() {
        let snap = super::sample();
        assert!(
            snap.mem_used_bytes <= snap.mem_total_bytes,
            "used {} > total {}",
            snap.mem_used_bytes,
            snap.mem_total_bytes
        );
        if snap.mem_total_bytes > 0 {
            let remaining = snap.mem_total_bytes - snap.mem_used_bytes;
            assert_eq!(snap.mem_used_bytes + remaining, snap.mem_total_bytes);
            eprintln!(
                "live mem: used={:.1}GiB remaining={:.1}GiB total={:.1}GiB",
                snap.mem_used_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                remaining as f64 / (1024.0 * 1024.0 * 1024.0),
                snap.mem_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
            );
        }
    }

    #[test]
    fn live_disk_sample_used_plus_remaining_equals_total() {
        let snap = super::sample();
        let (Some(used), Some(available)) = (snap.disk_used_bytes, snap.disk_available_bytes)
        else {
            eprintln!("live disk: unavailable");
            return;
        };
        let total = used + available;
        assert!(total > 0, "disk total should be > 0");
        let gib = 1024.0 * 1024.0 * 1024.0;
        eprintln!(
            "live disk: used={:.1}GiB remaining={:.1}GiB total={:.1}GiB ({:.1}%)",
            used as f64 / gib,
            available as f64 / gib,
            total as f64 / gib,
            (used as f64 / total as f64) * 100.0,
        );

        // 应对齐主挂载点的 statfs 实际可用，而不是 sysinfo ImportantUsage。
        #[cfg(unix)]
        {
            let mount = super::resolve_primary_disk_mount().expect("应能解析主盘挂载点");
            let (fs_total, fs_avail) =
                super::statfs_space(&mount).expect("statfs 应可读主盘");
            let (exp_used, exp_avail) =
                normalize_disk_bytes(fs_total, fs_avail).expect("total>0");
            assert_eq!(used, exp_used, "disk used 应来自 statfs total−bavail");
            assert_eq!(available, exp_avail, "disk remaining 应为 statfs 实际剩余");
        }
    }
}
