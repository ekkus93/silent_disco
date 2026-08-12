use super::{
    AssertionOutcome, NodeId, ScenarioAssertion, ScenarioOutcome, ScenarioParseError,
    ScenarioExecutionError, ScenarioRunControl, ScenarioRunControlError, ScenarioValidationError,
    evaluate_assertion, load_scenario_json, run_scenario, run_scenario_with_trace_controlled,
};
use crate::lab::LabRuntime;
use crate::lab::recorder::{RecordedNotification, RecordedNotificationKind};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

/// Duplicated from `crate::lab::tests` deliberately -- that helper is
/// private to its own module, and every other Lab submodule
/// (`clock::tests`, `fault::tests`) already keeps its own self-contained
/// test scaffolding rather than threading a shared one across module
/// boundaries.
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-lab-scenario-{}-{sequence}",
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

/// Block 40.4 "minimal happy path": a single node selects the host role and
/// exports diagnostics -- both real, unconditionally successful commands --
/// and the scenario's one lifecycle assertion holds.
#[test]
fn minimal_happy_path_completes_with_every_assertion_held() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1,
            "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [
                {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
                {"atMs": 10, "node": "host1", "action": {"kind": "exportDiagnostics"}}
            ],
            "assertions": [
                {"kind": "lifecycleReached", "byMs": 50, "node": "host1",
                 "target": {"machine": "role", "state": "host"}}
            ],
            "timeoutMs": 100
        }"#,
    )
    .expect("valid scenario document");

    let report = run_scenario(&lab, &scenario).expect("scenario runs");
    assert_eq!(report.outcome, ScenarioOutcome::Completed);
    assert_eq!(report.step_results.len(), 2);
    for step in &report.step_results {
        assert!(
            step.submit_error.is_none(),
            "unexpected step failure: {step:?}"
        );
    }
    assert_eq!(report.assertion_results.len(), 1);
    assert_eq!(report.assertion_results[0].outcome, AssertionOutcome::Held);
}

/// Block 40.1/40.4 "invalid schema": a document missing a required field
/// (`timeoutMs`) is rejected with a shape error, not silently defaulted.
#[test]
fn invalid_schema_is_rejected() {
    let error = load_scenario_json(
        br#"{"schemaVersion": 1, "seed": 1, "nodes": [], "steps": [], "assertions": []}"#,
    )
    .expect_err("missing timeoutMs must be rejected");
    assert!(matches!(error, ScenarioParseError::Shape(_)));
}

/// Block 40.1/40.4 "unknown version": an unsupported `schemaVersion` is
/// reported as its own distinct failure, not a generic shape error, and
/// never silently reinterpreted as the current version.
#[test]
fn unknown_schema_version_is_rejected_distinctly() {
    let error = load_scenario_json(
        br#"{"schemaVersion": 2, "seed": 1, "nodes": [], "steps": [], "assertions": [], "timeoutMs": 1}"#,
    )
    .expect_err("unknown schemaVersion must be rejected");
    assert!(matches!(
        error,
        ScenarioParseError::UnknownSchemaVersion { found: 2 }
    ));
}

/// A document with no `schemaVersion` field at all is rejected before any
/// other parsing is attempted.
#[test]
fn missing_schema_version_is_rejected() {
    let error = load_scenario_json(br#"{"seed": 1, "timeoutMs": 1}"#)
        .expect_err("missing schemaVersion must be rejected");
    assert!(matches!(error, ScenarioParseError::MissingSchemaVersion));
}

/// Block 40.1 "reject unknown commands": an action `"kind"` this runner
/// does not recognize is a hard parse error, never a silently skipped step.
#[test]
fn unknown_command_kind_is_rejected() {
    let error = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [{"atMs": 0, "node": "host1", "action": {"kind": "flyToTheMoon"}}],
            "assertions": [], "timeoutMs": 100
        }"#,
    )
    .expect_err("unknown action kind must be rejected");
    assert!(matches!(error, ScenarioParseError::Shape(_)));
}

/// Block 40.1 "reject unknown assertions": same discipline for assertion
/// kinds.
#[test]
fn unknown_assertion_kind_is_rejected() {
    let error = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [],
            "assertions": [{"kind": "vibesAreGood", "byMs": 1, "node": "host1"}],
            "timeoutMs": 100
        }"#,
    )
    .expect_err("unknown assertion kind must be rejected");
    assert!(matches!(error, ScenarioParseError::Shape(_)));
}

