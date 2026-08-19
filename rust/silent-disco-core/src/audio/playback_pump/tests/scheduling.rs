//! The tick-driven pacing loop: partial-write retry, buffering/waiting/
//! stopped reporting, the startup alignment prefill, the write lead, the
//! target-depth cap, rebuffer re-arm, and steady-state cushion convergence.

use crate::audio::{PlaybackScheduler, RenderRing, RenderRingConfig, SchedulerConfig};
use crate::domain::{SessionId, StreamId};

use super::super::{PlaybackPump, PlaybackPumpConfig, PumpTick};
use super::{
    HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, datagram, paced_pump, paced_pump_with,
    pump_with, unpaced_config,
};

#[test]
fn a_frame_the_ring_cannot_hold_is_retried_rather_than_partly_discarded() {
    // The smallest permitted ring holds exactly five 960-frame packets.
    let (mut pump, consumer) = pump_with(4_800);
    for sequence in 0..7 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    for slot in 0..5 {
        let tick = pump.tick(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS));
        assert!(
            matches!(tick, PumpTick::Queued { .. }),
            "slot {slot}: {tick:?}"
        );
    }

    // Free less than one packet's worth, so the next frame can only be
    // written in part.
    let mut small = vec![0.0_f32; 300 * 2];
    let _ = consumer.read_frames(&mut small);
    let blocked = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));

    assert!(
        matches!(
            blocked,
            PumpTick::RingFull {
                pending_frames: 660
            }
        ),
        "expected a held remainder, got {blocked:?}"
    );
    assert_eq!(pump.pending_frames(), 660);

    // Draining the ring lets the held remainder through intact: a partial
    // write must never cost audio.
    let mut all = vec![0.0_f32; 4_800 * 2];
    let _ = consumer.read_frames(&mut all);
    let flushed = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));

    assert!(matches!(flushed, PumpTick::FlushedPending { frames: 660 }));
    assert_eq!(pump.pending_frames(), 0);
}

#[test]
fn reports_buffering_waiting_and_stopped_states() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 400;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 1,
    })
    .expect("valid ring");
    let (producer, _consumer) = ring.split();
    let mut pump = PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("pump");
    pump.apply_sync_offset(0.0);

    assert!(matches!(
        pump.tick(HOST_START_MS),
        PumpTick::Buffering { buffered_ms: 0 }
    ));

    for sequence in 0..30 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 1_000))
            .expect("accepted");
    }
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
    // Polled before the next slot is due.
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Waiting));

    pump.finish();
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Stopped));
}

#[test]
fn finish_queues_the_buffered_tail_and_stops_the_scheduler() {
    let (mut pump, consumer) = pump_with(48_000);
    for sequence in 0..5 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    let _first = pump.tick(HOST_START_MS);

    let drained_frames = pump.finish();

    // Four packets were still buffered and must not be discarded.
    assert_eq!(drained_frames, 4 * 960);
    let mut output = vec![0.0_f32; 5 * 960 * 2];
    let outcome = consumer.read_frames(&mut output);
    assert_eq!(outcome.frames_supplied, 5 * 960);
    // The stream ends at zero rather than cutting mid-waveform.
    assert_eq!(output[5 * 960 * 2 - 1], 0.0);
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Stopped));
}

#[test]
fn the_startup_prefill_places_the_first_frame_at_its_presentation_deadline() {
    let (mut pump, consumer) = paced_pump();
    for sequence in 0..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // Sequence 0 is due at HOST_START_MS; tick a full lead ahead of that.
    let tick = pump.tick(HOST_START_MS - 400);

    assert!(matches!(tick, PumpTick::Queued { sequence: 0, .. }));
    // 400ms of silence at 48kHz precedes it, so it is heard exactly when
    // the ring has drained that much: at its deadline, not immediately.
    assert_eq!(pump.prefill_frames(), 19_200);
    let mut output = vec![0.0_f32; 19_200 * 2];
    let outcome = consumer.read_frames(&mut output);
    assert_eq!(outcome.frames_supplied, 19_200);
    assert!(output.iter().all(|&sample| sample == 0.0));
}

#[test]
fn startup_alignment_uses_true_now_not_the_future_write_horizon() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
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
        PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default()).expect("valid pump");
    pump.apply_sync_offset(0.0);

    for sequence in 0..=20 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    let tick = pump.tick(HOST_START_MS + 400);
    assert!(
        matches!(tick, PumpTick::Queued { sequence: 20, .. }),
        "the first reachable frame must survive the write-ahead horizon: {tick:?}"
    );
    let diagnostics = pump.diagnostics();
    assert_eq!(diagnostics.sequences_skipped, 20);
    assert_eq!(diagnostics.packets_emitted, 1);
    assert_eq!(diagnostics.hard_resync_signals, 0);
}

