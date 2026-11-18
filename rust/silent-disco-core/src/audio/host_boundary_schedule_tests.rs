use super::{
    HostBoundarySchedule, HostBoundaryScheduleErrorKind, MAX_PENDING_HOST_BOUNDARIES,
    PendingHostBoundary,
};

const START_MS: u64 = 10_000;
const RATE: u32 = 48_000;

fn boundary(entry_id: &str, first_sample_index: u64) -> PendingHostBoundary {
    PendingHostBoundary {
        entry_id: entry_id.to_owned(),
        first_sample_index,
        display_name: format!("{entry_id} title"),
    }
}

fn ids(boundaries: &[PendingHostBoundary]) -> Vec<&str> {
    boundaries
        .iter()
        .map(|boundary| boundary.entry_id.as_str())
        .collect()
}

#[test]
fn the_deadline_is_start_plus_sample_time_rounded_down_plus_the_pause_offset() {
    let schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    // The positions the two former host implementations were compared at,
    // including boundaries that do not fall on a packet or millisecond edge.
    for (sample, expected_ms) in [
        (0, 0),
        (47, 0),
        (48, 1),
        (241, 5),
        (960, 20),
        (48_000, 1_000),
        (2_646_001, 55_125),
    ] {
        assert_eq!(
            schedule.due_at_ms(sample, 0),
            START_MS + expected_ms,
            "sample {sample}"
        );
        assert_eq!(
            schedule.due_at_ms(sample, 3_217),
            START_MS + expected_ms + 3_217,
            "sample {sample} after a 3217 ms pause"
        );
    }
    let cd_rate = HostBoundarySchedule::new(START_MS, 44_100).expect("44.1 kHz");
    assert_eq!(cd_rate.due_at_ms(44_099, 0), START_MS + 999);
    assert_eq!(cd_rate.due_at_ms(44_100, 0), START_MS + 1_000);
}

#[test]
fn a_boundary_is_applied_exactly_at_its_deadline_and_not_a_millisecond_before() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    schedule
        .schedule(boundary("next", 480_000))
        .expect("schedule");
    assert_eq!(schedule.next_due_at_ms(0), Some(START_MS + 10_000));
    assert!(schedule.take_due(START_MS + 9_999, 0).is_empty());
    assert_eq!(ids(&schedule.take_due(START_MS + 10_000, 0)), ["next"]);
    assert!(schedule.is_empty());
    assert_eq!(schedule.next_due_at_ms(0), None);
}

#[test]
fn a_pause_moves_every_pending_boundary_by_exactly_the_offset_the_audio_moved() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    schedule
        .schedule(boundary("second", 96_000))
        .expect("second");
    schedule
        .schedule(boundary("third", 192_000))
        .expect("third");
    // The stream paused for 1 234 ms before either was due: both move by
    // that offset, whichever side of the pause they were scheduled on.
    let offset = 1_234;
    assert!(schedule.take_due(START_MS + 2_000, offset).is_empty());
    assert!(
        schedule
            .take_due(START_MS + 2_000 + offset - 1, offset)
            .is_empty()
    );
    assert_eq!(
        ids(&schedule.take_due(START_MS + 2_000 + offset, offset)),
        ["second"]
    );
    // A second pause adds to the first.
    let offset = offset + 766;
    assert!(
        schedule
            .take_due(START_MS + 4_000 + offset - 1, offset)
            .is_empty()
    );
    assert_eq!(
        ids(&schedule.take_due(START_MS + 4_000 + offset, offset)),
        ["third"]
    );
}

#[test]
fn several_overdue_boundaries_come_back_together_in_sample_order() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    for (index, entry) in ["a", "b", "c", "d"].into_iter().enumerate() {
        let sample = 48_000 * (u64::try_from(index).expect("index") + 1);
        schedule
            .schedule(boundary(entry, sample))
            .expect("schedule");
    }
    assert_eq!(
        ids(&schedule.take_due(START_MS + 3_500, 0)),
        ["a", "b", "c"]
    );
    assert_eq!(
        schedule
            .pending()
            .map(|b| b.entry_id.as_str())
            .collect::<Vec<_>>(),
        ["d"]
    );
}

