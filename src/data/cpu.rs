use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{CpuRefreshKind, Components, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

/// /proc/<pid>/stat: field 39 (last processor) is the 36th whitespace token
/// after the closing `)` of the comm field.
const LAST_CPU_STAT_FIELD: usize = 36;
/// Max frequency below which a shared-L2 core is classified low-power (MHz).
const LPE_MAX_FREQ_MHZ: u64 = 3000;
/// cpuinfo_max_freq is exposed in kHz; our numbers are MHz.
const KHZ_PER_MHZ: u64 = 1000;

#[derive(Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: Option<u32>,
    /// Owning process pid when this entry is a thread (None for real processes).
    pub owner: Option<u32>,
    pub is_kernel: bool,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_bytes: u64,
    pub cmd: String,
    /// Index of the CPU this process last ran on (from /proc/<pid>/stat field 39).
    pub last_cpu: Option<u32>,
    /// Bytes actually reaching the storage layer per second (from
    /// /proc/<pid>/io read_bytes/write_bytes deltas). Zero when unreadable
    /// (yama/other-user) or idle.
    pub read_bps: u64,
    pub write_bps: u64,
}

/// Parses /proc/<pid>/io: (read_bytes, write_bytes) — bytes that actually
/// reached the storage layer (submit_bio), unlike rchar/wchar which count
/// syscall bytes including page-cache hits.
fn proc_io_from(raw: &str) -> (u64, u64) {
    let mut out = (0, 0);
    for line in raw.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else { continue };
        let Some(v) = val.parse::<u64>().ok() else { continue };
        match key {
            "read_bytes:" => out.0 = v,
            "write_bytes:" => out.1 = v,
            _ => {}
        }
    }
    out
}

fn proc_io_of(pid: u32) -> Option<(u64, u64)> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    Some(proc_io_from(&raw))
}

/// (iowait_ms, total_ms) from the aggregate "cpu " line of /proc/stat.
fn stat_iowait_from(raw: &str) -> (u64, u64) {
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            let nums: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if nums.len() >= 5 {
                // user nice system idle iowait irq ...
                let total: u64 = nums.iter().sum();
                return (nums[4], total);
            }
        }
    }
    (0, 0)
}

/// Last-run CPU from a /proc/<pid>/stat line. Field 39 (processor index) is
/// the 36th whitespace token after the closing `)` of the comm field (which
/// may itself contain spaces and parens).
fn last_cpu_from_stat(raw: &str) -> Option<u32> {
    let close = raw.rfind(')')?;
    raw[close + 1..].split_whitespace().nth(LAST_CPU_STAT_FIELD)?.parse().ok()
}

fn last_cpu_of(pid: u32) -> Option<u32> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    last_cpu_from_stat(&raw)
}

use crate::data::disk::{DiskInfo, DiskMonitor};
use crate::data::mem::{self, MemSnapshot};

#[derive(Serialize)]
pub struct CpuSnapshot {
    pub overall_percent: f32,
    /// % of time CPUs spent waiting on disk I/O (from /proc/stat iowait).
    pub iowait_percent: f32,
    pub per_core: Vec<f32>,
    pub core_count: usize,
    pub per_core_types: Vec<CoreType>,
    pub per_core_freq_mhz: Vec<u64>,
    pub per_core_max_freq_mhz: Vec<u64>,
    pub per_core_temp_c: Vec<Option<f32>>,
    pub cpu_temp_c: Option<f32>,
    pub load_avg: [f64; 3],
    pub total_mem_bytes: u64,
    pub used_mem_bytes: u64,
    pub mem: MemSnapshot,
    /// PSI I/O pressure "some" (10s/60s/300s) from /proc/pressure/io.
    pub io_pressure_some: [f64; 3],
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
pub enum CoreType {
    /// Performance core: private L2.
    P,
    /// Efficient core: shares L2 with a cluster.
    E,
    /// Low-power efficient core: shared L2, much lower max frequency.
    Lpe,
    /// Reserved for hardware shapes the heuristics cannot classify; serialized
    /// to JSON so the widget can render it instead of crashing.
    #[allow(dead_code)]
    Unknown,
}

impl CoreType {
    pub fn letter(self) -> char {
        match self {
            CoreType::P => 'P',
            CoreType::E => 'E',
            CoreType::Lpe => 'L',
            CoreType::Unknown => '?',
        }
    }
}

fn cpu_count() -> usize {
    std::fs::read_dir("/sys/devices/system/cpu")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    n.starts_with("cpu") && n[3..].chars().all(|c| c.is_ascii_digit())
                })
                .count()
        })
        .unwrap_or(0)
}

