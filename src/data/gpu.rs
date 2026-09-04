use std::fs;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

use serde::Serialize;

const AMD_VENDOR_ID: &str = "0x1002";
const PERF_FORMAT_TOTAL_TIME_ENABLED: u64 = 1;
const PERF_FORMAT_TOTAL_TIME_RUNNING: u64 = 2;
const PERF_EVENT_ATTR_SIZE: u32 = 64;
// The i915 PMU rejects the all-CPUs sentinel for device-wide events.
const PERF_MONITOR_CPU: i32 = 0;

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

struct IntelPmu {
    counters: Vec<PerfCounter>,
    usage_percent: Option<f32>,
}

impl IntelPmu {
    fn discover() -> Option<Self> {
        let root = Path::new("/sys/bus/event_source/devices/i915");
        let pmu_type = read_trimmed(&root.join("type"))?.parse::<u32>().ok()?;
        let Ok(entries) = fs::read_dir(root.join("events")) else {
            return None;
        };
        let mut counters: Vec<PerfCounter> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with("-busy"))
            .filter_map(|entry| {
                let config = event_config(&entry.path())?;
                PerfCounter::new(pmu_type, config, &entry.path())
            })
            .collect();
        counters.sort_by(|a, b| a.name.cmp(&b.name));
        (!counters.is_empty()).then_some(Self {
            counters,
            usage_percent: None,
        })
    }

    fn refresh(&mut self) {
        let mut maximum = None;
        for counter in &mut self.counters {
            if let Some(percent) = counter.refresh() {
                maximum = Some(maximum.map_or(percent, |old: f32| old.max(percent)));
            }
        }
        self.usage_percent = maximum;
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

struct PerfCounter {
    fd: RawFd,
    name: String,
    previous: Option<CounterSample>,
}

#[derive(Clone, Copy)]
struct CounterSample {
    count: u64,
    running: u64,
}

impl PerfCounter {
    fn new(pmu_type: u32, config: u64, path: &Path) -> Option<Self> {
        let attr = PerfEventAttr::new(pmu_type, config);
        let fd = unsafe {
            libc::syscall(
                libc::SYS_perf_event_open,
                &attr,
                -1_i32,
                PERF_MONITOR_CPU,
                -1_i32,
                0_u64,
            ) as RawFd
        };
        (fd >= 0).then_some(Self {
            fd,
            name: path.file_name()?.to_string_lossy().into_owned(),
            previous: None,
        })
    }

    fn refresh(&mut self) -> Option<f32> {
        let current = read_counter(self.fd)?;
        let previous = self.previous.replace(current)?;
        delta_percent(previous, current)
    }
}

impl Drop for PerfCounter {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[repr(C)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    bp_addr: u64,
}

impl PerfEventAttr {
    fn new(type_: u32, config: u64) -> Self {
        Self {
            type_,
            size: PERF_EVENT_ATTR_SIZE,
            config,
            sample_period: 0,
            sample_type: 0,
            read_format: PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING,
            flags: 0,
            wakeup_events: 0,
            bp_type: 0,
            bp_addr: 0,
        }
    }
}

enum Backend {
    Amd(AmdDevice),
    Intel(IntelPmu),
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
        if let Some(intel) = IntelPmu::discover() {
            backends.push(Backend::Intel(intel));
        }
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

fn event_config(path: &Path) -> Option<u64> {
    let raw = read_trimmed(path)?;
    parse_event_config(&raw)
}

fn parse_event_config(raw: &str) -> Option<u64> {
    let value = raw
        .strip_prefix("event=")
        .or_else(|| raw.strip_prefix("config="))?;
    u64::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

fn read_counter(fd: RawFd) -> Option<CounterSample> {
    let mut values = [0_u64; 3];
    let bytes = unsafe {
        libc::read(
            fd,
            values.as_mut_ptr().cast(),
            std::mem::size_of_val(&values),
        )
    };
    (bytes == std::mem::size_of_val(&values) as isize).then_some(CounterSample {
        count: values[0],
        running: values[2],
    })
}

fn delta_percent(previous: CounterSample, current: CounterSample) -> Option<f32> {
    let count = current.count.checked_sub(previous.count)?;
    let running = current.running.checked_sub(previous.running)?;
    (running > 0).then(|| (count as f64 * 100.0 / running as f64).clamp(0.0, 100.0) as f32)
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
    fn event_config_reads_kernel_formats() {
        assert_eq!(parse_event_config("event=0x1234"), Some(0x1234));
        assert_eq!(parse_event_config("config=0x1234"), Some(0x1234));
        assert_eq!(parse_event_config("other=0x1234"), None);
    }

    #[test]
    fn counter_delta_becomes_usage() {
        let old = CounterSample {
            count: 100,
            running: 100,
        };
        let new = CounterSample {
            count: 150,
            running: 200,
        };
        assert_eq!(delta_percent(old, new), Some(50.0));
    }

    #[test]
    fn perf_attribute_has_kernel_v0_size() {
        assert_eq!(
            std::mem::size_of::<PerfEventAttr>(),
            PERF_EVENT_ATTR_SIZE as usize
        );
    }

    fn read_percent_from(raw: &str) -> Option<f32> {
        let value = raw.trim().parse::<f32>().ok()?;
        value.is_finite().then(|| value.clamp(0.0, 100.0))
    }
}
