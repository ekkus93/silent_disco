mod concealment;
mod decoder;
mod jitter_buffer;
mod packetizer;
mod packetizer_worker;
mod ramp;
mod render_ring;
mod resampler;
mod scheduler;
mod source;
mod types;

pub use concealment::{
    ConcealmentConfigError, ConcealmentConfigErrorKind, ConcealmentOutcome, ConcealmentPolicy,
    ConcealmentStatistics, DEFAULT_CONCEALMENT_RAMP_MS, DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS,
    MAX_CONCEALMENT_RAMP_FRAMES, MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT,
};
pub use decoder::StreamingDecodeHandle;
pub use jitter_buffer::{
    DEFAULT_MAX_BUFFERED_DURATION_MS, DEFAULT_MAX_REORDER_WINDOW, JitterBuffer, JitterBufferConfig,
    JitterBufferConfigError, JitterBufferConfigErrorKind, JitterBufferRejection,
    JitterBufferRejectionKind, JitterBufferStatistics, MAX_BUFFERED_DURATION_LIMIT_MS,
    MAX_REORDER_WINDOW_LIMIT,
};
pub use packetizer::{
    DEFAULT_PACKET_DURATION_MS, MAX_PACKET_DURATION_MS, MIN_PACKET_DURATION_MS, PacketizeOutcome,
    Packetizer, PacketizerError, PacketizerErrorKind,
};
pub use packetizer_worker::{
    DEFAULT_PACKETIZER_QUEUE_CAPACITY, MAX_PACKETIZER_QUEUE_CAPACITY, PacketizerSummary,
    PacketizerWorkerError, PacketizerWorkerErrorKind, PacketizerWorkerState,
    StreamingPacketizeConfig, StreamingPacketizeHandle,
};
pub use render_ring::{
    DEFAULT_RING_CAPACITY_FRAMES, DEFAULT_TARGET_FILL_FRAMES, MAX_RING_CAPACITY_FRAMES,
    MIN_RING_CAPACITY_FRAMES, RENDER_CHANNELS, RenderReadOutcome, RenderRing, RenderRingConfig,
    RenderRingConfigError, RenderRingConfigErrorKind, RenderRingConsumer, RenderRingProducer,
    RenderRingSnapshot,
};
pub use scheduler::{
    BufferHealth, DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS, DEFAULT_HARD_RESYNC_THRESHOLD_MS,
    DEFAULT_HIGH_WATER_MS, DEFAULT_LOW_WATER_MS, DEFAULT_STARTUP_BUFFER_TARGET_MS,
    OffsetUpdateOutcome, PlaybackScheduler, ScheduledFrame, SchedulerConfig, SchedulerConfigError,
    SchedulerConfigErrorKind, SchedulerPoll,
};
pub use types::{
    AudioFormat, AudioSampleFormat, CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ,
    DEFAULT_DECODE_CHUNK_FRAMES, DEFAULT_DECODE_QUEUE_CHUNKS, DecodeError, DecodeErrorKind,
    DecodeStatistics, DecodeStreamInfo, DecodeSummary, DecodeWorkerState, DecodedPcmChunk,
    MAX_DECODE_CHUNK_FRAMES, MAX_DECODE_QUEUE_CHUNKS, MAX_SOURCE_BYTES, StreamingDecodeConfig,
};

#[cfg(test)]
mod concealment_tests;
#[cfg(test)]
mod jitter_buffer_tests;
#[cfg(test)]
mod packetizer_tests;
#[cfg(test)]
mod ramp_tests;
#[cfg(test)]
mod render_ring_tests;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod tests;
