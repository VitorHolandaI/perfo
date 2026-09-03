use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

use super::disk::rate;

/// Per-interface network counters from /proc/net/dev.
///
/// Line layout (after the name): rx_bytes rx_packets rx_errs rx_drop
/// rx_fifo rx_frame rx_compressed rx_multicast tx_bytes tx_packets
/// tx_errs tx_drop tx_fifo tx_colls tx_carrier tx_compressed.
type DevCounters = (u64, u64, u64, u64, u64, u64, u64, u64);

fn netdev_from(raw: &str) -> HashMap<String, DevCounters> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let (name, rest) = match line.split_once(':') {
            Some((n, r)) => (n.trim().to_string(), r),
            None => continue,
        };
        let mut f = rest.split_whitespace();
        let mut n = [0u64; 16];
        let mut count = 0usize;
        for slot in n.iter_mut() {
            match f.next().and_then(|v| v.parse().ok()) {
                Some(v) => {
                    *slot = v;
                    count += 1;
                }
                None => break,
            }
        }
        if count < 12 {
            continue;
        }
        out.insert(
            name,
            (
                n[0],  // rx_bytes
                n[1],  // rx_packets
                n[2],  // rx_errs
                n[3],  // rx_drop
                n[8],  // tx_bytes
                n[9],  // tx_packets
                n[10], // tx_errs
                n[11], // tx_drop
            ),
        );
    }
    out
}

/// (tcp_retrans_total, tcp_established) from /proc/net/snmp + /proc/net/tcp.
fn tcp_stats_from(snmp_raw: &str, tcp_raw: &str) -> (u64, u64) {
    let mut retrans = 0u64;
    let mut in_snmp = false;
    for line in snmp_raw.lines() {
        // Header first ("Tcp: RtoAlgorithm ..."), values second ("Tcp: 1 ...").
        if line.starts_with("Tcp:") {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if in_snmp {
                // RetransSegs is field 13 (1-based) after "Tcp:" -> token 12.
                if let Some(v) = fields.get(12).and_then(|s| s.parse().ok()) {
                    retrans = v;
                }
                break;
            }
            in_snmp = true;
        }
    }
    // Established connections = lines minus the "sl local ..." header.
    let established = tcp_raw.lines().filter(|l| !l.starts_with("sl ")).count() as u64;
    (retrans, established)
}

