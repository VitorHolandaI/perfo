use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;
use sysinfo::Disks;

use super::psi;

/// Per-disk I/O rates derived from /proc/diskstats deltas (iostat-style).
#[derive(Clone, Serialize, Default)]
pub struct DiskIoStats {
    /// Read/write operations per second (IOPS, after merges).
    pub r_s: u64,
    pub w_s: u64,
    /// Average read/write latency in ms, queue time included. The number
    /// that actually tells you the disk is slow: NVMe healthy < 1ms,
    /// > 10ms = queueing hard.
    pub r_await_ms: f32,
    pub w_await_ms: f32,
    /// Average number of requests in flight (weighted time / interval).
    pub queue_avg: f32,
    /// % of the interval with at least one I/O in flight. On NVMe this is
    /// NOT saturation — a drive with 16 queues reports 100% with one
    /// request; trust await + queue instead.
    pub busy_pct: f32,
    /// % of requests that were merged with neighbours (sequential-ish).
    pub read_merge_pct: f32,
    pub write_merge_pct: f32,
    /// Average request size in KiB: 128+ = sequential, 4-16 = random.
    pub read_req_kib: f32,
    pub write_req_kib: f32,
}

#[derive(Clone, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub fs: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub percent: f32,
    /// Read rate since the last refresh (bytes/second).
    pub read_bps: u64,
    /// Write rate since the last refresh (bytes/second).
    pub write_bps: u64,
    /// Cumulative bytes read since boot (from /proc/diskstats).
    pub total_read_bytes: u64,
    pub total_written_bytes: u64,
    #[serde(flatten)]
    pub io: DiskIoStats,
}

/// Bytes per second from a refresh-interval delta.
fn rate(delta: u64, elapsed_secs: f32) -> u64 {
    if elapsed_secs <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed_secs as f64) as u64
    }
}

/// Raw per-device counters from /proc/diskstats (kernel iostats fields).
#[derive(Clone, Default)]
struct RawCounters {
    reads: u64,
    reads_merged: u64,
    sectors_read: u64,
    ms_reading: u64,
    writes: u64,
    writes_merged: u64,
    sectors_written: u64,
    ms_writing: u64,
    ms_doing_io: u64,
    ms_weighted_io: u64,
}

/// Parses /proc/diskstats lines keyed by device name. Discard/flush fields
/// (kernel 4.18+/5.5+) are ignored — they do not drive the metrics we show.
fn diskstats_from(raw: &str) -> HashMap<String, RawCounters> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let mut f = line.split_whitespace();
        let (_major, _minor, name) = (f.next(), f.next(), f.next());
        let Some(name) = name else { continue };
        let mut n = [0u64; 11];
        let mut ok = true;
        for slot in n.iter_mut() {
            match f.next().and_then(|v| v.parse().ok()) {
                Some(v) => *slot = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        out.insert(
            name.to_string(),
            RawCounters {
                reads: n[0],
                reads_merged: n[1],
                sectors_read: n[2],
                ms_reading: n[3],
                writes: n[4],
                writes_merged: n[5],
                sectors_written: n[6],
                ms_writing: n[7],
                ms_doing_io: n[9],
                ms_weighted_io: n[10],
            },
        );
    }
    out
}

/// Deltas between two diskstats samples -> iostat-style metrics.
fn io_stats_from(prev: &RawCounters, cur: &RawCounters, elapsed_secs: f32) -> DiskIoStats {
    let e = if elapsed_secs <= 0.0 {
        1.0
    } else {
        elapsed_secs
    };
    let d = |a: u64, b: u64| b.saturating_sub(a);
    let reads = d(prev.reads, cur.reads);
    let writes = d(prev.writes, cur.writes);
    let r_await = if reads > 0 {
        d(prev.ms_reading, cur.ms_reading) as f32 / reads as f32
    } else {
        0.0
    };
    let w_await = if writes > 0 {
        d(prev.ms_writing, cur.ms_writing) as f32 / writes as f32
    } else {
        0.0
    };
    let r_merge = reads + d(prev.reads_merged, cur.reads_merged);
    let w_merge = writes + d(prev.writes_merged, cur.writes_merged);
    DiskIoStats {
        r_s: rate(reads, e),
        w_s: rate(writes, e),
        r_await_ms: r_await,
        w_await_ms: w_await,
        queue_avg: d(prev.ms_weighted_io, cur.ms_weighted_io) as f32 / (e * 1000.0),
        busy_pct: d(prev.ms_doing_io, cur.ms_doing_io) as f32 / (e * 10.0),
        read_merge_pct: if r_merge > 0 {
            d(prev.reads_merged, cur.reads_merged) as f32 / r_merge as f32 * 100.0
        } else {
            0.0
        },
        write_merge_pct: if w_merge > 0 {
            d(prev.writes_merged, cur.writes_merged) as f32 / w_merge as f32 * 100.0
        } else {
            0.0
        },
        read_req_kib: if reads > 0 {
            d(prev.sectors_read, cur.sectors_read) as f32 * 512.0 / 1024.0 / reads as f32
        } else {
            0.0
        },
        write_req_kib: if writes > 0 {
            d(prev.sectors_written, cur.sectors_written) as f32 * 512.0 / 1024.0 / writes as f32
        } else {
            0.0
        },
    }
}

pub struct DiskMonitor {
    disks: Disks,
    /// When the last refresh happened; drives the delta->rate conversion.
    last_refresh: Option<Instant>,
    prev_stats: HashMap<String, RawCounters>,
    io_stats: HashMap<String, DiskIoStats>,
    /// PSI "some" I/O pressure (10s/60s/300s) from /proc/pressure/io.
    io_pressure: [f64; 3],
}

