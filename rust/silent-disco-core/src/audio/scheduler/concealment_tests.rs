//! Concealment bound, gap-skip vs. packet-by-packet concealment, fade-in/
//! blend/waveform continuity, and `drain_remaining`.

use super::test_support::{
    CHANNELS, HOST_START_MS, PACKET_DURATION_MS, RAMP_FRAMES, SAMPLES_PER_PACKET,
    buffered_scheduler, config, datagram, frame_at,
};
use super::{PlaybackScheduler, SchedulerPoll};

#[test]
fn conceals_a_missing_packet_once_its_presentation_deadline_arrives_and_progresses_monotonically() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    for sequence in 1..=21 {
        scheduler
            .submit_packet(datagram(sequence, 1))
            .expect("accepted");
    }

    match scheduler.poll(HOST_START_MS) {
        SchedulerPoll::Frame { frame, .. } => {
            assert_eq!(frame.sequence, 0);
            assert_eq!(frame.first_sample_index, 0);
            assert_eq!(frame.host_presentation_time_ms, HOST_START_MS);
            assert!(frame.concealed);
            assert!(frame.samples.iter().all(|&sample| sample == 0));
            assert_eq!(
                frame.samples.len(),
                usize::try_from(SAMPLES_PER_PACKET).expect("fits usize") * usize::from(CHANNELS)
            );
        }
        other => panic!("expected a concealed Frame, got {other:?}"),
    }

    match scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)) {
        SchedulerPoll::Frame { frame, .. } => {
            assert_eq!(frame.sequence, 1);
            assert!(!frame.concealed);
        }
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn signals_awaiting_rebuffer_after_the_consecutive_concealment_bound_is_reached() {
    let mut cfg = config();
    cfg.max_consecutive_concealed_packets = 2;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    let first = scheduler.poll(HOST_START_MS);
    assert!(matches!(first, SchedulerPoll::Frame { ref frame, .. } if frame.concealed));

    let second = scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS));
    assert!(matches!(second, SchedulerPoll::AwaitingRebuffer));
    assert!(scheduler.is_awaiting_rebuffer());

    let third = scheduler.poll(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS));
    assert!(matches!(third, SchedulerPoll::AwaitingRebuffer));
}

#[test]
fn rebuffer_resumes_playback_and_preserves_already_buffered_packets() {
    let mut cfg = config();
    cfg.max_consecutive_concealed_packets = 1;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");

    assert!(matches!(
        scheduler.poll(HOST_START_MS),
        SchedulerPoll::AwaitingRebuffer
    ));

    scheduler.rebuffer(0.0);
    assert!(!scheduler.is_awaiting_rebuffer());

    scheduler.submit_packet(datagram(1, 1)).expect("accepted");
    match scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)) {
        SchedulerPoll::Frame { frame, .. } => {
            assert_eq!(frame.sequence, 1);
            assert!(!frame.concealed);
        }
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn the_first_frame_of_a_stream_fades_in_from_silence() {
    let mut scheduler = buffered_scheduler(30, 8_000);

    let first = frame_at(scheduler.poll(HOST_START_MS));
    let second = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));

    // A stream opens at whatever sample its presentation timeline lands on,
    // so the very first frame ramps up instead of stepping to full scale.
    assert!(!first.concealed);
    assert_eq!(first.samples[0], 0);
    assert_eq!(first.samples[RAMP_FRAMES / 2 * 2], 4_000);
    assert_eq!(first.samples[RAMP_FRAMES * 2], 8_000);
    // Steady-state frames pass through untouched.
    assert_eq!(second.samples[0], 8_000);
}

#[test]
fn real_audio_resuming_after_concealment_blends_from_the_concealed_tail() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    // Sequence 1 never arrives, so its slot is concealed and sequence 2
    // resumes real audio afterwards.
    for sequence in [
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ] {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    let _first = frame_at(scheduler.poll(HOST_START_MS));
    let concealed = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));
    let resumed = frame_at(scheduler.poll(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS)));

    assert!(concealed.concealed);
    assert!(!resumed.concealed);
    assert_eq!(resumed.sequence, 2);
    // The concealed frame ended mid-decay, so the resume seam continues from
    // that value and ramps back to full amplitude rather than stepping from a
    // silence the concealment never reached.
    let concealed_tail = concealed.samples[concealed.samples.len() - 2];
    assert_ne!(concealed_tail, 0);
    assert_eq!(resumed.samples[0], concealed_tail);
    assert_eq!(resumed.samples[RAMP_FRAMES * 2], 8_000);
}

