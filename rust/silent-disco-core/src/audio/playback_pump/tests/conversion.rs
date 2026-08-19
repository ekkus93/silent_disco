//! PCM16-to-float32 conversion and volume scaling.

use crate::audio::{PlaybackScheduler, RenderRing, RenderRingConfig, SchedulerConfig};
use crate::domain::{SessionId, StreamId};

use super::super::{PlaybackPump, PlaybackPumpConfig, PumpTick};
use super::{
    HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, datagram, pump_with, unpaced_config,
};

#[test]
fn queues_a_due_frame_into_the_ring_as_normalized_float32() {
    let (mut pump, consumer) = pump_with(4_800);
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");

    let tick = pump.tick(HOST_START_MS);

    assert!(matches!(
        tick,
        PumpTick::Queued {
            sequence: 0,
            frames: 960,
            concealed: false
        }
    ));
    // A stream's first frame fades in, so sample the steady-state body
    // past the 240-frame (5ms) ramp rather than the very first sample.
    let mut output = vec![0.0_f32; 960 * 2];
    let outcome = consumer.read_frames(&mut output);
    assert_eq!(outcome.frames_supplied, 960);
    assert_eq!(output[0], 0.0);
    // 16384 / 32768 is exactly half scale.
    assert_eq!(output[300 * 2], 0.5);
    assert_eq!(output[300 * 2 + 1], 0.5);
}

#[test]
fn volume_scales_the_queued_samples() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 1,
    })
    .expect("valid ring");
    let (producer, consumer) = ring.split();
    let mut pump = PlaybackPump::new(
        scheduler,
        producer,
        PlaybackPumpConfig {
            volume: 0.5,
            ..unpaced_config(4_800)
        },
    )
    .expect("valid pump");
    pump.apply_sync_offset(0.0);
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");

    pump.tick(HOST_START_MS);

    let mut output = vec![0.0_f32; 960 * 2];
    let _ = consumer.read_frames(&mut output);
    assert_eq!(output[300 * 2], 0.25);
}

#[test]
fn dynamic_volume_updates_only_subsequently_converted_frames() {
    let (mut pump, consumer) = pump_with(4_800);
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");
    pump.scheduler_mut()
        .submit_packet(datagram(1, 16_384))
        .expect("accepted");

    let first = pump.tick(HOST_START_MS);
    assert!(matches!(first, PumpTick::Queued { sequence: 0, .. }));
    pump.set_volume(0.5).expect("valid dynamic volume");
    let second = pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS));
    assert!(matches!(second, PumpTick::Queued { sequence: 1, .. }));

    let mut output = vec![0.0_f32; 2 * 960 * 2];
    let outcome = consumer.read_frames(&mut output);
    assert_eq!(outcome.frames_supplied, 2 * 960);
    assert_eq!(output[300 * 2], 0.5);
    assert_eq!(output[(960 + 300) * 2], 0.25);

    assert!(pump.set_volume(f32::NAN).is_err());
    assert!(pump.set_volume(-0.01).is_err());
    assert!(pump.set_volume(1.01).is_err());
}