/// Block 40.4 "impossible assertion": a lone node can never have a
/// listener (no scenario in Block 40's scope wires live transport -- see
/// `scenario`'s own module doc comment), so this can never hold regardless
/// of how much virtual time passes.
#[test]
fn impossible_assertion_times_out() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [],
            "assertions": [{"kind": "listenerCountAtLeast", "byMs": 50, "node": "host1", "count": 1}],
            "timeoutMs": 50
        }"#,
    )
    .expect("valid scenario document");

    let report = run_scenario(&lab, &scenario).expect("scenario runs");
    assert_eq!(report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(
        report.assertion_results[0].outcome,
        AssertionOutcome::TimedOut
    );
}

/// Block 40.4 "timeout": the step that would have satisfied the assertion
/// is scheduled *after* the scenario's own bounded timeout budget, so it
/// never runs and the assertion can only time out -- distinct from
/// [`impossible_assertion_times_out`], where nothing could ever satisfy it
/// regardless of budget.
#[test]
fn a_step_scheduled_past_the_scenario_timeout_never_runs() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [
                {"atMs": 200, "node": "host1", "action": {"kind": "selectRole", "role": "host"}}
            ],
            "assertions": [
                {"kind": "lifecycleReached", "byMs": 50, "node": "host1",
                 "target": {"machine": "role", "state": "host"}}
            ],
            "timeoutMs": 50
        }"#,
    )
    .expect("valid scenario document");

    let report = run_scenario(&lab, &scenario).expect("scenario runs");
    assert!(
        report.step_results.is_empty(),
        "the step past timeoutMs must never run"
    );
    assert_eq!(report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(report.final_time_ms, 50);
}

/// Block 40.4 "deterministic report": two independent runs of the same
/// scenario document and seed produce an equal report.
#[test]
fn identical_scenario_and_seed_produce_a_deterministic_report() {
    let scenario_json = br#"{
        "schemaVersion": 1, "seed": 42,
        "nodes": [{"id": "host1"}, {"id": "listener1"}],
        "clocks": {"listener1": {"offsetMs": 15, "driftPpm": 20}},
        "steps": [
            {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
            {"atMs": 5, "node": "listener1", "action": {"kind": "selectRole", "role": "listener"}},
            {"atMs": 10, "node": "listener1", "action": {"kind": "startDiscovery"}},
            {"atMs": 15, "node": "host1", "action": {"kind": "exportDiagnostics"}}
        ],
        "assertions": [
            {"kind": "lifecycleReached", "byMs": 100, "node": "host1",
             "target": {"machine": "role", "state": "host"}},
            {"kind": "lifecycleReached", "byMs": 100, "node": "listener1",
             "target": {"machine": "role", "state": "listener"}}
        ],
        "timeoutMs": 100
    }"#;

    let first_root = TestDirectory::new();
    let first_lab = LabRuntime::new(&first_root.0, 0).expect("lab runtime");
    let first_scenario = load_scenario_json(scenario_json).expect("valid scenario document");
    let first_report = run_scenario(&first_lab, &first_scenario).expect("first run");

    let second_root = TestDirectory::new();
    let second_lab = LabRuntime::new(&second_root.0, 0).expect("lab runtime");
    let second_scenario = load_scenario_json(scenario_json).expect("valid scenario document");
    let second_report = run_scenario(&second_lab, &second_scenario).expect("second run");

    assert_eq!(first_report, second_report);
    assert_eq!(first_report.outcome, ScenarioOutcome::Completed);
}

