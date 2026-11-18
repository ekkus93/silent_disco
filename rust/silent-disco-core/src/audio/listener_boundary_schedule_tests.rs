use super::{
    ListenerBoundary, ListenerBoundarySchedule, ListenerBoundaryWithdrawal,
    MAX_APPLIED_LISTENER_BOUNDARIES, MAX_PENDING_LISTENER_BOUNDARIES,
};

const STREAM: &str = "stream-1";

/// 48 kHz: sample 48 000 is one second after the stream's host start.
fn boundary(seconds: u64, name: &str) -> ListenerBoundary {
    ListenerBoundary {
        first_sample_index: seconds * 48_000,
        display_name: name.to_owned(),
    }
}

fn names(boundaries: &[ListenerBoundary]) -> Vec<&str> {
    boundaries
        .iter()
        .map(|boundary| boundary.display_name.as_str())
        .collect()
}

fn started(host_start_time_ms: u64) -> ListenerBoundarySchedule {
    let mut schedule = ListenerBoundarySchedule::new();
    assert!(!schedule.stream_started(STREAM, host_start_time_ms, Some("First".to_owned())));
    schedule
}

#[test]
fn a_boundary_is_presented_at_its_host_time_mapped_onto_the_local_clock() {
    let mut schedule = started(1_000);
    assert!(schedule.add(STREAM, boundary(2, "Second")).expect("add"));
    // Host time 3 000 ms is local 3 500 ms with a -500 ms offset.
    assert_eq!(schedule.next_due_local_ms(-500.0), Some(3_500.0));
    assert!(schedule.take_due(3_499, -500.0).is_empty());
    assert_eq!(names(&schedule.take_due(3_500, -500.0)), ["Second"]);
    assert_eq!(schedule.pending().len(), 0);
    assert_eq!(schedule.applied().len(), 1);
    // A positive offset (the host clock ahead) moves it earlier.
    let mut ahead = started(1_000);
    ahead.add(STREAM, boundary(2, "Second")).expect("add");
    assert_eq!(names(&ahead.take_due(2_750, 250.0)), ["Second"]);
}

#[test]
fn a_non_packet_aligned_boundary_uses_its_exact_sample_time() {
    let mut schedule = started(5_000);
    schedule
        .add(
            STREAM,
            ListenerBoundary {
                first_sample_index: 241,
                display_name: "Odd".to_owned(),
            },
        )
        .expect("add");
    let local = schedule.next_due_local_ms(0.0).expect("due");
    assert!(
        (local - (5_000.0 + 241_000.0 / 48_000.0)).abs() < 1e-9,
        "{local}"
    );
    // 5 005.0208 ms: not presented at 5 005, presented at 5 006.
    assert!(schedule.take_due(5_005, 0.0).is_empty());
    assert_eq!(names(&schedule.take_due(5_006, 0.0)), ["Odd"]);
}

#[test]
fn nothing_is_due_before_a_stream_start_or_after_stop() {
    let mut schedule = ListenerBoundarySchedule::new();
    assert!(
        !schedule
            .add(STREAM, boundary(1, "Second"))
            .expect("no stream yet")
    );
    assert!(schedule.take_due(i64::MAX, 0.0).is_empty());

    let mut schedule = started(0);
    schedule.add(STREAM, boundary(1, "Second")).expect("add");
    schedule.stream_stopped();
    assert_eq!(schedule.host_start_time_ms(), None);
    assert!(schedule.take_due(i64::MAX, 0.0).is_empty());
    assert_eq!(schedule.next_due_local_ms(0.0), None);
}

#[test]
fn a_boundary_for_another_stream_is_ignored() {
    let mut schedule = started(0);
    assert!(
        !schedule
            .add("stream-2", boundary(1, "Other"))
            .expect("ignored")
    );
    assert_eq!(schedule.pending().len(), 0);
    assert_eq!(schedule.paused("stream-2", 0), None);
}

#[test]
fn several_overdue_boundaries_come_back_together_in_sample_order() {
    let mut schedule = started(0);
    schedule.add(STREAM, boundary(3, "Fourth")).expect("add");
    schedule.add(STREAM, boundary(1, "Second")).expect("add");
    schedule.add(STREAM, boundary(2, "Third")).expect("add");
    assert_eq!(names(&schedule.take_due(2_500, 0.0)), ["Second", "Third"]);
    assert_eq!(names(&schedule.take_due(10_000, 0.0)), ["Fourth"]);
}

#[test]
fn a_boundary_sent_again_replaces_the_waiting_copy() {
    let mut schedule = started(0);
    schedule.add(STREAM, boundary(1, "Old name")).expect("add");
    schedule
        .add(STREAM, boundary(1, "New name"))
        .expect("replay");
    assert_eq!(schedule.pending().len(), 1);
    assert_eq!(names(&schedule.take_due(1_000, 0.0)), ["New name"]);
}