/// Parses a cpumask-style "a,b-d" list into individual cpu indexes.
fn shared_l2_from_list(raw: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in raw.trim().split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = a.trim().parse().unwrap_or(0);
            let b: usize = b.trim().parse().unwrap_or(a);
            for c in a..=b {
                out.push(c);
            }
        } else if let Ok(c) = part.trim().parse::<usize>() {
            out.push(c);
        }
    }
    out
}

/// CPUs sharing this core's L2 (reads the kernel's shared_cpu_list).
fn shared_l2_cpus(i: usize) -> Vec<usize> {
    let path = format!("/sys/devices/system/cpu/cpu{i}/cache/index2/shared_cpu_list");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    shared_l2_from_list(&raw)
}

/// core_id of cpu N (for mapping coretemp "Core N" labels to cpu indexes).
fn core_id_of(cpu: usize) -> Option<u32> {
    let path = format!("/sys/devices/system/cpu/cpu{cpu}/topology/core_id");
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

/// Per-cpu temperature from coretemp/hwmon components, matched by core_id.
fn per_core_temps(components: &Components) -> Vec<Option<f32>> {
    let mut by_core: HashMap<u32, f32> = HashMap::new();
    for c in components.list() {
        let label = c.label();
        // sysinfo prefixes labels with the hwmon name, e.g. "coretemp Core 0".
        let short = label.strip_prefix("coretemp ").unwrap_or(label);
        if let Some(id) = short
            .strip_prefix("Core")
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            if let Some(t) = c.temperature() {
                by_core.insert(id, t);
            }
        }
    }
    (0..cpu_count())
        .map(|i| core_id_of(i).and_then(|id| by_core.get(&id).copied()))
        .collect()
}

/// Max frequency of each cpu (MHz) from cpufreq sysfs (source is kHz).
fn per_core_max_freqs() -> Vec<u64> {
    (0..cpu_count())
        .map(|i| {
            std::fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{i}/cpufreq/cpuinfo_max_freq"
            ))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|khz| khz / KHZ_PER_MHZ)
            .unwrap_or(0)
        })
        .collect()
}

/// Hybrid-core classification: private L2 => P-core; shared L2 + low max
/// frequency (< LPE_MAX_FREQ_MHZ) => low-power E-core; shared L2 otherwise
/// => E-core.
fn core_type_of(shares_l2_with_others: bool, max_freq_mhz: u64) -> CoreType {
    if !shares_l2_with_others {
        return CoreType::P;
    }
    if max_freq_mhz < LPE_MAX_FREQ_MHZ {
        CoreType::Lpe
    } else {
        CoreType::E
    }
}

fn core_types_of() -> Vec<CoreType> {
    let max_freqs = per_core_max_freqs();
    (0..cpu_count())
        .map(|i| {
            let others = shared_l2_cpus(i).iter().filter(|&&c| c != i).count();
            core_type_of(others > 0, max_freqs.get(i).copied().unwrap_or(0))
        })
        .collect()
}

/// Samples CPU + process data via sysinfo.
///
/// CPU usage is a delta over the time between two refreshes, so callers must
/// space `refresh()` calls about 1s apart (see `wait_sample_interval`). The
/// first `refresh()` after construction seeds the deltas and should be ignored.
pub struct CpuMonitor {
    sys: System,
    components: Components,
    disks: DiskMonitor,
    users_cache: HashMap<u32, String>,
    /// pid -> last-run CPU, refreshed only on full process refreshes.
    last_cpu: HashMap<u32, u32>,
    /// pid -> (read_bytes, write_bytes) from the previous full refresh.
    io_prev: HashMap<u32, (u64, u64)>,
    /// pid -> bytes/second, computed on full refreshes.
    io_rates: HashMap<u32, (u64, u64)>,
    /// (iowait, total) counters from the previous /proc/stat read.
    stat_prev: (u64, u64),
    /// When the last full refresh happened (drives I/O rate conversion).
    last_full: Option<Instant>,
}

/// Resolve a uid to a username via NSS (getpwuid_r), "?" on failure.
///
/// `users` crate (0.11) has RUSTSEC-2023-0059 (unsound) and is
/// unmaintained — this drops that dependency entirely.
fn user_name_of(uid: u32) -> String {
    let mut buf = [0u8; 256];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: buf lives for the call, getpwuid_r fills passwd/buf and stores
    // the resolved entry in `result`; return value 0 = success.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr() as *mut i8,
            buf.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
        // SAFETY: pw_name points into buf, NUL-terminated by getpwuid_r.
        unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned()
    } else {
        "?".into()
    }
}

