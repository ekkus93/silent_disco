//! A listener's own pause silences output at once, and the resume rebuilds
//! only the short resume span instead of the full startup or rebuffer target.

use super::test_support::{HOST_START_MS, datagram};
use super::{PlaybackScheduler, SchedulerPoll};

/// Packets carry 20 ms each in these tests, so a 100 ms span is six packets.
const RESUME_SPAN_PACKETS: u64 = 6;

fn playing_scheduler() -> PlaybackScheduler {
    let mut scheduler = super::test_support::buffered_scheduler(30, 1);
    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 0),
        other => panic!("expected the first frame, got {other:?}"),
    }
    scheduler
}

#[test]
fn pausing_discards_everything_buffered_and_returns_to_buffering() {
    let mut scheduler = playing_scheduler();

    // Sequence 0 has played; sequences 1..30 were still buffered.
    assert_eq!(scheduler.pause(), 29);

    assert!(matches!(
        scheduler.poll(HOST_START_MS + 60),
        SchedulerPoll::Buffering { .. }
    ));
}

#[test]
fn resuming_after_a_pause_needs_only_the_short_resume_span() {
    let mut scheduler = playing_scheduler();
    scheduler.pause();

    // The host continues numbering after the pause, so the resume starts at
    // sequence 30. Six packets span exactly the 100 ms resume target.
    for sequence in 30..30 + RESUME_SPAN_PACKETS {
        scheduler
            .submit_packet(datagram(sequence, 2))
            .expect("accepted");
    }

    match scheduler.poll(HOST_START_MS + 30 * 20) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, 30),
        other => panic!("expected the resumed frame, got {other:?}"),
    }
}

#[test]
fn a_span_shorter_than_the_resume_target_keeps_the_stream_buffering() {
    let mut scheduler = playing_scheduler();
    scheduler.pause();

    // Five packets span 80 ms, below the 100 ms resume target.
    for sequence in 30..30 + RESUME_SPAN_PACKETS - 1 {
        scheduler
            .submit_packet(datagram(sequence, 2))
            .expect("accepted");
    }

    assert!(matches!(
        scheduler.poll(HOST_START_MS + 30 * 20),
        SchedulerPoll::Buffering { .. }
    ));
}
