use super::{ScenarioOutcome, load_scenario_json, run_scenario_with_trace};
use crate::lab::LabRuntime;
use crate::lab::fault::trace::{RecordedFaultDecision, TransportFactKind};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-lab-transport-recording-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const LIVE_SCENARIO: &[u8] = br#"{
    "schemaVersion": 1,
    "seed": 77,
    "nodes": [{"id": "host1"}, {"id": "listener1"}],
    "links": [{
        "from": "host1",
        "to": "listener1",
        "latencyMs": 0,
        "jitterMs": 0,
        "lossPermille": 0
    }],
    "fixtures": [{"id": "track", "displayName": "Lab Track"}],
    "steps": [
        {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
        {"atMs": 1, "node": "host1", "action": {"kind": "configureHost", "sessionName": "Lab Party", "fixture": "track"}},
        {"atMs": 2, "node": "host1", "action": {"kind": "createHostSession"}},
        {"atMs": 3, "node": "listener1", "action": {"kind": "selectRole", "role": "listener"}},
        {"atMs": 4, "node": "listener1", "action": {"kind": "startDiscovery"}},
        {"atMs": 5, "node": "listener1", "action": {"kind": "selectSession", "sessionId": "session-1"}},
        {"atMs": 6, "node": "listener1", "action": {"kind": "submitJoin"}},
        {"atMs": 8, "node": "host1", "action": {"kind": "approveJoin", "requestId": "desktop-join-1"}},
        {"atMs": 9, "node": "listener1", "action": {"kind": "injectUnderrun", "missingFrames": 0}}
    ],
    "assertions": [
        {"kind": "listenerCountAtLeast", "byMs": 20, "node": "host1", "count": 1},
        {"kind": "lifecycleReached", "byMs": 20, "node": "listener1", "target": {"machine": "listener", "state": "approved"}}
    ],
    "timeoutMs": 20
}"#;

#[test]
fn scenario_trace_contains_real_packet_and_fault_evidence() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(LIVE_SCENARIO).expect("valid live scenario");

    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("live scenario runs");

    assert_eq!(report.outcome, ScenarioOutcome::Completed);
    assert!(
        trace.transport_trace.facts.iter().any(|fact| matches!(
            &fact.entry,
            TransportFactKind::Packet { .. }
        )),
        "the live driver must persist real received packet metadata"
    );
    assert!(
        trace.transport_trace.facts.iter().any(|fact| matches!(
            &fact.entry,
            TransportFactKind::FaultDecision {
                decision: RecordedFaultDecision::Pass,
                ..
            }
        )),
        "a zero-fault live datagram must persist its actual pass decision"
    );
}