impl DiskMonitor {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            last_refresh: None,
            prev_stats: HashMap::new(),
            io_stats: HashMap::new(),
            io_pressure: [0.0; 3],
        }
    }

    pub fn refresh(&mut self) {
        // refresh(false) refreshes everything, including the io_usage deltas
        // that Disk::usage() reports as "since the last refresh".
        self.disks.refresh(false);
        let now = Instant::now();
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let raw = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();
        let cur = diskstats_from(&raw);
        // device-mapper devices show as dm-N; resolve to the friendly name
        // (e.g. dm-0 -> "root") so they match sysinfo's /dev/mapper/root.
        let cur: HashMap<String, RawCounters> = cur
            .into_iter()
            .map(|(dev, counters)| {
                let key = if let Some(n) = dev.strip_prefix("dm-") {
                    std::fs::read_to_string(format!("/sys/block/dm-{n}/dm/name"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or(dev.clone())
                } else {
                    dev.clone()
                };
                (key, counters)
            })
            .collect();
        self.io_stats = cur
            .iter()
            .filter_map(|(name, c)| {
                self.prev_stats
                    .get(name)
                    .map(|p| (name.clone(), io_stats_from(p, c, elapsed)))
            })
            .collect();
        self.prev_stats = cur;
        self.last_refresh = Some(now);
        let (p10, p60, p300) = psi::some("io");
        self.io_pressure = [p10, p60, p300];
    }

    /// PSI I/O pressure "some" averages (10s/60s/300s).
    pub fn io_pressure(&self) -> [f64; 3] {
        self.io_pressure
    }

    pub fn snapshot(&self) -> Vec<DiskInfo> {
        const REAL_FS: [&str; 9] = [
            "btrfs", "vfat", "ext4", "xfs", "f2fs", "ntfs", "zfs", "exfat", "ext2",
        ];
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        self.disks
            .list()
            .iter()
            .filter(|d| {
                let fs = d.file_system().to_string_lossy();
                REAL_FS.contains(&fs.as_ref())
            })
            .map(|d| {
                let u = d.usage();
                let total = d.total_space();
                let available = d.available_space();
                let used = total.saturating_sub(available);
                let name = d.name().to_string_lossy().into_owned();
                let key = name.rsplit('/').next().unwrap_or(&name);
                let io = self.io_stats.get(key).cloned().unwrap_or_default();
                DiskInfo {
                    name,
                    mount: d.mount_point().to_string_lossy().into_owned(),
                    fs: d.file_system().to_string_lossy().into_owned(),
                    total_bytes: total,
                    available_bytes: available,
                    used_bytes: used,
                    percent: if total > 0 {
                        used as f32 / total as f32 * 100.0
                    } else {
                        0.0
                    },
                    read_bps: rate(u.read_bytes, elapsed),
                    write_bps: rate(u.written_bytes, elapsed),
                    total_read_bytes: u.total_read_bytes,
                    total_written_bytes: u.total_written_bytes,
                    io,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> &'static str {
        "259 0 nvme0n1 1000 0 100000 500 200 0 40000 400 0 600 800\n"
    }

    #[test]
    fn rate_converts_delta_to_bps() {
        assert_eq!(rate(1_000_000, 2.0), 500_000);
        assert_eq!(rate(0, 2.0), 0);
        assert_eq!(rate(500, 0.0), 0);
    }

    #[test]
    fn diskstats_parses_counters() {
        let m = diskstats_from(line());
        let c = &m["nvme0n1"];
        assert_eq!(c.reads, 1000);
        assert_eq!(c.sectors_read, 100000);
        assert_eq!(c.ms_reading, 500);
        assert_eq!(c.writes, 200);
        assert_eq!(c.ms_doing_io, 600);
        assert_eq!(c.ms_weighted_io, 800);
    }

    #[test]
    fn diskstats_skips_garbage() {
        let m = diskstats_from("garbage line\n259 0 nvme0n1 1 2 3 4 5\n");
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn io_stats_computes_await_and_busy() {
        let prev = diskstats_from(line());
        // 2s later: 2000 reads done, 400ms total reading, 2000ms doing io.
        let cur_raw = "259 0 nvme0n1 3000 0 300000 900 400 0 80000 800 0 2600 2800\n";
        let cur = diskstats_from(cur_raw);
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 2.0);
        assert_eq!(s.r_s, 1000);
        assert_eq!(s.w_s, 100);
        // (900-500)ms / 2000 reads = 0.2ms
        assert_eq!(s.r_await_ms, 0.2);
        // (2600-600)ms / (2s*1000) = 1.0
        assert_eq!(s.queue_avg, 1.0);
        // (2800-800)ms / (2s*10) = 100%
        assert_eq!(s.busy_pct, 100.0);
    }

    #[test]
    fn io_stats_zero_elapsed_is_safe() {
        let prev = diskstats_from(line());
        let cur = diskstats_from(line());
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 0.0);
        assert_eq!(s.r_s, 0);
        assert_eq!(s.busy_pct, 0.0);
    }

    #[test]
    fn io_stats_computes_merge_and_req_size() {
        let prev = diskstats_from(line());
        let cur_raw = "259 0 nvme0n1 2000 1000 200000 600 300 150 50000 500 0 700 900\n";
        let cur = diskstats_from(cur_raw);
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 1.0);
        // 1000 extra merged of 2000 total (1000 reads + 1000 merged) -> 50%
        assert!((s.read_merge_pct - 50.0).abs() < 0.01);
        // 100000 extra sectors * 512 / 1024 / 1000 reads = 50 KiB
        assert_eq!(s.read_req_kib, 50.0);
        // (500-400)ms / 100 writes = 1ms
        assert_eq!(s.w_await_ms, 1.0);
    }
}
