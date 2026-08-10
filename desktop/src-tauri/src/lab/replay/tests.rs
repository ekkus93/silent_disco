use super::{ReplayError, ScenarioRecording, replay};
use crate::lab::LabRuntime;
use crate::lab::scenario::{ScenarioOutcome, load_scenario_json, run_scenario};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-lab-replay-{}-{sequence}",
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

const SCENARIO_JSON: &[u8] = br#"{
    "schemaVersion": 1,
    "seed": 7,
    "nodes": [{"id": "host1"}],
    "steps": [
        {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}}
    ],
    "assertions": [
        {"kind": "lifecycleReached", "byMs": 50, "node": "host1",
         "target": {"machine": "role", "state": "host"}}
    ],
    "timeoutMs": 50
}"#;

/// Replaying against the exact scenario/seed a recording was captured under
/// succeeds and reproduces the same report (Block 40.4 "deterministic
/// report" carried through replay, not just a fresh run).
#[test]
fn replay_against_the_matching_scenario_reproduces_the_report() {
    let capture_root = TestDirectory::new();
    let capture_lab = LabRuntime::new(&capture_root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let original_report = run_scenario(&capture_lab, &scenario).expect("captured run");
    let recording = ScenarioRecording::capture(&scenario, original_report.clone());

    let replay_root = TestDirectory::new();
    let replay_lab = LabRuntime::new(&replay_root.0, 0).expect("lab runtime");
    let replayed_report = replay(&replay_lab, &scenario, &recording).expect("replay succeeds");

    assert_eq!(replayed_report, recording.report);
    assert_eq!(replayed_report.outcome, ScenarioOutcome::Completed);
}

/// Spec 29.5 "Replay must detect version incompatibility rather than
/// silently reinterpret an old recording": a recording captured under a
/// different schema version is refused outright.
#[test]
fn replay_refuses_a_schema_version_mismatch() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let report = run_scenario(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report);
    recording.schema_version += 1;

    let error =
        replay(&lab, &scenario, &recording).expect_err("mismatched schema version must be refused");
    assert!(matches!(
        error,
        ReplayError::SchemaVersionMismatch { recorded, replaying }
            if recorded == scenario.schema_version + 1 && replaying == scenario.schema_version
    ));
}

/// Same discipline for a seed mismatch -- replaying against a scenario
/// document whose seed no longer matches what was recorded is refused, not
/// silently re-run under the new seed as if nothing changed.
#[test]
fn replay_refuses_a_seed_mismatch() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let report = run_scenario(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report);
    recording.seed = recording.seed.wrapping_add(1);

    let error = replay(&lab, &scenario, &recording).expect_err("mismatched seed must be refused");
    assert!(matches!(error, ReplayError::SeedMismatch { .. }));
}
