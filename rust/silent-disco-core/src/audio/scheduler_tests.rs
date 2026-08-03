use super::{
    BufferHealth, DEFAULT_CONCEALMENT_RAMP_MS, DEFAULT_HARD_RESYNC_THRESHOLD_MS,
    JitterBufferRejectionKind, OffsetUpdateOutcome, PlaybackScheduler, ScheduledFrame,
    SchedulerConfig, SchedulerConfigErrorKind, SchedulerPoll,
};
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram};

const PACKET_DURATION_MS: u32 = 20;
const SAMPLES_PER_PACKET: u32 = 960;
const CHANNELS: u16 = 2;
const HOST_START_MS: u64 = 1_000;

fn session() -> SessionId {
    SessionId::new("session-scheduler").expect("session id")
}

fn stream() -> StreamId {
    StreamId::new("stream-scheduler").expect("stream id")
}

fn config() -> SchedulerConfig {
    SchedulerConfig::new(
        session(),
        stream(),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        CHANNELS,
    )
}

fn payload_for(sample_value: i16) -> Vec<u8> {
    (0..usize::try_from(SAMPLES_PER_PACKET).expect("fits usize") * usize::from(CHANNELS))
        .flat_map(|_| sample_value.to_le_bytes())
        .collect()
}

fn datagram(sequence: u64, sample_value: i16) -> AudioDatagram {
    let host_time = HOST_START_MS + sequence * u64::from(PACKET_DURATION_MS);
    AudioDatagram {
        session_id: session(),
        stream_id: stream(),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: CHANNELS,
        samples_per_packet: SAMPLES_PER_PACKET,
        first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
        host_presentation_time_ms: MonotonicMillis::new(host_time),
        payload: payload_for(sample_value),
    }
}

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
            assert_eq!(frame.first_sample_index, u64::from(SAMPLES_PER_PACKET));
            assert_eq!(
                frame.host_presentation_time_ms,
                HOST_START_MS + u64::from(PACKET_DURATION_MS)
            );
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
fn rejects_an_invalid_packet_duration() {
    let mut cfg = config();
    cfg.packet_duration_ms = 0;

    let error = PlaybackScheduler::new(cfg, 0.0).expect_err("invalid duration must be rejected");
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

/// Drives a scheduler past its startup buffer with `count` packets of
/// `sample_value`, returning it ready to deliver sequence zero.
fn buffered_scheduler(count: u64, sample_value: i16) -> PlaybackScheduler {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    for sequence in 0..count {
        scheduler
            .submit_packet(datagram(sequence, sample_value))
            .expect("accepted");
    }
    scheduler
}

fn frame_at(poll: SchedulerPoll) -> ScheduledFrame {
    match poll {
        SchedulerPoll::Frame { frame, .. } => frame,
        other => panic!("expected a frame, got {other:?}"),
    }
}

/// Ramp length for the default config: 5ms of a 20ms/960-sample packet.
/// Frames the scheduler ramps over, derived exactly as it derives them so
/// this stays correct if either the ramp or the packet duration moves.
const RAMP_FRAMES: usize =
    (SAMPLES_PER_PACKET * DEFAULT_CONCEALMENT_RAMP_MS / PACKET_DURATION_MS) as usize;

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
    assert_eq!(concealed.samples[480 * 2], 4_000);
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
    assert_eq!(bridged[0].samples[480 * 2], 4_000);
    assert_eq!(bridged[1].samples[480 * 2], 2_000);
    assert_eq!(bridged[2].samples[480 * 2], 1_000);
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
