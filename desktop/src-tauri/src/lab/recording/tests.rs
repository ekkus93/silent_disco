use super::{
    Divergence, MAX_RECORDING_FILE_BYTES, RECORDING_FORMAT_VERSION, RecordedCoreVersion,
    RecordingLoadError, RecordingSaveError, ScenarioRecording, first_divergence,
    first_trace_divergence, load_recording_from_path, load_recording_json, save_recording_to_path,
};
use crate::lab::recorder::{RecordedNotification, RecordedNotificationKind};
use crate::lab::scenario::{
    AssertionOutcome, AssertionResult, NodeId, ScenarioOutcome, ScenarioReport, ScenarioTrace,
    StepResult, StepSettlement,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// `NodeId`'s inner field is private to `scenario`, so a plain test module
/// elsewhere in `lab` constructs one the same way any other out-of-crate
/// consumer of a persisted recording would: by deserializing it, exactly
/// like [`super::load_recording_json`] itself does for a whole recording.
fn node_id(value: &str) -> NodeId {
    serde_json::from_str(&format!("{value:?}")).expect("valid node id")
}

fn empty_report() -> ScenarioReport {
    ScenarioReport {
        schema_version: 1,
        seed: 7,
        outcome: ScenarioOutcome::Completed,
        final_time_ms: 100,
        step_results: Vec::new(),
        assertion_results: Vec::new(),
    }
}

fn empty_trace() -> ScenarioTrace {
    ScenarioTrace {
        clock_advances: Vec::new(),
        node_notifications: Vec::new(),
        transport_trace: crate::lab::fault::trace::TransportTrace::default(),
    }
}

fn sample_recording() -> ScenarioRecording {
    ScenarioRecording {
        recording_format_version: RECORDING_FORMAT_VERSION,
        scenario_schema_version: 1,
        protocol_version: 2,
        core_version: RecordedCoreVersion {
            major: 0,
            minor: 1,
            patch: 0,
        },
        seed: 7,
        report: empty_report(),
        trace: empty_trace(),
    }
}

fn step_result(index: usize, submit_error: Option<&str>) -> StepResult {
    StepResult {
        index,
        at_ms: 0,
        node: node_id("host1"),
        submit_error: submit_error.map(str::to_owned),
        settlement: StepSettlement::Settled,
    }
}

fn assertion_result(outcome: AssertionOutcome) -> AssertionResult {
    AssertionResult {
        kind: "lifecycleReached".to_owned(),
        node: node_id("host1"),
        by_ms: 50,
        outcome,
    }
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-lab-recording-{}-{sequence}",
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

/// Block 41.3 "bounded output": a recording whose content genuinely exceeds
/// [`MAX_RECORDING_FILE_BYTES`] is rejected by `to_bounded_json`, not
/// silently written unbounded.
#[test]
fn oversized_recording_is_rejected_before_being_written() {
    let mut recording = sample_recording();
    // Comfortably exceeds `MAX_RECORDING_FILE_BYTES` once serialized
    // (~100 bytes of JSON per entry) without needing to allocate anything
    // close to the byte bound itself in Rust-side struct memory.
    recording.report.assertion_results =
        vec![assertion_result(AssertionOutcome::Held); MAX_RECORDING_FILE_BYTES / 30];

    let error = recording
        .to_bounded_json()
        .expect_err("an oversized recording must be rejected, not truncated or accepted");
    assert!(matches!(error, RecordingSaveError::TooLarge { .. }));
}

/// Block 41.3 "bounded output", input side: a raw byte slice larger than
/// the bound is rejected before JSON parsing is even attempted (mirrors
/// `scenario::load_scenario_json`'s own "check the length first"
/// discipline).
#[test]
fn oversized_file_bytes_are_rejected_before_parsing() {
    let oversized = vec![b'a'; MAX_RECORDING_FILE_BYTES + 1];
    let error = load_recording_json(&oversized).expect_err("oversized input must be rejected");
    assert!(matches!(error, RecordingLoadError::TooLarge { .. }));
}

/// Block 41.3 "truncated recording rejected": cutting a valid recording's
/// serialized bytes in half produces a bounded parse error, never a panic.
#[test]
fn truncated_recording_bytes_are_rejected_not_a_panic() {
    let recording = sample_recording();
    let bytes = recording.to_bounded_json().expect("encodes");
    let truncated = &bytes[..bytes.len() / 2];

    let error = load_recording_json(truncated).expect_err("truncated bytes must be rejected");
    assert!(matches!(error, RecordingLoadError::Malformed(_)));
}

/// A format-v2 recording cannot omit the transport evidence that v2 was
/// introduced to carry. Missing evidence is malformed input, not an empty
/// successful trace synthesized by a serde default.
#[test]
fn format_v2_recording_missing_transport_trace_is_rejected() {
    let recording = sample_recording();
    let mut document = serde_json::to_value(&recording).expect("recording converts to JSON value");
    let trace = document
        .get_mut("trace")
        .and_then(serde_json::Value::as_object_mut)
        .expect("recording trace is a JSON object");
    assert!(trace.remove("transportTrace").is_some());
    let bytes = serde_json::to_vec(&document).expect("modified recording encodes");

    let error = load_recording_json(&bytes)
        .expect_err("format v2 without transportTrace must fail structurally");
    assert!(matches!(error, RecordingLoadError::Malformed(_)));
}

/// Arbitrary non-JSON bytes are a bounded error too, not a panic --
/// complementing the truncation case above with a completely unstructured
/// input.
#[test]
fn arbitrary_binary_input_is_a_bounded_error_not_a_panic() {
    let garbage: Vec<u8> = (0_u8..=255).collect();
    let error = load_recording_json(&garbage).expect_err("garbage bytes must be rejected");
    assert!(matches!(error, RecordingLoadError::Malformed(_)));
}

/// Block 41's own acceptance criterion, literally: a recording saved to a
/// file and loaded back later reproduces exactly what was saved.
#[test]
fn saving_then_loading_a_recording_reproduces_it_exactly() {
    let directory = TestDirectory::new();
    let path = directory.0.join("recording.json");
    let mut recording = sample_recording();
    recording.report.step_results = vec![step_result(0, None)];
    recording.report.assertion_results = vec![assertion_result(AssertionOutcome::Held)];

    save_recording_to_path(&recording, &path).expect("save succeeds");
    let loaded = load_recording_from_path(&path).expect("load succeeds");

    assert_eq!(loaded, recording);
}

/// Two structurally identical reports have no divergence at all.
#[test]
fn identical_reports_have_no_divergence() {
    let mut report = empty_report();
    report.step_results = vec![step_result(0, None)];
    report.assertion_results = vec![assertion_result(AssertionOutcome::Held)];
    let replayed = report.clone();

    assert_eq!(first_divergence(&report, &replayed), None);
}

/// The first differing step result is reported, even when a later step
/// also differs -- "first meaningful event", not every difference.
#[test]
fn first_divergence_reports_the_first_differing_step_not_a_later_one() {
    let mut recorded = empty_report();
    recorded.step_results = vec![step_result(0, None), step_result(1, None)];
    let mut replayed = recorded.clone();
    replayed.step_results[0].submit_error = Some("regressed".to_owned());
    replayed.step_results[1].submit_error = Some("also regressed".to_owned());

    let divergence = first_divergence(&recorded, &replayed).expect("must diverge");
    assert!(matches!(
        divergence,
        Divergence::StepResultMismatch { index: 0, .. }
    ));
}

/// A step result changing is detected even when every assertion result
/// still matches -- the divergence is not masked by comparing assertions
/// first.
#[test]
fn a_changed_step_result_diverges_before_assertions_are_even_compared() {
    let mut recorded = empty_report();
    recorded.step_results = vec![step_result(0, None)];
    recorded.assertion_results = vec![assertion_result(AssertionOutcome::Held)];
    let mut replayed = recorded.clone();
    replayed.step_results[0].settlement = StepSettlement::TimedOut;

    let divergence = first_divergence(&recorded, &replayed).expect("must diverge");
    assert!(matches!(
        divergence,
        Divergence::StepResultMismatch { index: 0, .. }
    ));
}

#[test]
fn first_divergence_reports_a_changed_assertion_outcome() {
    let mut recorded = empty_report();
    recorded.assertion_results = vec![assertion_result(AssertionOutcome::Held)];
    let mut replayed = recorded.clone();
    replayed.assertion_results[0].outcome = AssertionOutcome::TimedOut;

    let divergence = first_divergence(&recorded, &replayed).expect("must diverge");
    assert!(matches!(
        divergence,
        Divergence::AssertionResultMismatch { index: 0, .. }
    ));
}

#[test]
fn first_divergence_reports_a_changed_step_count() {
    let recorded = empty_report();
    let mut replayed = empty_report();
    replayed.step_results = vec![step_result(0, None)];

    let divergence = first_divergence(&recorded, &replayed).expect("must diverge");
    assert!(matches!(
        divergence,
        Divergence::DifferentStepCount {
            recorded: 0,
            replayed: 1
        }
    ));
}

#[test]
fn first_divergence_reports_a_changed_overall_outcome_when_nothing_else_differs() {
    let mut recorded = empty_report();
    recorded.outcome = ScenarioOutcome::Completed;
    let mut replayed = empty_report();
    replayed.outcome = ScenarioOutcome::TimedOut;

    let divergence = first_divergence(&recorded, &replayed).expect("must diverge");
    assert!(matches!(divergence, Divergence::DifferentOutcome { .. }));
}

#[test]
fn transport_trace_overflow_difference_is_a_replay_divergence() {
    let recorded = empty_trace();
    let mut replayed = empty_trace();
    replayed.transport_trace.dropped_count = 1;

    let divergence = first_trace_divergence(&recorded, &replayed)
        .expect("different transport evidence must diverge");
    assert!(matches!(
        divergence,
        Divergence::TransportOverflowMismatch {
            recorded: 0,
            replayed: 1
        }
    ));
}

#[test]
fn changed_node_notification_is_a_replay_divergence_even_when_report_matches() {
    let mut recorded = empty_trace();
    recorded.node_notifications = vec![(
        "host1".to_owned(),
        vec![RecordedNotification {
            sequence: 0,
            kind: RecordedNotificationKind::Effect {
                name: "startAdvertising".to_owned(),
            },
        }],
    )];
    let mut replayed = recorded.clone();
    replayed.node_notifications[0].1[0].kind = RecordedNotificationKind::Effect {
        name: "stopAdvertising".to_owned(),
    };

    let divergence =
        first_trace_divergence(&recorded, &replayed).expect("changed notification must diverge");
    assert!(matches!(
        divergence,
        Divergence::NotificationMismatch {
            node,
            index: 0,
            ..
        } if node == "host1"
    ));
}