#[test]
fn alignment_does_not_reaccumulate_a_full_target_after_discarding_a_stale_buffer() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
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
        PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default()).expect("valid pump");
    pump.apply_sync_offset(0.0);

    for sequence in 0..=20 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    let emptied = pump.tick(HOST_START_MS + 420);
    assert!(matches!(emptied, PumpTick::Buffering { buffered_ms: 0 }));
    assert_eq!(pump.diagnostics().hard_resync_signals, 0);

    pump.scheduler_mut()
        .submit_packet(datagram(21, 16_384))
        .expect("accepted");
    let resumed = pump.tick(HOST_START_MS + 420);
    assert!(
        matches!(resumed, PumpTick::Queued { sequence: 21, .. }),
        "alignment should resume on the first reachable packet: {resumed:?}"
    );
    assert_eq!(pump.diagnostics().hard_resync_signals, 0);
}

#[test]
fn a_first_frame_that_is_already_due_is_not_delayed_by_a_prefill() {
    let (mut pump, _consumer) = paced_pump();
    for sequence in 0..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // Well past sequence 0's deadline. The scheduler drops the elapsed
    // head and starts on a frame that is genuinely due, so there is
    // nothing to align and no prefill is queued.
    let tick = pump.tick(HOST_START_MS + 5_000);

    assert!(
        matches!(tick, PumpTick::Queued { .. }),
        "expected a queued frame, got {tick:?}"
    );
    assert_eq!(pump.prefill_frames(), 0);
}

#[test]
fn the_prefill_is_clamped_so_a_distant_first_deadline_cannot_flood_the_ring() {
    // A lead wider than the ceiling is the only way the clamp can bind:
    // ordinarily a frame is released at most `write_lead_ms` early, so the
    // prefill never exceeds the lead.
    let (mut pump, _consumer) = paced_pump_with(PlaybackPumpConfig {
        write_lead_ms: 2_000,
        max_prefill_ms: 800,
        ..PlaybackPumpConfig::default()
    });
    for sequence in 0..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // With a lead longer than the prefill ceiling, a first frame a full
    // second out would otherwise queue a second of silence.
    let tick = pump.tick(0);

    assert!(matches!(tick, PumpTick::Queued { .. }));
    // 800ms at 48kHz, the configured ceiling, not the full 1000ms gap.
    assert_eq!(pump.prefill_frames(), 38_400);
}

#[test]
fn the_write_lead_releases_frames_before_their_deadline() {
    // Prefill off, so the lead is observable on its own: with it on, the
    // alignment silence immediately establishes the cushion and the depth
    // cap takes over (see the prefill tests above).
    let (mut pump, _consumer) = paced_pump_with(PlaybackPumpConfig {
        max_prefill_ms: 0,
        ..PlaybackPumpConfig::default()
    });
    for sequence in 0..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // Sequence 0 is due at HOST_START_MS. One millisecond earlier than a
    // full lead, it is not released yet.
    assert!(matches!(pump.tick(HOST_START_MS - 401), PumpTick::Waiting));
    // Exactly one lead ahead of the deadline, it is.
    assert!(matches!(
        pump.tick(HOST_START_MS - 400),
        PumpTick::Queued { sequence: 0, .. }
    ));
    // The next frame follows the same rule against its own deadline.
    assert!(matches!(
        pump.tick(HOST_START_MS - 380),
        PumpTick::Queued { sequence: 1, .. }
    ));
}

#[test]
fn the_depth_cap_stops_a_startup_backlog_from_pinning_the_ring_full() {
    let (mut pump, consumer) = paced_pump();
    // A second of audio arrives at once, as it does when a send-ahead
    // host floods a listener that has just locked sync.
    for sequence in 0..50 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // Drive far past every deadline: without the cap this would run the
    // ring to capacity and hold it there.
    let mut ticks = 0;
    loop {
        if let PumpTick::AtTargetDepth { queued_frames } = pump.tick(HOST_START_MS + 10_000) {
            assert!(queued_frames >= 19_200);
            break;
        }
        ticks += 1;
        assert!(ticks < 200, "the pump never reached its target depth");
    }

    // The cushion is the configured depth, not the ring's 48000-frame
    // capacity.
    assert!(pump.queued_frames() < 48_000);
    assert!(pump.queued_frames() >= 19_200);

    // Once the consumer drains below the cushion, writing resumes.
    let mut output = vec![0.0_f32; 5_000 * 2];
    let _ = consumer.read_frames(&mut output);
    assert!(matches!(
        pump.tick(HOST_START_MS + 10_000),
        PumpTick::Queued { .. }
    ));
}

