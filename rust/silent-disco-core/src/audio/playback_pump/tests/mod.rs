//! Shared fixtures for the `playback_pump` test clusters below. Each cluster
//! lives in its own sibling file and imports exactly the fixtures it needs
//! from here, rather than each rebuilding its own scheduler/ring/pump
//! plumbing.

// Exact round trips through the ring's bit-identical float storage, not
// approximate arithmetic.
#![allow(clippy::float_cmp)]

mod config;
mod conversion;
mod diagnostics;
mod recording;
mod scheduling;
mod sync;

use crate::audio::{
    PlaybackScheduler, RenderRing, RenderRingConfig, RenderRingConsumer, SchedulerConfig,
};
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram};

use super::{PlaybackPump, PlaybackPumpConfig};

pub(super) const PACKET_DURATION_MS: u32 = 20;
pub(super) const SAMPLES_PER_PACKET: u32 = 960;
pub(super) const HOST_START_MS: u64 = 1_000;

pub(super) fn datagram(sequence: u64, sample_value: i16) -> AudioDatagram {
    AudioDatagram {
        session_id: SessionId::new("session-pump").expect("session id"),
        stream_id: StreamId::new("stream-pump").expect("stream id"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: SAMPLES_PER_PACKET,
        first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
        host_presentation_time_ms: MonotonicMillis::new(
            HOST_START_MS + sequence * u64::from(PACKET_DURATION_MS),
        ),
        payload: (0..SAMPLES_PER_PACKET * 2)
            .flat_map(|_| sample_value.to_le_bytes())
            .collect(),
    }
}

pub(super) fn pump_with(capacity_frames: usize) -> (PlaybackPump, RenderRingConsumer) {
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
        capacity_frames,
        target_fill_frames: 1,
    })
    .expect("valid ring");
    let (producer, consumer) = ring.split();
    // Pacing off: these cases cover conversion and queueing semantics.
    // Write-lead, depth cap, and prefill have their own tests below.
    let mut pump = PlaybackPump::new(scheduler, producer, unpaced_config(capacity_frames))
        .expect("valid pump");
    pump.apply_sync_offset(0.0);
    (pump, consumer)
}

/// A pump with production pacing: a one-second ring, a 400ms write lead
/// and cushion, and an 800ms prefill ceiling.
pub(super) fn paced_pump_with(config: PlaybackPumpConfig) -> (PlaybackPump, RenderRingConsumer) {
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
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, consumer) = ring.split();
    let mut pump = PlaybackPump::new(scheduler, producer, config).expect("valid pump");
    // These cases exercise pacing, not the sync gate; lock it at zero
    // offset so the scheduler's timeline matches the test clock.
    pump.apply_sync_offset(0.0);
    (pump, consumer)
}

pub(super) fn paced_pump() -> (PlaybackPump, RenderRingConsumer) {
    paced_pump_with(PlaybackPumpConfig::default())
}

/// A paced pump whose sync gate has NOT been unlocked.
pub(super) fn pump_with_unlocked_sync() -> (PlaybackPump, RenderRingConsumer) {
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
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, consumer) = ring.split();
    let pump = PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
    (pump, consumer)
}

/// A config with pacing disabled and a depth cap the given ring can reach.
pub(super) fn unpaced_config(capacity_frames: usize) -> PlaybackPumpConfig {
    PlaybackPumpConfig {
        volume: 1.0,
        write_lead_ms: 0,
        max_prefill_ms: 0,
        target_depth_frames: capacity_frames,
    }
}
