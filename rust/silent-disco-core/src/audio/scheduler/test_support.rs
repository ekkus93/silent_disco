//! Shared fixtures for the `scheduler` test clusters below. Each cluster
//! lives in its own sibling file and imports exactly the fixtures it needs
//! from here, rather than each rebuilding its own session/stream/datagram
//! plumbing.

use crate::audio::DEFAULT_CONCEALMENT_RAMP_MS;
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram};

use super::{PlaybackScheduler, ScheduledFrame, SchedulerConfig, SchedulerPoll};

pub(super) const PACKET_DURATION_MS: u32 = 20;
pub(super) const SAMPLES_PER_PACKET: u32 = 960;
pub(super) const CHANNELS: u16 = 2;
pub(super) const HOST_START_MS: u64 = 1_000;

/// Ramp length for the default config: 5ms of a 20ms/960-sample packet.
/// Frames the scheduler ramps over, derived exactly as it derives them so
/// this stays correct if either the ramp or the packet duration moves.
pub(super) const RAMP_FRAMES: usize =
    (SAMPLES_PER_PACKET * DEFAULT_CONCEALMENT_RAMP_MS / PACKET_DURATION_MS) as usize;

pub(super) fn session() -> SessionId {
    SessionId::new("session-scheduler").expect("session id")
}

pub(super) fn stream() -> StreamId {
    StreamId::new("stream-scheduler").expect("stream id")
}

pub(super) fn config() -> SchedulerConfig {
    SchedulerConfig::new(
        session(),
        stream(),
        PACKET_DURATION_MS,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        CHANNELS,
    )
}

pub(super) fn payload_for(sample_value: i16) -> Vec<u8> {
    (0..usize::try_from(SAMPLES_PER_PACKET).expect("fits usize") * usize::from(CHANNELS))
        .flat_map(|_| sample_value.to_le_bytes())
        .collect()
}

pub(super) fn datagram(sequence: u64, sample_value: i16) -> AudioDatagram {
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

/// Drives a scheduler past its startup buffer with `count` packets of
/// `sample_value`, returning it ready to deliver sequence zero.
pub(super) fn buffered_scheduler(count: u64, sample_value: i16) -> PlaybackScheduler {
    let mut scheduler = PlaybackScheduler::new(config(), 0.0).expect("valid scheduler");
    for sequence in 0..count {
        scheduler
            .submit_packet(datagram(sequence, sample_value))
            .expect("accepted");
    }
    scheduler
}

pub(super) fn frame_at(poll: SchedulerPoll) -> ScheduledFrame {
    match poll {
        SchedulerPoll::Frame { frame, .. } => frame,
        other => panic!("expected a frame, got {other:?}"),
    }
}
