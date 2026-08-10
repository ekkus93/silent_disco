use super::{RecordedNotificationKind, RecordingObserver, ScenarioRecorder, SnapshotSummary};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot, DiagnosticField,
};
use std::time::Duration;

fn fatal_error() -> CoreError {
    CoreError::new(
        CoreErrorCode::ShutdownFailed,
        "shutdown failed",
        ErrorSeverity::Fatal,
        false,
        None,
    )
    .expect("valid error")
}

fn recoverable_error() -> CoreError {
    CoreError::new(
        CoreErrorCode::InvalidArgument,
        "bad argument",
        ErrorSeverity::Error,
        false,
        None,
    )
    .expect("valid error")
}

/// Every recorded notification keeps its own classification and is
/// returned in the exact order it was recorded.
#[test]
fn records_every_notification_kind_in_order() {
    let recorder = ScenarioRecorder::new();
    let observer = RecordingObserver(recorder.clone());

    observer
        .on_notification(CoreNotification::Error(recoverable_error()))
        .expect("record error");
    observer
        .on_notification(CoreNotification::Diagnostic(
            CoreDiagnostic::new(
                "audio_underrun",
                vec![DiagnosticField::new("missing_frames", "5").expect("valid field")],
            )
            .expect("valid diagnostic"),
        ))
        .expect("record diagnostic");

    let entries = recorder.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].sequence, 0);
    assert_eq!(entries[1].sequence, 1);
    assert!(matches!(
        entries[0].kind,
        RecordedNotificationKind::Error { .. }
    ));
    assert!(matches!(
        entries[1].kind,
        RecordedNotificationKind::Diagnostic { .. }
    ));
}

/// Block 40.3 "no unexpected fatal error": only a Fatal-severity error is
/// classified as fatal; a merely recoverable error is not.
#[test]
fn only_fatal_severity_errors_are_classified_as_fatal() {
    let recorder = ScenarioRecorder::new();
    let observer = RecordingObserver(recorder.clone());
    observer
        .on_notification(CoreNotification::Error(recoverable_error()))
        .expect("record");
    observer
        .on_notification(CoreNotification::Error(fatal_error()))
        .expect("record");

    let entries = recorder.entries();
    assert!(!entries[0].kind.is_fatal_error());
    assert!(entries[1].kind.is_fatal_error());
}

/// Once [`super::MAX_RECORDED_NOTIFICATIONS`] is reached, further
/// notifications are counted as dropped rather than silently discarded
/// without a trace, and never grow the retained trace past the bound.
#[test]
fn bounded_recording_reports_drops_instead_of_growing_unboundedly() {
    let recorder = ScenarioRecorder::new();
    let observer = RecordingObserver(recorder.clone());
    for _ in 0..(super::MAX_RECORDED_NOTIFICATIONS + 10) {
        observer
            .on_notification(CoreNotification::Error(recoverable_error()))
            .expect("record");
    }

    assert_eq!(recorder.entries().len(), super::MAX_RECORDED_NOTIFICATIONS);
    assert_eq!(recorder.dropped_count(), 10);
}

/// [`ScenarioRecorder::wait_for_progress`] wakes as soon as a notification
/// arrives on another thread, well inside its bounded timeout -- it never
/// busy-waits for the full timeout when progress genuinely happens.
#[test]
fn wait_for_progress_wakes_on_a_concurrent_notification() {
    let recorder = ScenarioRecorder::new();
    let since = recorder.next_sequence();
    let waiter_recorder = recorder.clone();
    let waiter = std::thread::spawn(move || {
        waiter_recorder.wait_for_progress(since, Duration::from_secs(5))
    });

    // Give the waiter thread a moment to start blocking, then record.
    std::thread::sleep(Duration::from_millis(20));
    let observer = RecordingObserver(recorder.clone());
    observer
        .on_notification(CoreNotification::Error(recoverable_error()))
        .expect("record");

    assert!(waiter.join().expect("waiter thread"));
}

/// A timeout with no progress at all returns `false`, not a panic or hang.
#[test]
fn wait_for_progress_times_out_when_nothing_arrives() {
    let recorder = ScenarioRecorder::new();
    let since = recorder.next_sequence();
    assert!(!recorder.wait_for_progress(since, Duration::from_millis(20)));
}

/// Block 41.1 "snapshot revisions and safe hashes/full bounded snapshots" /
/// Block 41.3 "secret redaction": a `CoreSnapshot` carrying a real host
/// admission secret (`host_draft.invite_code`) must never have that secret
/// reach the recorded, persistable [`SnapshotSummary`] -- checked both on
/// the typed value directly and on its serialized JSON form, since a
/// serialization bug (e.g. an accidentally reintroduced field) would not
/// necessarily be caught by inspecting the Rust struct alone.
#[test]
fn snapshot_summary_never_carries_the_raw_invite_code() {
    let mut snapshot = CoreSnapshot::default();
    snapshot.host_draft.invite_code = Some("top-secret-admission-code".to_owned());
    snapshot.host_draft.session_name = "Alice's House Party".to_owned();

    let observer = RecordingObserver(ScenarioRecorder::new());
    observer
        .on_notification(CoreNotification::Snapshot(snapshot))
        .expect("record snapshot");
    let entries = observer.0.entries();

    let RecordedNotificationKind::Snapshot { summary, .. } = &entries[0].kind else {
        panic!("expected a recorded Snapshot entry");
    };
    let json = serde_json::to_string(summary).expect("summary serializes");

    assert!(!json.contains("top-secret-admission-code"));
    assert!(!json.contains("Alice's House Party"));
    // The approval *mode* (an enum, not a secret) is still present -- proves
    // this is deliberate field-level redaction, not an empty/broken summary.
    assert!(json.contains("hostApprovalMode"));
}

/// [`SnapshotSummary::capture`] is the recorder's own redaction boundary
/// (see its doc comment); calling it directly (as `classify` does) proves
/// the exclusion holds at the source, not only after the round trip through
/// `CoreNotification`/serialization above.
#[test]
fn snapshot_summary_capture_excludes_invite_code_by_construction() {
    let mut snapshot = CoreSnapshot::default();
    snapshot.host_draft.invite_code = Some("another-secret".to_owned());

    let summary = SnapshotSummary::capture(&snapshot);

    // `SnapshotSummary` has no field that could hold `invite_code` at all --
    // this assertion documents that fact by construction rather than
    // grepping a serialized form for a specific string.
    assert_eq!(summary.host_approval_mode, "manual");
    let debug = format!("{summary:?}");
    assert!(!debug.contains("another-secret"));
}