#[test]
fn a_rebuffered_stream_fades_its_next_real_frame_in() {
    let mut scheduler = buffered_scheduler(30, 8_000);
    let _first = frame_at(scheduler.poll(HOST_START_MS));

    scheduler.rebuffer(0.0);
    // Deliver enough span to leave Buffering again, then take the next frame.
    let resumed = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));

    assert!(!resumed.concealed);
    assert_eq!(resumed.samples[0], 0);
    assert_eq!(resumed.samples[RAMP_FRAMES * 2], 8_000);
}

#[test]
fn a_gap_wider_than_the_skip_threshold_is_abandoned_rather_than_concealed() {
    let mut cfg = config();
    cfg.concealment_skip_threshold_packets = 3;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    scheduler
        .submit_packet(datagram(0, 8_000))
        .expect("accepted");
    // Sequences 1..=9 never arrive: a nine-packet hole, well past the
    // three-packet threshold.
    for sequence in 10..=20 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    let first = frame_at(scheduler.poll(HOST_START_MS));
    // Polled well past the whole hole, the scheduler must not emit a single
    // concealed frame for it.
    let after_gap = frame_at(scheduler.poll(HOST_START_MS + 10 * u64::from(PACKET_DURATION_MS)));

    assert_eq!(first.sequence, 0);
    assert!(!after_gap.concealed);
    assert_eq!(after_gap.sequence, 10);
    // The post-gap frame keeps its own presentation time, so playback does
    // not trail the timeline by the length of the outage.
    assert_eq!(
        after_gap.host_presentation_time_ms,
        HOST_START_MS + 10 * u64::from(PACKET_DURATION_MS)
    );
    // Nothing is emitted for the abandoned span, so the post-gap frame plays
    // directly after the pre-gap one. The seam is a crossfade from the
    // outgoing waveform: fading in from a zero that never renders would be
    // the step it was meant to prevent.
    let outgoing_tail = first.samples[first.samples.len() - 2];
    assert_eq!(outgoing_tail, 8_000);
    assert_eq!(after_gap.samples[0], outgoing_tail);
    assert_eq!(after_gap.samples[RAMP_FRAMES * 2], 8_000);
}

/// A skip that follows a concealment run must still splice continuously: the
/// concealed frame ends mid-decay, and the post-gap frame has to continue from
/// that value rather than from zero.
#[test]
fn a_skip_after_concealment_continues_from_the_concealed_tail() {
    let mut cfg = config();
    cfg.concealment_skip_threshold_packets = 3;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    scheduler
        .submit_packet(datagram(0, 8_000))
        .expect("accepted");

    let _first = frame_at(scheduler.poll(HOST_START_MS));
    // Sequence 1's slot comes due with nothing buffered behind it, so it is
    // concealed rather than skipped.
    let concealed = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));
    // The stream then resumes far ahead: a hole wide enough to abandon, with
    // the concealed frame as the last thing actually emitted.
    for sequence in 12..=20 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }
    let after_gap = frame_at(scheduler.poll(HOST_START_MS + 12 * u64::from(PACKET_DURATION_MS)));

    assert!(concealed.concealed);
    assert!(!after_gap.concealed);
    assert_eq!(after_gap.sequence, 12);
    let concealed_tail = concealed.samples[concealed.samples.len() - 2];
    assert_ne!(concealed_tail, 0);
    assert_eq!(after_gap.samples[0], concealed_tail);
}

#[test]
fn a_gap_within_the_skip_threshold_is_still_concealed_packet_by_packet() {
    let mut cfg = config();
    cfg.concealment_skip_threshold_packets = 3;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    scheduler
        .submit_packet(datagram(0, 8_000))
        .expect("accepted");
    // A two-packet hole (sequences 1 and 2), inside the threshold.
    for sequence in 3..=20 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    let _first = frame_at(scheduler.poll(HOST_START_MS));
    let concealed = frame_at(scheduler.poll(HOST_START_MS + u64::from(PACKET_DURATION_MS)));

    assert!(concealed.concealed);
    assert_eq!(concealed.sequence, 1);
    // Repetition, not silence.
    // First consecutive loss repeats at full amplitude; halving starts on the second.
    assert_eq!(concealed.samples[480 * 2], 8_000);
}

#[test]
fn a_skipped_gap_does_not_count_against_the_consecutive_concealment_bound() {
    let mut cfg = config();
    cfg.concealment_skip_threshold_packets = 3;
    cfg.max_consecutive_concealed_packets = 2;
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    scheduler
        .submit_packet(datagram(0, 8_000))
        .expect("accepted");
    for sequence in 10..=20 {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }

    let _first = frame_at(scheduler.poll(HOST_START_MS));
    let after_gap = frame_at(scheduler.poll(HOST_START_MS + 10 * u64::from(PACKET_DURATION_MS)));

    // Skipping is a deliberate decision, not a failure to keep up: it must
    // not push the scheduler toward AwaitingRebuffer.
    assert!(!scheduler.is_awaiting_rebuffer());
    assert_eq!(after_gap.sequence, 10);
}

