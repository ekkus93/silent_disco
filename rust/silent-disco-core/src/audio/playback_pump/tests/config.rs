//! Construction-validation rejections: volume, channel count, target depth.

use crate::audio::{PlaybackScheduler, RenderRing, RenderRingConfig, SchedulerConfig};
use crate::domain::{SessionId, StreamId};

use super::super::{PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigErrorKind};
use super::{HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, pump_with, unpaced_config};

#[test]
fn rejects_a_target_depth_the_ring_could_never_reach() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
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
    let (producer, _consumer) = ring.split();

    let error = PlaybackPump::new(
        scheduler,
        producer,
        PlaybackPumpConfig {
            target_depth_frames: 9_600,
            ..unpaced_config(4_800)
        },
    )
    .expect_err("a depth beyond the ring's capacity must be rejected");
    assert_eq!(error.kind, PlaybackPumpConfigErrorKind::InvalidTargetDepth);
}

#[test]
fn rejects_an_invalid_volume() {
    let (pump, _consumer) = pump_with(4_800);
    drop(pump);

    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
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
    let (producer, _consumer) = ring.split();

    let error = PlaybackPump::new(
        scheduler,
        producer,
        PlaybackPumpConfig {
            volume: 1.5,
            ..unpaced_config(4_800)
        },
    )
    .expect_err("an out-of-range volume must be rejected");
    assert_eq!(error.kind, PlaybackPumpConfigErrorKind::InvalidVolume);
}

#[test]
fn rejects_a_stream_whose_channel_count_the_ring_cannot_render() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        1,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 1,
    })
    .expect("valid ring");
    let (producer, _consumer) = ring.split();

    let error = PlaybackPump::new(scheduler, producer, unpaced_config(4_800))
        .expect_err("a mono stream must be rejected by a stereo ring");
    assert_eq!(
        error.kind,
        PlaybackPumpConfigErrorKind::ChannelCountMismatch
    );
}