impl CpuMonitor {
    pub fn new() -> Self {
        // new_all() does a heavyweight first refresh (disks, net, users,
        // processes with everything); build only CPU + memory here and let
        // do_refresh handle the rest, so startup stays cheap.
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let mut monitor = Self {
            sys,
            components: Components::new_with_refreshed_list(),
            disks: DiskMonitor::new(),
            users_cache: HashMap::new(),
            last_cpu: HashMap::new(),
            io_prev: HashMap::new(),
            io_rates: HashMap::new(),
            stat_prev: (0, 0),
            last_full: None,
        };
        // Seed CPU deltas so the first real refresh has a window to measure.
        monitor.refresh();
        monitor
    }

    fn do_refresh(sys: &mut System) {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_user(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_tasks(),
        );
    }

    /// Full refresh: CPU + memory + all process stats + last-run CPU map.
    /// Expensive (tens of thousands of /proc reads); call sparingly (~every 2s).
    pub fn refresh(&mut self) {
        Self::do_refresh(&mut self.sys);
        self.disks.refresh();
        let now = Instant::now();
        let elapsed = self.last_full.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        let mut m = HashMap::with_capacity(self.sys.processes().len());
        let mut io_cur: HashMap<u32, (u64, u64)> = HashMap::with_capacity(self.sys.processes().len());
        let mut io_rates: HashMap<u32, (u64, u64)> = HashMap::with_capacity(self.sys.processes().len());
        for (pid, _) in self.sys.processes() {
            let pid = pid.as_u32();
            if let Some(c) = last_cpu_of(pid) {
                m.insert(pid, c);
            }
            // /proc/<pid>/io is one tiny file per process; the page cache
            // keeps the read cheap (~µs) once warm.
            if let Some((rb, wb)) = proc_io_of(pid) {
                if let Some((prb, pwb)) = self.io_prev.get(&pid) {
                    io_rates.insert(pid, (rate_bps(rb.saturating_sub(*prb), elapsed), rate_bps(wb.saturating_sub(*pwb), elapsed)));
                }
                io_cur.insert(pid, (rb, wb));
            }
        }
        self.last_cpu = m;
        self.io_prev = io_cur;
        self.io_rates = io_rates;
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let (iw, tot) = stat_iowait_from(&stat);
        self.stat_prev = (iw, tot);
        self.last_full = Some(now);
    }

    /// Cheap refresh: CPU + memory + temperatures only. Process data stays at
    /// the last full refresh; use between full refreshes so the bars stay smooth.
    pub fn refresh_light(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.components.refresh(false);
    }


pub fn snapshot(&mut self) -> CpuSnapshot {
        // On Linux every thread appears as its own /proc entry; map each
        // thread tid to its owning process via Process::tasks().
        let mut task_of: HashMap<u32, u32> = HashMap::new();
        for (pid, p) in self.sys.processes() {
            if let Some(tids) = p.tasks() {
                for t in tids {
                    task_of.insert(t.as_u32(), pid.as_u32());
                }
            }
        }

        let overall_percent = self.sys.global_cpu_usage();
        // iowait needs a delta between two /proc/stat samples; stat_prev
        // holds the latest, so the first snapshot reports 0.
        let (iw, tot) = self.stat_prev;
        let iowait_percent = if tot > 0 {
            (iw as f64 / tot as f64 * 100.0) as f32
        } else {
            0.0
        };
        let per_core: Vec<f32> = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let per_core_types = core_types_of();
        let per_core_freq_mhz: Vec<u64> = self.sys.cpus().iter().map(|c| c.frequency()).collect();
        let per_core_max_freq_mhz = per_core_max_freqs();
        let cpu_temp_c: Option<f32> = self
            .components
            .list()
            .iter()
            .filter(|c| {
                let l = c.label();
                let short = l.strip_prefix("coretemp ").unwrap_or(l);
                short.contains("Package") || short.starts_with("Core") || short.contains("PECI")
            })
            .filter_map(|c| c.temperature())
            .max_by(|a, b| a.total_cmp(b));
        let per_core_temp_c = per_core_temps(&self.components);
        let la = sysinfo::System::load_average();
        let load_avg = [la.one, la.five, la.fifteen];

        // Username lookups hit NSS; cache them per uid instead of resolving
        // every process every tick.
        let mut cache = std::mem::take(&mut self.users_cache);
        let mut processes: Vec<ProcessInfo> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| {
                let pid_u = pid.as_u32();
                let owner = task_of.get(&pid_u).copied();
                let user = match p.user_id().map(|u| **u) {
                    Some(uid) => match cache.get(&uid) {
                        Some(n) => n.clone(),
                        None => {
                            let n = user_name_of(uid);
                            cache.insert(uid, n.clone());
                            n
                        }
                    },
                    None => "?".into(),
                };
                let cmd = if p.cmd().is_empty() {
                    p.name().to_string_lossy().into_owned()
                } else {
                    p.cmd()
                        .iter()
                        .map(|s| s.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let (read_bps, write_bps) = self.io_rates.get(&pid_u).copied().unwrap_or((0, 0));
                ProcessInfo {
                    pid: pid_u,
                    ppid: p.parent().map(|pp| pp.as_u32()),
                    owner,
                    is_kernel: pid_u == 2 || p.parent().map(|pp| pp.as_u32()) == Some(2),
                    user,
                    cpu_percent: p.cpu_usage(),
                    mem_bytes: p.memory(),
                    cmd,
                    last_cpu: self.last_cpu.get(&pid_u).copied(),
                    read_bps,
                    write_bps,
                }
            })
            .collect();
        self.users_cache = cache;
        processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));

