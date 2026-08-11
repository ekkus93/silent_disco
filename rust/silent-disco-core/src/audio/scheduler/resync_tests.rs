//! Offset soft/hard correction, `submit_packet` rejection, stale-packet
//! resync-onto-live-stream, out-of-order/outage/bootstrap integration tests,
//! and the ignored cross-listener alignment acceptance test.

use crate::audio::JitterBufferRejectionKind;
use crate::domain::SessionId;

use super::test_support::{HOST_START_MS, PACKET_DURATION_MS, config, datagram, frame_at};
use super::{
    DEFAULT_HARD_RESYNC_THRESHOLD_MS, OffsetUpdateOutcome, PlaybackScheduler, SchedulerPoll,
};

#[test]
fn applies_a_small_offset_change_as_a_soft_correction() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    let outcome = scheduler.apply_offset_update(10.0);
    assert_eq!(outcome, OffsetUpdateOutcome::SoftCorrected);
    assert!(!scheduler.is_awaiting_rebuffer());
}

#[test]
fn applies_a_large_offset_change_as_a_hard_resync() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    let outcome = scheduler.apply_offset_update(DEFAULT_HARD_RESYNC_THRESHOLD_MS + 1.0);
    assert_eq!(outcome, OffsetUpdateOutcome::HardResyncRequired);
    assert!(scheduler.is_awaiting_rebuffer());
    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::AwaitingRebuffer
    ));
}

#[test]
fn submit_packet_rejects_a_packet_from_the_wrong_session() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    let mut wrong = datagram(0, 1);
    wrong.session_id = SessionId::new("session-other").expect("session id");

    let error = scheduler
        .submit_packet(wrong)
        .expect_err("wrong session must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::WrongSession);
}

#[test]
fn submit_packet_rejects_a_duplicate_sequence() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    scheduler.submit_packet(datagram(0, 1)).expect("accepted");

    let error = scheduler
        .submit_packet(datagram(0, 1))
        .expect_err("duplicate must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::Duplicate);
}

#[test]
fn rejects_a_hostile_flood_of_far_future_sequences() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");

    let error = scheduler
        .submit_packet(datagram(1_000_000, 1))
        .expect_err("far-future sequence must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::ReorderWindowExceeded);
}

/// A handful of stale packets in the buffer must not be able to wedge the
/// stream forever.
///
/// Measured on a device (run 20): sync locked late enough that only 15
/// packets landed inside the reorder window before the live stream ran past
/// it. Those 15 were far below the scheduler's startup target, so it stayed
/// in `Buffering` and never popped them — and because the resync required an
/// *empty* buffer, their presence blocked the resync that would have
/// recovered. All 7,728 later arrivals were rejected and playback never
/// began; the listener heard one 75ms fragment when the tail drained at stop.
#[test]
fn stale_buffered_packets_do_not_block_resynchronisation_onto_a_live_stream() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 1_000;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    // A few packets land inside the window and stick: far short of the
    // startup target, so nothing ever plays them.
    for sequence in 0..15 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }
    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::Buffering { .. }
    ));

    // The live stream is now far beyond the reorder window. Enough corroborating
    // arrivals must move the scheduler onto it despite the stale packets.
    let live_start = 5_000;
    let mut accepted_live = 0;
    for offset in 0..8 {
        if scheduler
            .submit_packet(datagram(live_start + offset, 8_000))
            .is_ok()
        {
            accepted_live += 1;
        }
    }

    assert!(
        accepted_live > 0,
        "the stream stayed wedged: every live packet was rejected while stale ones were held"
    );
    assert_eq!(scheduler.jitter_statistics().resynchronisations, 1);
}

#[test]
fn a_packet_arriving_after_its_slot_played_is_rejected_and_never_plays_out_of_order() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    for sequence in [0, 2] {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    let first = frame_at(scheduler.poll(HOST_START_MS));
    // Sequence 1's slot is concealed and passes.
    let concealed = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));
    // Sequence 1 finally shows up, far too late to play in order.
    let rejection = scheduler
        .submit_packet(datagram(1, 8_000))
        .expect_err("an already-emitted sequence must be rejected");
    let next = frame_at(scheduler.poll(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS)));

    assert_eq!(first.sequence, 0);
    assert!(concealed.concealed);
    assert_eq!(rejection.kind, JitterBufferRejectionKind::AlreadyEmitted);
    // Playback continues forward; the late arrival never appears.
    assert_eq!(next.sequence, 2);
    assert!(!next.concealed);
}