#[test]
fn pause_before_first_step_holds_virtual_time_and_resume_preserves_report() {
    let scenario_json = br#"{
        "schemaVersion": 1, "seed": 43,
        "nodes": [{"id": "host1"}],
        "steps": [
            {"atMs": 10, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
            {"atMs": 20, "node": "host1", "action": {"kind": "exportDiagnostics"}}
        ],
        "assertions": [
            {"kind": "lifecycleReached", "byMs": 50, "node": "host1",
             "target": {"machine": "role", "state": "host"}}
        ],
        "timeoutMs": 50
    }"#;
    let scenario = load_scenario_json(scenario_json).expect("valid scenario document");

    let baseline_root = TestDirectory::new();
    let baseline_lab = LabRuntime::new(&baseline_root.0, 0).expect("baseline lab runtime");
    let baseline = run_scenario(&baseline_lab, &scenario).expect("baseline run");

    let controlled_root = TestDirectory::new();
    let controlled_lab = Arc::new(
        LabRuntime::new(&controlled_root.0, 0).expect("controlled lab runtime"),
    );
    let control = Arc::new(ScenarioRunControl::default());
    control.pause().expect("pause before run");
    let run_lab = Arc::clone(&controlled_lab);
    let run_control = Arc::clone(&control);
    let run_scenario = scenario.clone();
    let worker = thread::spawn(move || {
        run_scenario_with_trace_controlled(&run_lab, &run_scenario, &run_control)
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while controlled_lab.node_ids().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        controlled_lab.node_ids().len(),
        1,
        "scenario setup must reach the first deterministic step boundary"
    );
    thread::sleep(Duration::from_millis(30));
    assert_eq!(
        controlled_lab.now().get(),
        0,
        "pause must prevent the first virtual-time advance"
    );

    control.resume().expect("resume");
    let (report, _trace) = worker
        .join()
        .expect("controlled worker joins")
        .expect("controlled scenario completes");
    assert_eq!(report, baseline);
    assert!(controlled_lab.node_ids().is_empty(), "runner cleans its nodes");
}

#[test]
fn stop_releases_a_paused_run_and_runner_cleans_scenario_nodes() {
    let root = TestDirectory::new();
    let lab = Arc::new(LabRuntime::new(&root.0, 0).expect("lab runtime"));
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 44,
            "nodes": [{"id": "host1"}],
            "steps": [{"atMs": 10, "node": "host1", "action": {"kind": "exportDiagnostics"}}],
            "assertions": [], "timeoutMs": 100
        }"#,
    )
    .expect("valid scenario document");
    let control = Arc::new(ScenarioRunControl::default());
    control.pause().expect("pause before run");
    let run_lab = Arc::clone(&lab);
    let run_control = Arc::clone(&control);
    let worker = thread::spawn(move || {
        run_scenario_with_trace_controlled(&run_lab, &scenario, &run_control)
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while lab.node_ids().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lab.node_ids().len(), 1, "scenario node must be active before stop");
    control.request_stop().expect("request stop");
    let error = worker
        .join()
        .expect("controlled worker joins")
        .expect_err("stopped run must not report completion");
    assert!(matches!(
        error,
        ScenarioExecutionError::RunControl(ScenarioRunControlError::Stopped)
    ));
    assert!(lab.node_ids().is_empty(), "runner cleanup must release scenario nodes");
    assert_eq!(lab.now().get(), 0, "stop before first step must not advance time");
}

/// Block 40.4 "bounded malformed file behavior": an oversized file is
/// rejected outright, before any JSON parsing is even attempted.
#[test]
fn oversized_scenario_file_is_rejected_before_parsing() {
    let oversized = vec![b' '; super::MAX_SCENARIO_FILE_BYTES + 1];
    let error = load_scenario_json(&oversized).expect_err("oversized file must be rejected");
    assert!(matches!(error, ScenarioParseError::TooLarge { .. }));
}

/// Truncated/non-UTF8-shaped bytes never panic the parser -- always a
/// bounded, reported error.
#[test]
fn truncated_json_is_a_bounded_error_not_a_panic() {
    let error = load_scenario_json(b"{\"schemaVersion\": 1, \"seed\":")
        .expect_err("truncated JSON must be rejected");
    assert!(matches!(error, ScenarioParseError::NotUtf8OrJson(_)));
}

/// Bytes that are not JSON at all (arbitrary binary) are rejected the same
/// bounded way -- never interpreted as anything else, never a panic.
#[test]
fn arbitrary_binary_input_is_a_bounded_error_not_a_panic() {
    let error = load_scenario_json(&[0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF])
        .expect_err("non-JSON bytes must be rejected");
    assert!(matches!(error, ScenarioParseError::NotUtf8OrJson(_)));
}

/// Block 40.1 "bound nodes, links, steps, ... and duration": exceeding a
/// declared bound is a validation error, not silently truncated.
#[test]
fn exceeding_a_declared_bound_is_rejected() {
    let scenario = load_scenario_json(
        br#"{"schemaVersion": 1, "seed": 1, "nodes": [], "steps": [], "assertions": [],
             "timeoutMs": 100000000000}"#,
    )
    .expect("shape parses");
    let error = scenario
        .validate()
        .expect_err("oversized timeoutMs must be rejected");
    assert!(matches!(
        error,
        ScenarioValidationError::DurationOutOfBounds {
            field: "timeoutMs",
            ..
        }
    ));
}

