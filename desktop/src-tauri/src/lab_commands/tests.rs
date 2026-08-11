use super::dto_convert::{
    bounded_summary_text, node_dto, run_outcome_dto, scenario_summary_dto, state_dto,
};
use super::scenario_io::read_bounded_scenario_file;
use super::{LabSessionState, MAX_SUMMARY_CHARS, MAX_TIMELINE_ENTRIES_PER_NODE};
use crate::lab::LabRuntime;
use crate::lab::scenario::{MAX_SCENARIO_FILE_BYTES, load_scenario_json, run_scenario_with_trace};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-lab-commands-{}-{sequence}",
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
    "links": [{"from": "host1", "to": "host1", "latencyMs": 30, "jitterMs": 8, "lossPermille": 10}],
    "steps": [
        {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
        {"atMs": 10, "node": "host1", "action": {"kind": "exportDiagnostics"}}
    ],
    "assertions": [
        {"kind": "lifecycleReached", "byMs": 50, "node": "host1",
         "target": {"machine": "role", "state": "host"}}
    ],
    "timeoutMs": 100
}"#;

/// Block 42 "scenario open ... through restricted dialogs": every scenario
/// field the UI needs to display is present in the summary, including the
/// declared (not-yet-wired) fault configuration.
#[test]
fn scenario_summary_reflects_the_real_document() {
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario");
    let summary = scenario_summary_dto(&scenario);

    assert_eq!(summary.schema_version, 1);
    assert_eq!(summary.seed, "7");
    assert_eq!(summary.node_ids, vec!["host1".to_owned()]);
    assert_eq!(summary.step_count, 2);
    assert_eq!(summary.assertion_count, 1);
    assert_eq!(summary.timeout_ms, "100");
    assert_eq!(summary.links.len(), 1);
    assert_eq!(summary.links[0].latency_ms, "30");
    assert_eq!(summary.links[0].jitter_ms, "8");
    assert_eq!(summary.links[0].loss_permille, 10);
}

/// Block 42 "assertion results", "bounded event timeline": a real scenario
/// run's report/trace convert into a DTO whose outcome, step results, and
/// assertion results are legible wire strings, not opaque debug output.
#[test]
fn run_outcome_dto_reports_a_completed_scenario_with_a_held_assertion() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(SCENARIO_JSON).expect("valid scenario");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("scenario runs");

    let dto = run_outcome_dto(&report, &trace);
    assert_eq!(dto.outcome, "completed");
    assert_eq!(dto.step_results.len(), 2);
    for step in &dto.step_results {
        assert!(step.submit_error.is_none());
        assert_eq!(step.settlement, "settled");
    }
    assert_eq!(dto.assertion_results.len(), 1);
    assert_eq!(dto.assertion_results[0].outcome, "held");
    assert!(
        !dto.timeline.is_empty(),
        "a real run always emits a startup snapshot"
    );
    assert!(!dto.timeline_truncated);
}

/// Block 42 "bounded event timeline" / bounded-history test target: a node
/// that recorded more entries than the per-node cap is truncated, not
/// silently dropped without a signal.
#[test]
fn timeline_is_bounded_per_node_and_reports_truncation() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    // Repeatedly setting local volume is a real, always-accepted, no-op-safe
    // command that produces one snapshot notification per submission --
    // enough real, distinct recorded entries to exceed the per-node cap.
    let mut steps = String::new();
    for index in 0..(MAX_TIMELINE_ENTRIES_PER_NODE + 10) {
        let at_ms = index * 2;
        // Alternates the requested gain so every submission is a genuine
        // state change (and therefore settles quickly) rather than risking
        // an idempotent no-op that never bumps the snapshot revision.
        let gain = if index % 2 == 0 { 1.0 } else { 0.5 };
        if index > 0 {
            steps.push(',');
        }
        let _ = write!(
            steps,
            r#"{{"atMs": {at_ms}, "node": "host1", "action": {{"kind": "setLocalVolume", "linearGain": {gain}}}}}"#
        );
    }
    let json = format!(
        r#"{{"schemaVersion": 1, "seed": 1, "nodes": [{{"id": "host1"}}], "steps": [{steps}], "assertions": [], "timeoutMs": 100000}}"#
    );
    let scenario = load_scenario_json(json.as_bytes()).expect("valid scenario");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario).expect("scenario runs");

    let dto = run_outcome_dto(&report, &trace);
    let host_entries = dto
        .timeline
        .iter()
        .filter(|entry| entry.node == "host1")
        .count();
    assert_eq!(host_entries, MAX_TIMELINE_ENTRIES_PER_NODE);
    assert!(dto.timeline_truncated);
}

/// Block 42 "node list and state panels": a manually started node's DTO
/// carries its real, non-fabricated identity and clock configuration.
#[test]
fn node_dto_reflects_a_real_started_node() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let node_id = lab
        .start_node_with_clock(500, -20)
        .expect("start node with clock configuration");

    let dto = node_dto(&lab, node_id).expect("node exists");
    assert_eq!(dto.offset_ms, "500");
    assert_eq!(dto.drift_ppm, "-20");
    assert_eq!(dto.node_id, node_id.as_u32().to_string());

    lab.shutdown().expect("teardown");
}

/// [`state_dto`] reports every node currently held by the runtime and the
/// session's own `running`/loaded-scenario fields verbatim.
#[test]
fn state_dto_reports_nodes_and_session_fields() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    lab.start_node().expect("start node");
    let session = LabSessionState {
        runtime: None,
        loaded: None,
        running: true,
        last_run: None,
    };

    let dto = state_dto(&lab, &session);
    assert!(dto.running);
    assert_eq!(dto.nodes.len(), 1);
    assert!(dto.loaded_scenario.is_none());
    assert!(dto.last_run.is_none());

    lab.shutdown().expect("teardown");
}

/// Block 43.1/43.2 "bounded payload": an oversized scenario file is rejected
/// from filesystem metadata alone, before any of its bytes are read into
/// memory -- proven here by checking the returned error code, since a
/// successful read would return the file's own bytes instead.
#[test]
fn oversized_scenario_file_is_rejected_before_being_read() {
    let root = TestDirectory::new();
    let path = root.0.join("huge-scenario.json");
    let oversized = vec![b'a'; MAX_SCENARIO_FILE_BYTES + 1];
    fs::write(&path, &oversized).expect("write oversized fixture file");

    let error =
        read_bounded_scenario_file(&path).expect_err("oversized scenario file must be rejected");
    assert_eq!(error.code, "desktop.lab.scenario_too_large");
}

/// The same helper accepts a file at or under the limit and returns its
/// exact bytes, unmodified.
#[test]
fn scenario_file_within_the_limit_is_read_verbatim() {
    let root = TestDirectory::new();
    let path = root.0.join("scenario.json");
    fs::write(&path, SCENARIO_JSON).expect("write scenario fixture file");

    let bytes = read_bounded_scenario_file(&path).expect("scenario file within the limit reads");
    assert_eq!(bytes, SCENARIO_JSON);
}

/// A summary well within the bound is untouched; an oversized one is
/// truncated with a visible ellipsis rather than silently clipped.
#[test]
fn bounded_summary_text_truncates_only_when_necessary() {
    let short = "short message";
    assert_eq!(bounded_summary_text(short), short);

    let long = "x".repeat(MAX_SUMMARY_CHARS + 50);
    let truncated = bounded_summary_text(&long);
    assert!(truncated.chars().count() <= MAX_SUMMARY_CHARS + 1);
    assert!(truncated.ends_with('…'));
}
