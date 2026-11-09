//! A listener's own pause holds the buffered audio and releases nothing. A
//! resume continues with the first packet the listener has not yet played.

use super::test_support::HOST_START_MS;
use super::{PlaybackScheduler, SchedulerPoll};

/// Test packets are 20 ms each, so sequence N is due at `HOST_START_MS` + 20 * N.
const PACKET_MS: u64 = 20;

fn playing_scheduler() -> PlaybackScheduler {
    let mut scheduler = super::test_support::buffered_scheduler(30, 1);
    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 0),
        other => panic!("expected the first frame, got {other:?}"),
    }
    scheduler
}

#[test]
fn pausing_holds_the_buffered_packets_and_releases_nothing() {
    let mut scheduler = playing_scheduler();

    // Sequence 0 has played; sequences 1..30 are still buffered and held.
    assert_eq!(scheduler.pause(), 29);

    // Even well past their deadlines, nothing is released while paused.
    for offset_ms in [PACKET_MS, 500, 5_000] {
        assert!(matches!(
            scheduler.poll(HOST_START_MS + offset_ms),
            SchedulerPoll::Waiting { .. }
        ));
    }
    assert!(scheduler.is_paused());
}

#[test]
fn resuming_continues_with_the_first_unplayed_packet_on_the_shifted_timeline() {
    let mut scheduler = playing_scheduler();
    scheduler.pause();

    // The host resumes 1000 ms later and re-anchors its timeline by that pause.
    let shifted_start = HOST_START_MS + 1_000;
    scheduler.set_host_start_time_ms(shifted_start);
    scheduler.resume();
    assert!(!scheduler.is_paused());

    match scheduler.poll(shifted_start + PACKET_MS) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 1),
        other => panic!("expected sequence 1 on resume, got {other:?}"),
    }
}

#[test]
fn a_resume_without_a_pause_changes_nothing() {
    let mut scheduler = playing_scheduler();
    scheduler.resume();
    assert!(!scheduler.is_paused());
    match scheduler.poll(HOST_START_MS + PACKET_MS) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 1),
        other => panic!("expected sequence 1, got {other:?}"),
    }
}
