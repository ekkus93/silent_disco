//! Startup buffering, waiting, buffer health, the rebuffer-vs-startup target
//! family, lifecycle/stop, and host-start-time re-anchoring.

use super::test_support::{HOST_START_MS, PACKET_DURATION_MS, config, datagram};
use super::{BufferHealth, PlaybackScheduler, SchedulerPoll};

#[test]
fn remains_buffering_until_the_startup_target_is_reached() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    scheduler.submit_packet(datagram(0, 1)).expect("accepted");

    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Buffering { buffered_ms } => assert_eq!(buffered_ms, 0),
        other => panic!("expected Buffering, got {other:?}"),
    }
}

#[test]
fn transitions_to_playing_and_delivers_frames_once_the_startup_target_is_reached() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    for sequence in 0..=20 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { frame, .. } => {
            assert_eq!(frame.sequence, 0);
            assert!(!frame.concealed);
        }
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn returns_waiting_when_polled_before_the_next_presentation_deadline() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    for sequence in 0..=20 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }
    let _ = scheduler.poll(HOST_START_MS);

    match scheduler.poll(HOST_START_MS + 1) {
        SchedulerPoll::Waiting { .. } => {}
        other => panic!("expected Waiting, got {other:?}"),
    }
}

#[test]
fn reports_low_buffer_health_once_span_drops_below_the_low_water_mark() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 40;
    cfg.low_water_ms = 30;
    cfg.high_water_ms = 100;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    for sequence in 0..=2 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { buffer_health, .. } => assert_eq!(buffer_health, BufferHealth::Low),
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn reports_high_buffer_health_once_span_exceeds_the_high_water_mark() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 40;
    cfg.low_water_ms = 10;
    cfg.high_water_ms = 50;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    for sequence in 0..=5 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { buffer_health, .. } => {
            assert_eq!(buffer_health, BufferHealth::High);
        }
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn reanchoring_the_start_time_moves_the_expected_presentation_deadline_forward() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    // Mirrors a host that re-broadcast StreamStart with its outgoing
    // presentation timeline shifted forward by 500ms to account for elapsed
    // pause time -- the scheduler must be told the same new anchor so it
    // keeps expecting sequence 0 at the moment the host now actually sends
    // it.
    scheduler.set_host_start_time_ms(HOST_START_MS + 500);
    for sequence in 0..=20 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    // The shift moved sequence 0's expected deadline from HOST_START_MS to
    // HOST_START_MS + 500, so polling at the original anchor is still early.
    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::Waiting { .. }
    ));
    match scheduler.poll(HOST_START_MS + 500) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 0),
        other => panic!("expected Frame, got {other:?}"),
    }
}

/// Drives a scheduler to `Playing`, drains everything it buffered, then
/// forces a mid-stream rebuffer and supplies exactly 100ms of span.
/// Whether it resumes from that is decided solely by `rebuffer_target_ms`.
///
/// Note `buffered_span_ms` is last-minus-first, so N packets span
/// (N-1) * `PACKET_DURATION_MS` -- 21 packets are needed for 400ms.
fn scheduler_after_rebuffer_with_100ms_buffered(rebuffer_target_ms: u64) -> PlaybackScheduler {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 400;
    cfg.rebuffer_target_ms = rebuffer_target_ms;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    for sequence in 0..=20 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }
    assert!(
        matches!(scheduler.poll(HOST_START_MS), SchedulerPoll::Frame { .. }),
        "400ms of span must satisfy the 400ms startup target"
    );
    // Drain the 20 packets still buffered so the rebuffer starts empty.
    for tick in 1..=20 {
        let _ = scheduler.poll(HOST_START_MS + tick * u64::from(PACKET_DURATION_MS));
    }

    scheduler.rebuffer(0.0);
    // Six packets span 5 * 20ms = 100ms.
    for sequence in 21..=26 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }
    scheduler
}

/// The startup target governs a stream's first start only. A mid-stream
/// recovery must not have to rebuild it: the span rebuilds at 1x real time,
/// so a large target *is* the length of an audible hole in audio the
/// listener is already hearing.
#[test]
fn a_mid_stream_rebuffer_resumes_on_the_rebuffer_target_not_the_startup_target() {
    let mut scheduler = scheduler_after_rebuffer_with_100ms_buffered(100);
    assert!(
        matches!(
            scheduler.poll(HOST_START_MS + 420),
            SchedulerPoll::Frame { .. }
        ),
        "100ms buffered must satisfy a 100ms rebuffer target, without rebuilding the 400ms \
         startup span"
    );
}

/// Guards the regression this separation exists to prevent: with the
/// rebuffer target left equal to the startup target (the old behaviour),
/// the very same recovery is still silent.
#[test]
fn an_equal_rebuffer_target_reproduces_the_long_recovery() {
    let mut scheduler = scheduler_after_rebuffer_with_100ms_buffered(400);
    assert!(
        matches!(
            scheduler.poll(HOST_START_MS + 420),
            SchedulerPoll::Buffering { .. }
        ),
        "with the targets equal the recovery must still be rebuilding the full startup span"
    );
}

/// A rebuffer target deeper than the startup target is meaningless -- a
/// recovery cannot need more cushion than the stream's own first start --
/// so it is clamped. Without this, lowering only the startup target (as
/// several callers do) would silently buy a *longer* recovery.
#[test]
fn the_rebuffer_target_never_exceeds_the_startup_target() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 0;
    cfg.rebuffer_target_ms = 400;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::Frame { .. }
    ));
    scheduler.rebuffer(0.0);
    scheduler.submit_packet(datagram(1, 1)).expect("accepted");
    match scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 1),
        other => panic!("a zero startup target must clamp the rebuffer target, got {other:?}"),
    }
}

#[test]
fn stop_is_explicit_and_idempotent() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    scheduler.stop();
    assert!(scheduler.is_stopped());
    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::Stopped
    ));

    scheduler.stop();
    assert!(scheduler.is_stopped());
}
