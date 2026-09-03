//! Integration test through the library's public API (the way an external
//! consumer or the Omarchy widget would use it).

use perfo::data::cpu::CpuMonitor;

#[test]
fn snapshot_has_core_sections() {
    let mut m = CpuMonitor::new();
    perfo::data::cpu::wait_sample_interval();
    m.refresh();
    let snap = m.snapshot();

    assert!(snap.core_count > 0, "no CPUs detected");
    assert_eq!(snap.per_core.len(), snap.core_count);
    assert!(!snap.processes.is_empty(), "no processes in snapshot");
    assert!(!snap.disks.is_empty(), "no disks in snapshot");
    assert!(!snap.net.ifaces.is_empty(), "no network interfaces");
}

#[test]
fn snapshot_serializes_to_json() {
    let mut m = CpuMonitor::new();
    perfo::data::cpu::wait_sample_interval();
    m.refresh();
    let snap = m.snapshot();

    let json = serde_json::to_value(&snap).expect("snapshot must serialize");
    assert!(json["net"]["totals"].is_object());
    assert!(json["disks"].is_array());
    assert!(json["per_core"].is_array());
    assert!(json["processes"].is_array());
}