/// A step naming a node that was never declared is a validation error --
/// never silently ignored or routed nowhere.
#[test]
fn a_step_referencing_an_undeclared_node_is_rejected() {
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "host1"}],
            "steps": [{"atMs": 0, "node": "ghost", "action": {"kind": "exportDiagnostics"}}],
            "assertions": [], "timeoutMs": 100
        }"#,
    )
    .expect("shape parses");
    let error = scenario
        .validate()
        .expect_err("undeclared node reference must be rejected");
    assert!(matches!(
        error,
        ScenarioValidationError::UnknownNode {
            field: "steps[].node",
            ..
        }
    ));
}

/// A command whose production preconditions are not met (here: `submitJoin`
/// with no session ever selected, since Block 40 does not wire live
/// discovery -- see the module doc comment) fails through the real actor,
/// visibly, rather than the runner silently treating it as a success. Note
/// `submit_command`'s own synchronous return (`StepResult::submit_error`)
/// only ever reports a *queue-admission* failure (bad shape, shutdown,
/// full queue) -- exactly as its own doc comment says ("the receipt does
/// not prove command completion"). The actor's own asynchronous rejection
/// is visible only through the recorded `CoreNotification::Error`, which is
/// exactly what `errorCodeObserved` reads.
#[test]
fn a_command_that_is_illegal_in_the_current_state_is_reported_not_swallowed() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1, "seed": 1,
            "nodes": [{"id": "listener1"}],
            "steps": [
                {"atMs": 0, "node": "listener1", "action": {"kind": "selectRole", "role": "listener"}},
                {"atMs": 5, "node": "listener1", "action": {"kind": "submitJoin"}}
            ],
            "assertions": [
                {"kind": "errorCodeObserved", "byMs": 50, "node": "listener1", "code": "invalid_state_transition"}
            ],
            "timeoutMs": 50
        }"#,
    )
    .expect("valid scenario document");

    let report = run_scenario(&lab, &scenario).expect("scenario runs");
    assert!(
        report.step_results[1].submit_error.is_none(),
        "submitJoin is correctly shaped, so it is admitted to the actor's queue \
         synchronously -- its rejection happens asynchronously"
    );
    assert_eq!(
        report.assertion_results[0].outcome,
        AssertionOutcome::Held,
        "the actor's real asynchronous rejection must still be observed"
    );
}

fn underrun_frames_at_most(max_total_missing_frames: u32) -> ScenarioAssertion {
    ScenarioAssertion::UnderrunFramesAtMost {
        by_ms: 1_000,
        node: NodeId("n1".to_owned()),
        max_total_missing_frames,
    }
}

fn underrun_entry(sequence: u64, missing_frames: &str) -> RecordedNotification {
    RecordedNotification {
        sequence,
        kind: RecordedNotificationKind::Diagnostic {
            name: "audio_underrun".to_owned(),
            fields: vec![("missing_frames".to_owned(), missing_frames.to_owned())],
        },
    }
}

/// The ordinary, well-formed path: valid `missing_frames` values are summed
/// and compared against the bound as before.
#[test]
fn underrun_frames_at_most_sums_valid_missing_frames_values() {
    let entries = vec![underrun_entry(0, "3"), underrun_entry(1, "4")];
    assert!(evaluate_assertion(
        &underrun_frames_at_most(7),
        None,
        &entries
    ));
    assert!(!evaluate_assertion(
        &underrun_frames_at_most(6),
        None,
        &entries
    ));
}

/// Block 44 audit fix: a present-but-unparseable `missing_frames` value
/// used to be silently excluded from the sum (`.ok()` on the failed
/// `parse::<u64>()`), understating a real underrun count and letting a
/// regression pass. Confirms it now fails the assertion outright instead of
/// silently treating the malformed entry as zero missing frames.
#[test]
fn underrun_frames_at_most_fails_closed_on_an_unparseable_missing_frames_value() {
    let entries = vec![underrun_entry(0, "3"), underrun_entry(1, "not-a-number")];
    // Even an unbounded threshold must not let a malformed entry pass --
    // the assertion cannot be verified at all, not "verified as fine".
    assert!(!evaluate_assertion(
        &underrun_frames_at_most(u32::MAX),
        None,
        &entries
    ));
}