#[test]
fn drain_returns_buffered_tail_content_in_sequence_order_with_a_faded_final_frame() {
    let mut scheduler = buffered_scheduler(30, 8_000);
    let first = frame_at(scheduler.poll(HOST_START_MS));
    assert_eq!(first.sequence, 0);

    let drained = scheduler.drain_remaining();

    // Everything still buffered arrived in time: it is real tail content, not
    // backlog, and must not be discarded when the stream stops.
    assert_eq!(drained.len(), 29);
    assert_eq!(drained.first().expect("frames").sequence, 1);
    assert_eq!(drained.last().expect("frames").sequence, 29);
    assert!(drained.iter().all(|frame| !frame.concealed));
    // Contiguous interior seams stay untouched.
    let interior = &drained[5];
    assert_eq!(interior.samples[0], 8_000);
    assert_eq!(interior.samples[959 * 2], 8_000);
    // The stream must end at zero rather than cutting mid-waveform.
    assert_eq!(drained.last().expect("frames").samples[959 * 2], 0);
}

#[test]
fn drain_fades_both_edges_of_a_hole_inside_the_drained_range() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    for sequence in [0, 1, 3, 4] {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }
    let _first = frame_at(scheduler.poll(HOST_START_MS));

    let drained = scheduler.drain_remaining();

    assert_eq!(
        drained
            .iter()
            .map(|frame| frame.sequence)
            .collect::<Vec<_>>(),
        vec![1, 3, 4]
    );
    // Sequence 1 precedes the hole: its tail fades out.
    assert_eq!(drained[0].samples[959 * 2], 0);
    // Sequence 3 follows the hole: its head fades in.
    assert_eq!(drained[1].samples[0], 0);
    assert_eq!(drained[1].samples[RAMP_FRAMES * 2], 8_000);
    // Sequence 4 continues 3 with no hole, so its head is untouched.
    assert_eq!(drained[2].samples[0], 8_000);
}

#[test]
fn drain_fades_in_when_its_first_frame_does_not_continue_the_delivered_stream() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    for sequence in [0, 5, 6] {
        scheduler
            .submit_packet(datagram(sequence, 8_000))
            .expect("accepted");
    }
    let first = frame_at(scheduler.poll(HOST_START_MS));
    assert_eq!(first.sequence, 0);

    let drained = scheduler.drain_remaining();

    // The drain starts at 5 while 0 was the last delivered sequence: a hole
    // against the live stream, so the resumed content fades in.
    assert_eq!(drained[0].sequence, 5);
    assert_eq!(drained[0].samples[0], 0);
    assert_eq!(drained[0].samples[RAMP_FRAMES * 2], 8_000);
}

#[test]
fn draining_an_empty_buffer_yields_nothing() {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    assert!(scheduler.drain_remaining().is_empty());
}

#[test]
fn an_arrival_outage_is_bridged_to_the_configured_bound_then_awaits_an_explicit_rebuffer() {
    let mut cfg = config();
    cfg.startup_buffer_target_ms = 0;
    cfg.max_consecutive_concealed_packets = 4;
    let mut scheduler = PlaybackScheduler::new(cfg, 0.0).expect("valid scheduler");
    scheduler
        .submit_packet(datagram(0, 8_000))
        .expect("accepted");

    let first = frame_at(scheduler.poll(HOST_START_MS));
    assert_eq!(first.sequence, 0);

    // Nothing further ever arrives. The bridge covers the outage for exactly
    // the configured bound, decaying as it goes, then stops rather than
    // synthesizing forever.
    let bridged: Vec<_> = (1..=3)
        .map(|slot| frame_at(scheduler.poll(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS))))
        .collect();
    let at_bound = scheduler.poll(HOST_START_MS + 4 * u64::from(PACKET_DURATION_MS));

    assert!(bridged.iter().all(|frame| frame.concealed));
    assert_eq!(bridged[0].samples[480 * 2], 8_000);
    assert_eq!(bridged[1].samples[480 * 2], 4_000);
    assert_eq!(bridged[2].samples[480 * 2], 2_000);
    assert!(matches!(at_bound, SchedulerPoll::AwaitingRebuffer));
    assert!(scheduler.is_awaiting_rebuffer());

    // Recovery is explicit: the scheduler stays paused until told to
    // rebuffer, rather than silently resuming mid-outage.
    assert!(matches!(
        scheduler.poll(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS)),
        SchedulerPoll::AwaitingRebuffer
    ));
    scheduler.rebuffer(0.0);
    assert!(!scheduler.is_awaiting_rebuffer());
}