        CpuSnapshot {
            overall_percent,
            iowait_percent,
            per_core,
            core_count: self.sys.cpus().len(),
            per_core_types,
            per_core_freq_mhz,
            per_core_max_freq_mhz,
            per_core_temp_c,
            cpu_temp_c,
            load_avg,
            total_mem_bytes: self.sys.total_memory(),
            used_mem_bytes: self.sys.used_memory(),
            mem: mem::snapshot(),
            io_pressure_some: self.disks.io_pressure(),
            disks: self.disks.snapshot(),
            processes,
        }
    }
}

/// Bytes per second from a refresh-interval delta.
fn rate_bps(delta: u64, elapsed_secs: f32) -> u64 {
    if elapsed_secs <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed_secs as f64) as u64
    }
}

/// Sleep long enough for a fresh CPU/process delta to accumulate.
pub fn wait_sample_interval() {
    std::thread::sleep(Duration::from_millis(1000));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_cpu_from_stat_reads_field_39() {
        let stat = "123 (my app (worker) v2) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 7 38";
        assert_eq!(last_cpu_from_stat(stat), Some(7));
    }

    #[test]
    fn last_cpu_from_stat_requires_close_paren() {
        assert_eq!(last_cpu_from_stat("no parens here"), None);
    }

    #[test]
    fn last_cpu_from_stat_short_line_is_none() {
        assert_eq!(last_cpu_from_stat("1 (init) S 1"), None);
    }

    #[test]
    fn shared_l2_parses_single() {
        assert_eq!(shared_l2_from_list("0,1"), vec![0, 1]);
    }

    #[test]
    fn shared_l2_parses_range() {
        assert_eq!(shared_l2_from_list("8-11"), vec![8, 9, 10, 11]);
    }

    #[test]
    fn shared_l2_parses_mixed() {
        assert_eq!(shared_l2_from_list("4-6,12"), vec![4, 5, 6, 12]);
    }

    #[test]
    fn core_type_classification() {
        assert_eq!(core_type_of(false, 4900), CoreType::P);
        assert_eq!(core_type_of(true, 2500), CoreType::Lpe);
        assert_eq!(core_type_of(true, 4400), CoreType::E);
        assert_eq!(core_type_of(false, 0), CoreType::P);
    }

    #[test]
    fn proc_io_parses_storage_bytes() {
        let raw = "rchar: 1000\nwchar: 2000\nsyscr: 5\nsyscw: 7\nread_bytes: 4096000\nwrite_bytes: 2048000\ncancelled_write_bytes: 0\n";
        assert_eq!(proc_io_from(raw), (4096000, 2048000));
    }

    #[test]
    fn proc_io_missing_fields_are_zero() {
        assert_eq!(proc_io_from("rchar: 1\n"), (0, 0));
    }

    #[test]
    fn stat_iowait_parses_cpu_line() {
        let raw = "cpu  100 50 200 1000 40 30 20 10 5 2\ncpu0 1 2 3 4 5 6 7 8 9 1\n";
        assert_eq!(stat_iowait_from(raw), (40, 1457));
    }

    #[test]
    fn stat_iowait_missing_line_is_zero() {
        assert_eq!(stat_iowait_from("cpu0 1 2 3 4 5\n"), (0, 0));
    }

    #[test]
    fn rate_bps_converts_delta() {
        assert_eq!(rate_bps(1_000_000, 2.0), 500_000);
        assert_eq!(rate_bps(100, 0.0), 0);
    }

    #[test]
    fn user_name_of_resolves_root() {
        // root exists in every passwd; the NSS lookup must not return "?".
        let name = user_name_of(0);
        assert_eq!(name, "root");
    }

    #[test]
    fn user_name_of_missing_uid_is_question() {
        // u32::MAX is never a real uid; getpwuid_r must fail -> "?".
        assert_eq!(user_name_of(u32::MAX), "?");
    }
}