#[test]
fn a_boundary_that_does_not_follow_the_last_one_is_refused_even_after_it_was_applied() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    schedule.schedule(boundary("first", 48_000)).expect("first");
    let same = schedule
        .schedule(boundary("again", 48_000))
        .expect_err("same sample");
    assert_eq!(same.kind, HostBoundaryScheduleErrorKind::OutOfOrder);
    let earlier = schedule
        .schedule(boundary("earlier", 47_999))
        .expect_err("earlier");
    assert_eq!(earlier.kind, HostBoundaryScheduleErrorKind::OutOfOrder);

    assert_eq!(ids(&schedule.take_due(START_MS + 1_000, 0)), ["first"]);
    let replayed = schedule
        .schedule(boundary("first", 48_000))
        .expect_err("applied already");
    assert_eq!(replayed.kind, HostBoundaryScheduleErrorKind::OutOfOrder);
    assert!(schedule.is_empty(), "a refused boundary is not queued");
    schedule
        .schedule(boundary("second", 48_001))
        .expect("a later boundary still schedules");
}

#[test]
fn withdrawing_from_the_first_unsent_sample_keeps_only_boundaries_inside_the_sent_audio() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    schedule.schedule(boundary("heard", 48_000)).expect("heard");
    schedule
        .schedule(boundary("at-the-tail", 96_000))
        .expect("at the tail");
    schedule
        .schedule(boundary("beyond", 144_000))
        .expect("beyond");
    // The last packet sent ended at sample 96 000: a boundary there starts a
    // track no listener received any audio for.
    let withdrawn = schedule.withdraw_from(96_000);
    assert_eq!(ids(&withdrawn), ["at-the-tail", "beyond"]);
    assert_eq!(
        schedule
            .pending()
            .map(|b| b.entry_id.as_str())
            .collect::<Vec<_>>(),
        ["heard"]
    );
    // The tail deadline passing no longer advances the queue to them.
    assert_eq!(ids(&schedule.take_due(START_MS + 3_000, 0)), ["heard"]);
    assert!(schedule.withdraw_from(0).is_empty());
}

#[test]
fn clear_drops_every_pending_boundary() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    schedule.schedule(boundary("a", 48_000)).expect("a");
    schedule.schedule(boundary("b", 96_000)).expect("b");
    schedule.clear();
    assert!(schedule.is_empty());
    assert!(schedule.take_due(u64::MAX, 0).is_empty());
}

#[test]
fn the_schedule_is_bounded_and_a_refused_boundary_changes_nothing() {
    let mut schedule = HostBoundarySchedule::new(START_MS, RATE).expect("schedule");
    for index in 1..=MAX_PENDING_HOST_BOUNDARIES {
        let sample = u64::try_from(index).expect("index") * 960;
        schedule
            .schedule(boundary("entry", sample))
            .expect("within the bound");
    }
    let full = schedule
        .schedule(boundary("over", u64::MAX))
        .expect_err("full");
    assert_eq!(full.kind, HostBoundaryScheduleErrorKind::Full);
    assert_eq!(schedule.pending().len(), MAX_PENDING_HOST_BOUNDARIES);
    // The refused boundary did not move the order check forward.
    let _ = schedule.take_due(u64::MAX, 0);
    schedule
        .schedule(boundary("after", u64::MAX - 1))
        .expect("the refused sample index was not recorded");
}

#[test]
fn a_zero_sample_rate_is_refused_and_extreme_inputs_saturate() {
    let zero = HostBoundarySchedule::new(START_MS, 0).expect_err("zero rate");
    assert_eq!(zero.kind, HostBoundaryScheduleErrorKind::InvalidSampleRate);
    let schedule = HostBoundarySchedule::new(u64::MAX - 5, 1).expect("schedule");
    assert_eq!(schedule.due_at_ms(u64::MAX, u64::MAX), u64::MAX);
    assert_eq!(schedule.due_at_ms(0, 3), u64::MAX - 2);
}

#[test]
fn the_canonical_constructor_uses_the_network_rate() {
    let schedule = HostBoundarySchedule::at_canonical_rate(START_MS);
    assert_eq!(schedule.sample_rate_hz(), super::CANONICAL_SAMPLE_RATE_HZ);
    assert_eq!(schedule.host_start_time_ms(), START_MS);
    assert_eq!(schedule.due_at_ms(48_000, 7), START_MS + 1_007);
}
