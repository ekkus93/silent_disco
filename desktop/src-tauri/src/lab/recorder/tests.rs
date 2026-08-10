use super::{RecordedNotificationKind, RecordingObserver, ScenarioRecorder};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CoreDiagnostic, CoreNotification, CoreObserver, DiagnosticField};
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
