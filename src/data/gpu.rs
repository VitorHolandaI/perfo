use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

const AMD_VENDOR_ID: &str = "0x1002";
const INTEL_VENDOR_ID: &str = "0x8086";
const DRM_ENGINE_COUNT: usize = 5;

#[derive(Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub usage_percent: Option<f32>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
}

#[derive(Clone, Default, Serialize)]
pub struct GpuSnapshot {
    pub devices: Vec<GpuInfo>,
}

struct AmdDevice {
    path: PathBuf,
    name: String,
}

impl AmdDevice {
    fn discover() -> Vec<Self> {
        let Ok(entries) = fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        let mut devices: Vec<Self> = entries
            .filter_map(Result::ok)
            .filter(|entry| is_drm_card(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| {
                let path = entry.path().join("device");
                if read_trimmed(&path.join("vendor")).as_deref() != Some(AMD_VENDOR_ID) {
                    return None;
                }
                let name = read_trimmed(&path.join("product_name"))
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
                Some(Self { path, name })
            })
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }

    fn snapshot(&self) -> GpuInfo {
        GpuInfo {
            name: self.name.clone(),
            vendor: "AMD".into(),
            usage_percent: read_percent(&self.path.join("gpu_busy_percent")),
            memory_used_bytes: read_u64(&self.path.join("mem_info_vram_used")),
            memory_total_bytes: read_u64(&self.path.join("mem_info_vram_total")),
            temperature_c: hwmon_temperature(&self.path),
            power_w: read_u64(&self.path.join("power1_average")).map(|u| u as f32 / 1_000_000.0),
        }
    }
}

struct IntelDrm {
    pdev: String,
    previous: Option<IntelEngineTimes>,
    sampled_at: Option<Instant>,
    usage_percent: Option<f32>,
}

impl IntelDrm {
    fn discover() -> Vec<Self> {
        let Ok(entries) = fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| is_drm_card(&entry.file_name().to_string_lossy()))
            .filter_map(intel_device_from_entry)
            .collect()
    }

    fn refresh(&mut self) {
        let now = Instant::now();
        let current = read_drm_engine_times(&self.pdev);
        self.usage_percent =
            self.previous
                .zip(self.sampled_at)
                .and_then(|(previous, sampled_at)| {
                    engine_usage_percent(
                        previous,
                        current,
                        now.duration_since(sampled_at).as_secs_f32(),
                    )
                });
        self.previous = Some(current);
        self.sampled_at = Some(now);
    }

    fn snapshot(&self) -> GpuInfo {
        GpuInfo {
            name: "Intel GPU".into(),
            vendor: "Intel".into(),
            usage_percent: self.usage_percent,
            memory_used_bytes: None,
            memory_total_bytes: None,
            temperature_c: None,
            power_w: None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct IntelEngineTimes {
    values: [u64; DRM_ENGINE_COUNT],
}

impl IntelEngineTimes {
    fn add_assign(&mut self, other: Self) {
        for (total, value) in self.values.iter_mut().zip(other.values) {
            *total = total.saturating_add(value);
        }
    }
}

#[derive(Clone, Copy)]
struct IntelClientSample {
    client_id: u64,
    engines: IntelEngineTimes,
}

fn intel_device_from_entry(entry: fs::DirEntry) -> Option<IntelDrm> {
    let device = entry.path().join("device");
    let raw = read_trimmed(&device.join("uevent"))?;
    if uevent_value(&raw, "DRIVER") != Some("i915") {
        return None;
    }
    let pdev = uevent_value(&raw, "PCI_SLOT_NAME")?.to_owned();
    if read_trimmed(&device.join("vendor")).as_deref() != Some(INTEL_VENDOR_ID) {
        return None;
    }
    Some(IntelDrm {
        pdev,
        previous: None,
        sampled_at: None,
        usage_percent: None,
    })
}

fn uevent_value<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    raw.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

fn read_drm_engine_times(target_pdev: &str) -> IntelEngineTimes {
    let mut totals = IntelEngineTimes::default();
    let mut clients = HashSet::new();
    let Ok(processes) = fs::read_dir("/proc") else {
        return totals;
    };
    for process in processes.flatten() {
        accumulate_process_engine_times(&process.path(), target_pdev, &mut clients, &mut totals);
    }
    totals
}

fn accumulate_process_engine_times(
    process_path: &Path,
    target_pdev: &str,
    clients: &mut HashSet<u64>,
    totals: &mut IntelEngineTimes,
) {
    if process_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.parse::<u32>().ok())
        .is_none()
    {
        return;
    }
    let Ok(file_descriptors) = fs::read_dir(process_path.join("fdinfo")) else {
        return;
    };
    for file_descriptor in file_descriptors.flatten() {
        accumulate_fd_engine_times(&file_descriptor.path(), target_pdev, clients, totals);
    }
}

fn accumulate_fd_engine_times(
    fdinfo_path: &Path,
    target_pdev: &str,
    clients: &mut HashSet<u64>,
    totals: &mut IntelEngineTimes,
) {
    let Ok(raw) = fs::read_to_string(fdinfo_path) else {
        return;
    };
    let Some(sample) = parse_drm_fdinfo(&raw, target_pdev) else {
        return;
    };
    if clients.insert(sample.client_id) {
        totals.add_assign(sample.engines);
    }
}

fn parse_drm_fdinfo(raw: &str, target_pdev: &str) -> Option<IntelClientSample> {
    let mut pdev = None;
    let mut client_id = None;
    let mut engines = IntelEngineTimes::default();
    for line in raw.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "drm-pdev" => pdev = Some(value),
            "drm-client-id" => client_id = value.parse().ok(),
            _ => {
                if let Some((index, time_ns)) = drm_engine_value(key, value) {
                    engines.values[index] = time_ns;
                }
            }
        }
    }
    (pdev == Some(target_pdev)).then_some(IntelClientSample {
        client_id: client_id?,
        engines,
    })
}