#[test]
fn a_pause_takes_back_boundaries_presented_after_it_and_names_the_track_at_the_pause() {
    let mut schedule = started(0);
    schedule.add(STREAM, boundary(1, "Second")).expect("add");
    schedule.add(STREAM, boundary(3, "Third")).expect("add");
    assert_eq!(names(&schedule.take_due(4_000, 0.0)), ["Second", "Third"]);

    assert_eq!(
        schedule.paused(STREAM, 2_000),
        Some(ListenerBoundaryWithdrawal {
            count: 1,
            track_at_pause: Some("Second".to_owned()),
        })
    );
    assert_eq!(
        schedule
            .pending()
            .map(|b| b.first_sample_index)
            .collect::<Vec<_>>(),
        [144_000]
    );
    assert_eq!(
        schedule.paused(STREAM, 500),
        Some(ListenerBoundaryWithdrawal {
            count: 1,
            track_at_pause: Some("First".to_owned()),
        })
    );
    assert_eq!(schedule.paused(STREAM, 500), None);
    // A boundary exactly at the pause instant was heard before it.
    let mut exact = started(0);
    exact.add(STREAM, boundary(1, "Second")).expect("add");
    let _ = exact.take_due(1_000, 0.0);
    assert_eq!(exact.paused(STREAM, 1_000), None);
}

#[test]
fn a_resumed_stream_keeps_its_pending_boundaries_and_a_new_one_drops_them() {
    let mut schedule = started(0);
    schedule.add(STREAM, boundary(1, "Second")).expect("add");
    let _ = schedule.take_due(0, 0.0);

    assert!(schedule.stream_started(STREAM, 5_000, Some("First".to_owned())));
    assert_eq!(schedule.pending().len(), 1);
    // The resume's start time is the one boundaries now count from.
    assert_eq!(schedule.next_due_local_ms(0.0), Some(6_000.0));

    assert!(!schedule.stream_started("stream-2", 9_000, Some("Other".to_owned())));
    assert_eq!(schedule.pending().len(), 0);
    assert_eq!(schedule.stream_id(), Some("stream-2"));
}

#[test]
fn a_same_stream_start_after_stop_is_still_a_resume_of_that_stream() {
    let mut schedule = started(0);
    schedule.stream_stopped();
    assert!(schedule.stream_started(STREAM, 7_000, None));
    assert!(schedule.add(STREAM, boundary(1, "Second")).expect("add"));
}

#[test]
fn pending_and_presented_boundaries_are_bounded() {
    let mut schedule = started(0);
    for index in 0..MAX_PENDING_LISTENER_BOUNDARIES {
        let sample = u64::try_from(index).expect("index") * 960;
        schedule
            .add(
                STREAM,
                ListenerBoundary {
                    first_sample_index: sample,
                    display_name: format!("track {index}"),
                },
            )
            .expect("within the bound");
    }
    let refused = schedule
        .add(
            STREAM,
            ListenerBoundary {
                first_sample_index: u64::MAX,
                display_name: "over".to_owned(),
            },
        )
        .expect_err("full");
    assert!(
        refused.message.contains("already waiting"),
        "{}",
        refused.message
    );
    // Replacing one already waiting is still accepted.
    schedule
        .add(STREAM, boundary(0, "replaced"))
        .expect("replace");

    let presented = schedule.take_due(i64::MAX, 0.0);
    assert_eq!(presented.len(), MAX_PENDING_LISTENER_BOUNDARIES);
    assert_eq!(schedule.applied().len(), MAX_APPLIED_LISTENER_BOUNDARIES);
    // The oldest are the ones forgotten.
    assert_eq!(
        schedule.applied().next().map(|b| b.first_sample_index),
        Some(
            u64::try_from(MAX_PENDING_LISTENER_BOUNDARIES - MAX_APPLIED_LISTENER_BOUNDARIES)
                .expect("count")
                * 960
        )
    );
}

#[test]
fn after_forgetting_the_stream_the_same_stream_id_starts_fresh() {
    let mut schedule = started(0);
    schedule.add(STREAM, boundary(1, "Second")).expect("add");
    schedule.forget_stream();
    assert_eq!(schedule.stream_id(), None);
    assert!(
        !schedule
            .add(STREAM, boundary(2, "Ignored"))
            .expect("no stream")
    );
    assert!(!schedule.stream_started(STREAM, 0, None), "not a resume");
    assert_eq!(schedule.pending().len(), 0);
}

/// The host applies a boundary at its integer-millisecond deadline (the host
/// schedule); the listener presents it at the exact sample time. Mapped onto
/// the listener clock, the listener is never early and less than 1 ms late.
#[test]
fn host_and_listener_deadlines_agree_across_the_sample_timeline() {
    let host_start = 73_000;
    let offset = 186.25;
    let host = crate::audio::HostBoundarySchedule::at_canonical_rate(host_start);
    let mut listener = started(host_start);
    for sample in [0, 1, 241, 24_001, 48_000, 1_234_567, 17_280_000] {
        listener
            .add(
                STREAM,
                ListenerBoundary {
                    first_sample_index: sample,
                    display_name: String::new(),
                },
            )
            .expect("add");
        let listener_ms = listener.local_time_ms(sample, offset).expect("started");
        #[allow(clippy::cast_precision_loss)]
        let host_ms_on_listener_clock = host.due_at_ms(sample, 0) as f64 - offset;
        let late_by = listener_ms - host_ms_on_listener_clock;
        assert!((0.0..1.0).contains(&late_by), "sample {sample}: {late_by}");
    }
}