#[test]
fn an_outage_wider_than_the_reorder_window_does_not_permanently_wedge_the_stream() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 100;
    cfg.max_consecutive_concealed_packets = 3;
    cfg.max_reorder_window = 8;
    cfg.concealment_skip_threshold_packets = 4;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    // Only as many as the reorder window admits at once.
    for sequence in 0..9 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }
    for slot in 0..5 {
        let _ = scheduler.poll(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS));
    }
    // The network drops entirely; concealment exhausts its bound and the
    // caller rebuffers, exactly as the pump does automatically.
    for slot in 20..40 {
        if matches!(
            scheduler.poll(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS)),
            SchedulerPoll::AwaitingRebuffer
        ) {
            scheduler.rebuffer(0.0);
        }
    }

    // The host has moved far ahead. If every packet is rejected as
    // unreorderable, the stream is silent until the runtime is torn down.
    let mut accepted = 0;
    for sequence in 400..420 {
        if scheduler.submit_packet(datagram(sequence, 8_000)).is_ok() {
            accepted += 1;
        }
    }
    assert!(
        accepted > 0,
        "a listener that fell behind can never resynchronise: all 20 packets rejected"
    );
}

#[test]
fn a_listener_joining_a_stream_already_in_progress_can_bootstrap() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 100;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    // The host has been streaming for ten seconds. The listener starts at
    // sequence zero, so the first arrivals are beyond its reorder window and
    // are rejected until they corroborate each other; after that it must
    // adopt the live position rather than demanding the packets it missed.
    let mut accepted = 0;
    for sequence in 500..520 {
        if scheduler.submit_packet(datagram(sequence, 8_000)).is_ok() {
            accepted += 1;
        }
    }
    assert!(
        accepted > 0,
        "a listener joining mid-stream could never accept a packet"
    );

    // The corroborating packets are themselves rejected, so their slots
    // conceal; real audio resumes immediately after them.
    let mut real_frame = None;
    for _ in 0..10 {
        let frame = frame_at(scheduler.poll(HOST_START_MS + 520 * u64::from(PACKET_DURATION_MS)));
        assert!(
            frame.sequence >= 500,
            "playback must resume at the live position, got {}",
            frame.sequence
        );
        if !frame.concealed {
            real_frame = Some(frame);
            break;
        }
    }
    assert!(
        real_frame.is_some(),
        "a listener joining mid-stream never reached real audio"
    );
}

/// Local time at which a scheduler's frame is actually heard, given that a
/// stream is heard at `write time + ring depth` and the ring drains at a
/// fixed rate. With no ring in this test the depth is the write-lead the
/// pump maintains, which is constant across listeners.
fn playout_time_ms(write_time_ms: u64, lead_ms: u64) -> u64 {
    write_time_ms + lead_ms
}

// Acceptance test for item 4. Ignored: the first implementation regressed a
// real device (see the fixes TODO) and was reverted, so this documents the
// target rather than current behaviour.
#[test]
#[ignore = "alignment is unimplemented: the first attempt regressed on device and was reverted"]
fn two_listeners_locking_sync_at_different_moments_play_the_same_audio_together() {
    // Same host stream, same offset; the only difference is when each
    // listener finished buffering and began playing. That difference used to
    // shift each listener's whole stream by its own startup latency.
    const LEAD_MS: u64 = 400;
    let mut early = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    let mut late = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    // Stay inside the default reorder window.
    for sequence in 0..60 {
        early
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
        late.submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    // One listener starts promptly; the other is 300ms slower to lock sync.
    let early_start = HOST_START_MS + 100;
    let late_start = HOST_START_MS + 400;
    let early_frame = frame_at(early.poll(early_start));
    let late_frame = frame_at(late.poll(late_start));

    // Each plays a frame that is genuinely due, so their playout times for
    // the same sequence agree rather than differing by their startup skew.
    assert_eq!(
        early_frame.sequence, 5,
        "the early listener must start on the frame due at its start time"
    );
    assert_eq!(
        late_frame.sequence, 20,
        "the late listener must skip the head it missed, not replay it"
    );

    // Advance the early listener to the same sequence the late one started
    // on, and compare when each would be heard.
    let mut early_playout = None;
    for step in 1..40 {
        let frame = frame_at(early.poll(early_start + step * u64::from(PACKET_DURATION_MS)));
        if frame.sequence == late_frame.sequence {
            early_playout = Some(playout_time_ms(
                early_start + step * u64::from(PACKET_DURATION_MS),
                LEAD_MS,
            ));
            break;
        }
    }
    let early_playout =
        early_playout.expect("the early listener never reached the shared sequence");
    let late_playout = playout_time_ms(late_start, LEAD_MS);

    let skew_ms = early_playout.abs_diff(late_playout);
    assert!(
        skew_ms <= u64::from(PACKET_DURATION_MS),
        "listeners drifted {skew_ms}ms apart on the same sequence; \
         they must agree within one packet"
    );
}