/// Link speed (Mbps) and carrier state from /sys/class/net/<iface>/.
/// Virtual interfaces (lo, veth) expose neither — both become None/false
/// with speed None signalling "no link concept".
fn link_state(iface: &str) -> (Option<u64>, bool) {
    let base = format!("/sys/class/net/{iface}");
    let speed = std::fs::read_to_string(format!("{base}/speed"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0);
    let up = std::fs::read_to_string(format!("{base}/carrier"))
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    (speed, up)
}

#[derive(Clone, Serialize)]
pub struct NetInfo {
    pub name: String,
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub rx_pps: u64,
    pub tx_pps: u64,
    pub rx_errs_s: u64,
    pub tx_errs_s: u64,
    pub rx_drops_s: u64,
    pub tx_drops_s: u64,
    pub link_mbps: Option<u64>,
    pub link_up: bool,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

#[derive(Clone, Serialize, Default)]
pub struct NetTotals {
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub tcp_retrans_s: u64,
    pub tcp_established: u64,
}

#[derive(Clone, Serialize)]
pub struct NetSnapshot {
    pub ifaces: Vec<NetInfo>,
    pub totals: NetTotals,
}

pub struct NetMonitor {
    /// When the last refresh happened; drives the delta->rate conversion.
    last_refresh: Option<Instant>,
    prev: HashMap<String, DevCounters>,
    prev_retrans: u64,
    snapshot: NetSnapshot,
}

impl NetMonitor {
    pub fn new() -> Self {
        Self {
            last_refresh: None,
            prev: HashMap::new(),
            prev_retrans: 0,
            snapshot: NetSnapshot {
                ifaces: Vec::new(),
                totals: NetTotals::default(),
            },
        }
    }

    pub fn refresh(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let raw = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let cur = netdev_from(&raw);
        let mut ifaces = Vec::new();
        for (name, c) in &cur {
            if let Some(p) = self.prev.get(name) {
                let d = |a: u64, b: u64| b.saturating_sub(a);
                let (link_mbps, link_up) = link_state(name);
                ifaces.push(NetInfo {
                    name: name.clone(),
                    rx_bps: rate(d(p.0, c.0), elapsed),
                    tx_bps: rate(d(p.4, c.4), elapsed),
                    rx_pps: rate(d(p.1, c.1), elapsed),
                    tx_pps: rate(d(p.5, c.5), elapsed),
                    rx_errs_s: rate(d(p.2, c.2), elapsed),
                    tx_errs_s: rate(d(p.6, c.6), elapsed),
                    rx_drops_s: rate(d(p.3, c.3), elapsed),
                    tx_drops_s: rate(d(p.7, c.7), elapsed),
                    link_mbps,
                    link_up,
                    total_rx_bytes: c.0,
                    total_tx_bytes: c.4,
                });
            }
        }
        ifaces.sort_by_key(|a| std::cmp::Reverse(a.rx_bps));
        let (retrans_total, established) = tcp_stats_from(
            &std::fs::read_to_string("/proc/net/snmp").unwrap_or_default(),
            &std::fs::read_to_string("/proc/net/tcp").unwrap_or_default(),
        );
        let retrans_s = rate(retrans_total.saturating_sub(self.prev_retrans), elapsed);
        self.prev_retrans = retrans_total;
        self.snapshot = NetSnapshot {
            totals: NetTotals {
                rx_bps: ifaces.iter().map(|i| i.rx_bps).sum(),
                tx_bps: ifaces.iter().map(|i| i.tx_bps).sum(),
                tcp_retrans_s: retrans_s,
                tcp_established: established,
            },
            ifaces,
        };
        self.prev = cur;
        self.last_refresh = Some(now);
    }

    pub fn snapshot(&self) -> NetSnapshot {
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_line() -> &'static str {
        "  enp3s0: 1000 200 3 4 0 0 0 0 5000 100 2 6 0 0 0 0\n"
    }

    #[test]
    fn netdev_parses_columns() {
        let m = netdev_from(&format!(
            "Inter-|   Receive ...\nface |bytes ...\n{}",
            dev_line()
        ));
        let c = &m["enp3s0"];
        assert_eq!(c.0, 1000); // rx_bytes
        assert_eq!(c.1, 200); // rx_packets
        assert_eq!(c.2, 3); // rx_errs
        assert_eq!(c.3, 4); // rx_drop
        assert_eq!(c.4, 5000); // tx_bytes
        assert_eq!(c.5, 100); // tx_packets
        assert_eq!(c.6, 2); // tx_errs
        assert_eq!(c.7, 6); // tx_drop
    }

    #[test]
    fn netdev_skips_garbage() {
        let m = netdev_from("not an interface line\nlo: 1 2 3 4 5\n");
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn tcp_stats_parses_retrans_and_established() {
        let snmp = "Tcp: RtoAlgorithm RtoMin RtoMax MaxConn ActiveOpens PassiveOpens AttemptFails EstabResets CurrEstab InSegs OutSegs RetransSegs InErrs OutRsts\nTcp: 1 200 120000 -1 10 5 0 2 3 100 90 7 0 1\n";
        let tcp = "sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n  0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000    30        0 1000 2 3\n  1: 0100007F:C350 00000000:0000 0A 00000000:00000000 00:00000000 00000000    30        0 1000 2 3\n";
        let (retrans, established) = tcp_stats_from(snmp, tcp);
        assert_eq!(retrans, 7);
        assert_eq!(established, 2);
    }

    #[test]
    fn rate_zero_elapsed_is_safe() {
        assert_eq!(rate(100, 0.0), 0);
        assert_eq!(rate(100, 2.0), 50);
    }
}