fn drm_engine_value(key: &str, value: &str) -> Option<(usize, u64)> {
    let index = match key {
        "drm-engine-render" => 0,
        "drm-engine-copy" => 1,
        "drm-engine-video" => 2,
        "drm-engine-video-enhance" => 3,
        "drm-engine-compute" => 4,
        _ => return None,
    };
    Some((index, value.strip_suffix(" ns")?.parse().ok()?))
}

fn engine_usage_percent(
    previous: IntelEngineTimes,
    current: IntelEngineTimes,
    elapsed_seconds: f32,
) -> Option<f32> {
    if elapsed_seconds <= 0.0 {
        return None;
    }
    let max_delta = previous
        .values
        .into_iter()
        .zip(current.values)
        .map(|(old, new)| new.saturating_sub(old))
        .max()
        .unwrap_or(0);
    Some(
        (max_delta as f64 / (elapsed_seconds as f64 * 1_000_000_000.0) * 100.0).clamp(0.0, 100.0)
            as f32,
    )
}

enum Backend {
    Amd(AmdDevice),
    Intel(IntelDrm),
}

pub struct GpuMonitor {
    backends: Vec<Backend>,
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuMonitor {
    pub fn new() -> Self {
        let mut backends: Vec<Backend> = AmdDevice::discover()
            .into_iter()
            .map(Backend::Amd)
            .collect();
        backends.extend(IntelDrm::discover().into_iter().map(Backend::Intel));
        Self { backends }
    }

    pub fn refresh(&mut self) {
        for backend in &mut self.backends {
            if let Backend::Intel(pmu) = backend {
                pmu.refresh();
            }
        }
    }

    pub fn snapshot(&self) -> GpuSnapshot {
        GpuSnapshot {
            devices: self
                .backends
                .iter()
                .map(|backend| match backend {
                    Backend::Amd(device) => device.snapshot(),
                    Backend::Intel(pmu) => pmu.snapshot(),
                })
                .collect(),
        }
    }
}

fn is_drm_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
    })
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().into())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn read_percent(path: &Path) -> Option<f32> {
    let value = read_trimmed(path)?.parse::<f32>().ok()?;
    value.is_finite().then(|| value.clamp(0.0, 100.0))
}

fn hwmon_temperature(device: &Path) -> Option<f32> {
    let hwmon = device.join("hwmon");
    let entries = fs::read_dir(hwmon).ok()?;
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| fs::read_dir(entry.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("temp"))
        .filter_map(|entry| read_u64(&entry.path()))
        .map(|millidegrees| millidegrees as f32 / 1000.0)
        .max_by(|a, b| a.total_cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drm_card_names_are_strict() {
        assert!(is_drm_card("card0"));
        assert!(is_drm_card("card12"));
        assert!(!is_drm_card("card"));
        assert!(!is_drm_card("card0-HDMI-A-1"));
    }

    #[test]
    fn percent_values_are_clamped() {
        assert_eq!(read_percent_from("55.5"), Some(55.5));
        assert_eq!(read_percent_from("150"), Some(100.0));
        assert_eq!(read_percent_from("bad"), None);
    }

    #[test]
    fn drm_fdinfo_reads_i915_engine_times() {
        let raw = "drm-client-id: 42\ndrm-pdev: 0000:00:02.0\ndrm-engine-render: 123 ns\ndrm-engine-copy: 9 ns\ndrm-engine-capacity-video: 2\n";
        let sample = parse_drm_fdinfo(raw, "0000:00:02.0").expect("i915 fdinfo");
        assert_eq!(sample.client_id, 42);
        assert_eq!(sample.engines.values, [123, 9, 0, 0, 0]);
    }

    #[test]
    fn drm_fdinfo_rejects_other_devices() {
        let raw = "drm-client-id: 42\ndrm-pdev: 0000:00:03.0\n";
        assert!(parse_drm_fdinfo(raw, "0000:00:02.0").is_none());
    }

    #[test]
    fn engine_usage_uses_the_busiest_engine() {
        let previous = IntelEngineTimes {
            values: [100, 200, 300, 400, 500],
        };
        let current = IntelEngineTimes {
            values: [200, 300, 400, 500, 600],
        };
        assert_eq!(engine_usage_percent(previous, current, 1.0), Some(0.00001));
    }

    fn read_percent_from(raw: &str) -> Option<f32> {
        let value = raw.trim().parse::<f32>().ok()?;
        value.is_finite().then(|| value.clamp(0.0, 100.0))
    }
}
