//! The diagnostics snapshot: `hard_resync_signals` summing both rebuffer
//! causes, packet-level accounting, and phase reporting through a stream's
//! life.

use crate::audio::{
    PlaybackPhase, PlaybackScheduler, RenderRing, RenderRingConfig, SchedulerConfig,
};
use crate::domain::{SessionId, StreamId};

use super::super::{PlaybackPump, PumpTick, SyncApplyOutcome};
use super::{HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, datagram, unpaced_config};

#[test]
fn hard_resync_signals_sums_concealment_and_offset_driven_causes() {
    // A4.4: the two rebuffer causes must both land in the same total,
    // not just whichever one a given code path happened to count first.
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    scheduler_config.max_consecutive_concealed_packets = 2;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, _consumer) = ring.split();
    let mut pump =
        PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
    pump.apply_sync_offset(0.0);

    // Force a concealment-driven rebuffer exactly as
    // `a_paused_scheduler_is_re_armed_so_playback_recovers_after_an_outage`
    // does below: one packet arrives, then nothing more, so the
    // consecutive-concealment bound (2) trips.
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
    assert!(matches!(
        pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS)),
        PumpTick::Queued {
            concealed: true,
            ..
        }
    ));
    assert!(matches!(
        pump.tick(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS)),
        PumpTick::AwaitingRebuffer
    ));
    let after_concealment = pump.diagnostics();
    assert_eq!(
        after_concealment.concealment_driven_rebuffers, 1,
        "expected the consecutive-concealment bound to have tripped exactly once"
    );
    assert_eq!(after_concealment.offset_driven_rebuffers, 0);
    assert_eq!(
        after_concealment.hard_resync_signals,
        after_concealment.concealment_driven_rebuffers
    );

    // Now force an offset-driven rebuffer too. Both causes must be
    // reflected in the same total.
    assert_eq!(
        pump.apply_sync_offset(5_000.0),
        SyncApplyOutcome::Rebuffered
    );
    let after_both = pump.diagnostics();
    assert_eq!(after_both.offset_driven_rebuffers, 1);
    assert_eq!(
        after_both.concealment_driven_rebuffers, after_concealment.concealment_driven_rebuffers,
        "the offset-driven jump must not also count as a concealment event"
    );
    assert_eq!(
        after_both.hard_resync_signals,
        after_both.concealment_driven_rebuffers + 1,
        "hard_resync_signals must be the sum of both causes, not just one"
    );
}

#[test]
fn diagnostics_account_for_delivered_concealed_and_skipped_audio() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    scheduler_config.concealment_skip_threshold_packets = 3;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, consumer) = ring.split();
    let mut pump =
        PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
    pump.apply_sync_offset(0.0);

    // Sequence 0 arrives, 1 is lost (concealed), 2 arrives, then a wide
    // gap to 20 is skipped outright.
    for sequence in [0, 2, 20, 21] {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    // A duplicate and a stale arrival, both ordinary traffic to account for.
    assert!(pump.scheduler_mut().submit_packet(datagram(20, 1)).is_err());
    // Stop at the last real packet: ticking past it would start the
    // outage bridge, whose concealments are a separate behaviour.
    for slot in 0..=21 {
        pump.tick(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS));
    }
    assert!(pump.scheduler_mut().submit_packet(datagram(1, 1)).is_err());

    let diagnostics = pump.diagnostics();

    assert_eq!(diagnostics.packets_accepted, 4);
    assert_eq!(diagnostics.duplicate_rejections, 1);
    assert_eq!(diagnostics.late_rejections, 1);
    // One concealed slot (sequence 1) and one skipped gap (3..=19).
    assert_eq!(diagnostics.concealed_packets, 1);
    assert!(
        diagnostics.sequences_skipped >= 17,
        "expected the wide gap to be accounted for, got {}",
        diagnostics.sequences_skipped
    );
    assert!(diagnostics.sync_locked);
    assert_eq!(diagnostics.phase, PlaybackPhase::Playing);

    // Ring-side counters come from the ring's own telemetry.
    assert!(diagnostics.ring_queued_frames > 0);
    assert_eq!(
        diagnostics.ring_peak_queued_frames,
        diagnostics.ring_queued_frames
    );
    let mut output = vec![0.0_f32; 60_000 * 2];
    let _ = consumer.read_frames(&mut output);
    let after_underrun = pump.diagnostics();
    assert!(after_underrun.ring_underruns > 0);
    assert!(after_underrun.ring_silence_filled_frames > 0);
}

#[test]
fn diagnostics_report_the_phase_through_a_streams_life() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 400;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, _consumer) = ring.split();
    let mut pump =
        PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");

    assert!(!pump.diagnostics().sync_locked);
    pump.apply_sync_offset(0.0);
    assert_eq!(pump.diagnostics().phase, PlaybackPhase::Buffering);

    for sequence in 0..30 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    pump.tick(HOST_START_MS);
    assert_eq!(pump.diagnostics().phase, PlaybackPhase::Playing);
    assert!(pump.diagnostics().buffered_span_ms > 0);

    pump.finish();
    assert_eq!(pump.diagnostics().phase, PlaybackPhase::Stopped);
}
