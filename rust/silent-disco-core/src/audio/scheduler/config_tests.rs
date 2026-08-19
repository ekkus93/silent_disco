//! Construction-validation rejections and `host_to_local_ms` mapping.

use super::test_support::{config, datagram};
use super::{PlaybackScheduler, SchedulerConfigErrorKind, SchedulerPoll};

#[test]
fn maps_host_presentation_time_to_local_time_using_the_configured_offset() {
    let mut scheduler = PlaybackScheduler::new(config(), 50.0).expect("valid scheduler");
    for sequence in 0..=20 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    assert!(matches!(scheduler.poll(949), SchedulerPoll::Waiting { .. }));
    match scheduler.poll(950) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 0),
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn rejects_an_invalid_packet_geometry() {
    let mut cfg = config();
    cfg.sample_rate = 0;

    let error = PlaybackScheduler::new(cfg, 0.0).expect_err("invalid geometry must be rejected");
    assert_eq!(error.kind, SchedulerConfigErrorKind::InvalidPacketDuration);
}

#[test]
fn rejects_zero_samples_per_packet() {
    let mut cfg = config();
    cfg.samples_per_packet = 0;

    let error =
        PlaybackScheduler::new(cfg, 0.0).expect_err("invalid samples per packet must be rejected");
    assert_eq!(
        error.kind,
        SchedulerConfigErrorKind::InvalidSamplesPerPacket
    );
}

#[test]
fn rejects_water_marks_that_are_not_strictly_ordered() {
    let mut cfg = config();
    cfg.low_water_ms = 700;
    cfg.high_water_ms = 200;

    let error =
        PlaybackScheduler::new(cfg, 0.0).expect_err("inverted water marks must be rejected");
    assert_eq!(error.kind, SchedulerConfigErrorKind::InvalidWaterMarks);
}

#[test]
fn host_to_local_time_mapping_stays_correct_over_multi_year_sessions_and_never_panics() {
    let ten_years_ms: u64 = 10 * 365 * 24 * 60 * 60 * 1_000;
    assert_eq!(
        super::scheduler::host_to_local_ms(ten_years_ms, 0.0),
        ten_years_ms
    );
    assert_eq!(
        super::scheduler::host_to_local_ms(ten_years_ms, 100.0),
        ten_years_ms - 100
    );

    // Must not panic even at the extreme end of the monotonic millisecond range.
    let _ = super::scheduler::host_to_local_ms(u64::MAX, 0.0);
    let _ = super::scheduler::host_to_local_ms(u64::MAX, -1_000_000.0);
    let _ = super::scheduler::host_to_local_ms(0, f64::MAX);
}

#[test]
fn rejects_a_non_positive_hard_resync_threshold() {
    let mut cfg = config();
    cfg.hard_resync_threshold_ms = 0.0;

    let error =
        PlaybackScheduler::new(cfg, 0.0).expect_err("non-positive threshold must be rejected");
    assert_eq!(
        error.kind,
        SchedulerConfigErrorKind::InvalidHardResyncThreshold
    );
}

#[test]
fn rejects_a_skip_threshold_that_no_observable_gap_could_reach() {
    let mut cfg = config();
    cfg.max_reorder_window = 8;
    cfg.concealment_skip_threshold_packets = 8;

    let error = PlaybackScheduler::new(cfg, 0.0)
        .expect_err("a threshold at the reorder window must be rejected");
    assert_eq!(
        error.kind,
        SchedulerConfigErrorKind::InvalidConcealmentSkipThreshold
    );
}
