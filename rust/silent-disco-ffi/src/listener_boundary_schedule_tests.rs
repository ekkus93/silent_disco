//! The listener schedule through the `UniFFI` object Android uses.

use super::{FfiListenerBoundary, FfiListenerBoundarySchedule, FfiListenerBoundaryWithdrawal};

#[test]
fn the_android_object_presents_pauses_and_resumes_like_the_core() {
    let schedule = FfiListenerBoundarySchedule::new();
    assert!(
        !schedule
            .stream_started("stream-1".to_owned(), 1_000, Some("First".to_owned()))
            .expect("start")
    );
    assert!(
        schedule
            .add("stream-1".to_owned(), 96_000, "Second".to_owned())
            .expect("add")
    );
    assert!(
        !schedule
            .add("stream-2".to_owned(), 48_000, "Other".to_owned())
            .expect("ignored")
    );
    assert_eq!(
        schedule.next_due_local_ms(-500.0).expect("next"),
        Some(3_500.0)
    );
    assert!(schedule.take_due(3_499, -500.0).expect("due").is_empty());
    assert_eq!(
        schedule.take_due(3_500, -500.0).expect("due"),
        vec![FfiListenerBoundary {
            first_sample_index: 96_000,
            display_name: "Second".to_owned(),
        }]
    );
    assert_eq!(
        schedule
            .paused("stream-1".to_owned(), 2_000)
            .expect("pause"),
        Some(FfiListenerBoundaryWithdrawal {
            count: 1,
            track_at_pause: Some("First".to_owned()),
        })
    );
    assert!(
        schedule
            .stream_started("stream-1".to_owned(), 4_000, Some("First".to_owned()))
            .expect("resume")
    );
    assert_eq!(schedule.pending().expect("pending").len(), 1);
    schedule.stream_stopped().expect("stop");
    assert_eq!(schedule.host_start_time_ms().expect("start"), None);
    assert!(schedule.pending().expect("pending").is_empty());
}