#[test]
fn a_paused_scheduler_is_re_armed_so_playback_recovers_after_an_outage() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
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
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");

    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
    // Nothing more arrives; the concealment bound is reached.
    assert!(matches!(
        pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS)),
        PumpTick::Queued {
            concealed: true,
            ..
        }
    ));
    let paused = pump.tick(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS));
    assert!(matches!(paused, PumpTick::AwaitingRebuffer));

    // The pause forces a fresh startup buffer, but must not end playback:
    // when audio resumes, so does the pump.
    for sequence in 10..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    let resumed = pump.tick(HOST_START_MS + 20 * u64::from(PACKET_DURATION_MS));
    assert!(
        matches!(
            resumed,
            PumpTick::Queued { .. } | PumpTick::Buffering { .. }
        ),
        "playback must recover, got {resumed:?}"
    );
}

#[test]
fn steady_state_ring_depth_converges_to_the_configured_cushion() {
    let (mut pump, consumer) = paced_pump();

    // Simulate a real stream: the host keeps delivering, the pump ticks
    // every 10ms, and the consumer drains at the sample rate.
    let mut clock_ms = HOST_START_MS - 400;
    let mut next_sequence = 0_u64;
    let mut output = vec![0.0_f32; 480 * 2];
    let mut depths = Vec::new();

    for step in 0..400 {
        // Packets arrive a little ahead of their deadlines, as a
        // send-ahead host delivers them.
        while next_sequence * u64::from(PACKET_DURATION_MS) + HOST_START_MS < clock_ms + 600 {
            let _ = pump
                .scheduler_mut()
                .submit_packet(datagram(next_sequence, 16_384));
            next_sequence += 1;
        }
        pump.tick(clock_ms);
        // Every 10ms the output consumes 480 frames at 48kHz.
        let _ = consumer.read_frames(&mut output);
        clock_ms += 10;
        if step > 200 {
            depths.push(pump.queued_frames());
        }
    }

    // Once running, the ring holds roughly the configured 400ms cushion:
    // deep enough to absorb writer jitter, and far short of the ring's
    // 48000-frame capacity.
    let minimum = depths.iter().copied().min().expect("samples");
    let maximum = depths.iter().copied().max().expect("samples");
    assert!(
        minimum > 12_000,
        "cushion collapsed toward empty: minimum depth {minimum}"
    );
    assert!(
        maximum <= 20_160,
        "cushion grew past the cap: maximum depth {maximum}"
    );

    // With the cushion holding, the consumer never had to invent silence.
    let diagnostics = pump.diagnostics();
    assert_eq!(
        diagnostics.ring_underruns, 0,
        "a held cushion must prevent underruns"
    );
    assert_eq!(diagnostics.ring_silence_filled_frames, 0);
}

#[test]
fn a_post_start_ring_underrun_realigns_to_the_live_timeline() {
    let (mut pump, consumer) = pump_with(48_000);
    for sequence in 0..20 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    assert!(matches!(
        pump.tick(HOST_START_MS),
        PumpTick::Queued { sequence: 0, .. }
    ));

    // Consume the only queued packet plus one packet of silence. The ring is
    // now empty and wall time has advanced while the stream position has not.
    let mut output = vec![0.0_f32; 2 * usize::try_from(SAMPLES_PER_PACKET).expect("fits") * 2];
    let outcome = consumer.read_frames(&mut output);
    assert_eq!(
        outcome.frames_supplied,
        usize::try_from(SAMPLES_PER_PACKET).expect("fits")
    );
    assert!(outcome.frames_silence_filled > 0);

    // At +100ms, sequences 1..4 are now irretrievably late. Replaying them
    // would leave this listener permanently 80ms behind; realignment skips
    // only those stale slots and resumes on sequence 5, which is due now.
    let resumed = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));
    assert!(
        matches!(resumed, PumpTick::Queued { sequence: 5, .. }),
        "underrun recovery must catch the live timeline, got {resumed:?}"
    );
    assert_eq!(pump.diagnostics().sequences_skipped, 4);
}

#[test]
fn startup_silence_does_not_trigger_underrun_realign_or_skip_future_audio() {
    let (mut pump, consumer) = paced_pump();

    // The platform output can begin asking for frames before the playback
    // worker has placed the first scheduled frame. That pre-roll silence is
    // expected and must only establish the underrun baseline.
    let mut startup = vec![0.0_f32; 480 * 2];
    let startup_outcome = consumer.read_frames(&mut startup);
    assert_eq!(startup_outcome.frames_supplied, 0);
    assert_eq!(startup_outcome.frames_silence_filled, 480);

    for sequence in 0..40 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    let first = pump.tick(HOST_START_MS - 400);
    assert!(
        matches!(first, PumpTick::Queued { sequence: 0, .. }),
        "startup underrun telemetry must not skip the first future frame: {first:?}"
    );
    assert_eq!(pump.prefill_frames(), 19_200);
    assert_eq!(pump.diagnostics().sequences_skipped, 0);
}
