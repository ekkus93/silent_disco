use super::{ReplayError, replay};
use crate::lab::LabRuntime;
use crate::lab::recording::ScenarioRecording;
use crate::lab::scenario::{ScenarioOutcome, load_scenario_json, run_scenario_with_trace};
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

const LIVE_TRANSPORT_SCENARIO_JSON: &[u8] = br#"{
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

/// Replaying against the exact scenario/seed a recording was captured under
/// succeeds and reports no divergence (Block 41.3 "record then replay
/// identical").
#[test]
fn replay_against_the_matching_scenario_reproduces_the_report_with_no_divergence() {
    let capture_root = TestDirectory::new();
    let capture_lab = LabRuntime::new(&capture_root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (original_report, trace) =
        run_scenario_with_trace(&capture_lab, &scenario).expect("captured run");
    let recording = ScenarioRecording::capture(&scenario, original_report.clone(), trace);

    let replay_root = TestDirectory::new();
    let replay_lab = LabRuntime::new(&replay_root.0, 0).expect("lab runtime");
    let outcome = replay(&replay_lab, &scenario, &recording).expect("replay succeeds");

    assert_eq!(outcome.report, recording.report);
    assert_eq!(outcome.divergence, None);
    assert_eq!(outcome.report.outcome, ScenarioOutcome::Completed);
    // Same build replaying its own just-captured recording: both version
    // pairs agree exactly.
    assert_eq!(
        outcome.recorded_protocol_version,
        outcome.current_protocol_version
    );
    assert_eq!(outcome.recorded_core_version, outcome.current_core_version);
}

/// Block 41's own acceptance criterion, taken literally: a recording is
/// saved to a real file, then loaded back and replayed later -- not just
/// held in memory across the same test.
#[test]
fn a_recording_saved_to_disk_can_be_loaded_back_and_replayed_later() {
    let capture_root = TestDirectory::new();
    let capture_lab = LabRuntime::new(&capture_root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&capture_lab, &scenario).expect("captured run");
    let recording = ScenarioRecording::capture(&scenario, report, trace);

    let recording_directory = TestDirectory::new();
    let recording_path = recording_directory.0.join("recording.json");
    crate::lab::recording::save_recording_to_path(&recording, &recording_path)
        .expect("save succeeds");
    let loaded_recording =
        crate::lab::recording::load_recording_from_path(&recording_path).expect("load succeeds");

    let replay_root = TestDirectory::new();
    let replay_lab = LabRuntime::new(&replay_root.0, 0).expect("lab runtime");
    let outcome =
        replay(&replay_lab, &scenario, &loaded_recording).expect("replay from disk succeeds");

    assert_eq!(outcome.divergence, None);
}

/// Block 41.3 "changed core behavior produces divergence": a recording
/// whose captured report no longer matches what a fresh run produces is
/// detected and reported, not silently accepted as a match. Simulated the
/// same way Block 40 simulated a mismatched schema version/seed (mutating a
/// captured recording directly) since building two genuinely different
/// core builds is outside a single test's reach.
#[test]
fn a_recording_whose_captured_behavior_differs_from_a_fresh_run_is_detected() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    assert!(
        !recording.report.step_results.is_empty(),
        "test scenario must submit at least one step"
    );
    recording.report.step_results[0].submit_error =
        Some("a later core build rejected this step".to_owned());

    let outcome = replay(&lab, &scenario, &recording).expect("replay itself still succeeds");

    assert!(matches!(
        outcome.divergence,
        Some(crate::lab::recording::Divergence::StepResultMismatch { index: 0, .. })
    ));
}

/// Spec 29.5 "Replay must detect version incompatibility rather than
/// silently reinterpret an old recording": a recording captured under a
/// different scenario schema version is refused outright.
#[test]
fn replay_refuses_a_schema_version_mismatch() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    recording.scenario_schema_version += 1;

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
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    recording.seed = recording.seed.wrapping_add(1);

    let error = replay(&lab, &scenario, &recording).expect_err("mismatched seed must be refused");
    assert!(matches!(error, ReplayError::SeedMismatch { .. }));
}

/// And again for the persisted recording's own on-disk format version --
/// distinct from the scenario's schema version, and from Block 40's
/// pre-existing two checks above.
#[test]
fn replay_refuses_a_recording_format_version_mismatch() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    recording.recording_format_version += 1;

    let error = replay(&lab, &scenario, &recording)
        .expect_err("mismatched recording format version must be refused");
    assert!(matches!(
        error,
        ReplayError::RecordingFormatVersionMismatch { .. }
    ));
}

/// Block 41's own design point: a *differing* protocol/core version between
/// the recording and this build must NOT block replay by itself -- that is
/// exactly what "replay against a later core build" requires. Simulated by
/// hand-editing the recorded versions to something this build will not
/// naturally match, then confirming replay still succeeds.
#[test]
fn a_differing_recorded_protocol_or_core_version_does_not_block_replay() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    recording.protocol_version = recording.protocol_version.wrapping_add(1);
    recording.core_version.patch = recording.core_version.patch.wrapping_add(1);

    let outcome = replay(&lab, &scenario, &recording)
        .expect("a differing protocol/core version alone must not refuse replay");

    assert_ne!(
        outcome.recorded_protocol_version,
        outcome.current_protocol_version
    );
    assert_ne!(outcome.recorded_core_version, outcome.current_core_version);
}

#[test]
fn replay_detects_transport_evidence_divergence_when_the_report_still_matches() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario document");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("run");
    let mut recording = ScenarioRecording::capture(&scenario, report, trace);
    recording.trace.transport_trace.dropped_count = 1;

    let replay_root = TestDirectory::new();
    let replay_lab = LabRuntime::new(&replay_root.0, 0).expect("replay lab runtime");
    let outcome = replay(&replay_lab, &scenario, &recording).expect("replay succeeds");

    assert!(matches!(
        outcome.divergence,
        Some(crate::lab::recording::Divergence::TransportOverflowMismatch {
            recorded: 1,
            replayed: 0
        })
    ));
}

#[test]
fn live_transport_recording_replays_with_identical_packet_and_fault_evidence() {
    let capture_root = TestDirectory::new();
    let capture_lab = LabRuntime::new(&capture_root.0, 0).expect("capture lab runtime");
    let scenario =
        load_scenario_json(LIVE_TRANSPORT_SCENARIO_JSON).expect("valid live transport scenario");
    let (report, trace) =
        run_scenario_with_trace(&capture_lab, &scenario).expect("capture live scenario");
    assert!(!trace.transport_trace.facts.is_empty());
    let recording = ScenarioRecording::capture(&scenario, report, trace);

    let replay_root = TestDirectory::new();
    let replay_lab = LabRuntime::new(&replay_root.0, 0).expect("replay lab runtime");
    let outcome = replay(&replay_lab, &scenario, &recording).expect("live replay succeeds");

    assert_eq!(outcome.divergence, None);
}
