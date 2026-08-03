use core::fmt;
use std::error::Error;

use super::{
    PlaybackScheduler, RENDER_CHANNELS, RenderRingProducer, ScheduledFrame, SchedulerPoll,
};

/// Full-scale magnitude of a 16-bit PCM sample, used to normalize into the
/// render ring's float32 representation.
const PCM16_FULL_SCALE: f32 = 32_768.0;

/// Tuning for one [`PlaybackPump`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackPumpConfig {
    /// Linear output gain applied while converting to the ring's float32
    /// format, in `0.0..=1.0`.
    pub volume: f32,
}

impl Default for PlaybackPumpConfig {
    fn default() -> Self {
        Self { volume: 1.0 }
    }
}

/// Stable failure taxonomy for pump configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPumpConfigErrorKind {
    /// `volume` is not a finite value within `0.0..=1.0`.
    InvalidVolume,
    /// The scheduler's channel count does not match the render ring's fixed
    /// interleaved channel count.
    ChannelCountMismatch,
}

/// Typed pump configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackPumpConfigError {
    /// Stable semantic failure category.
    pub kind: PlaybackPumpConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for PlaybackPumpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for PlaybackPumpConfigError {}

/// What one [`PlaybackPump::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpTick {
    /// The scheduler is still accumulating its startup presentation buffer.
    Buffering {
        /// Buffered span accumulated so far, in milliseconds.
        buffered_ms: u64,
    },
    /// A frame was converted and queued into the render ring.
    Queued {
        /// Sequence of the frame queued.
        sequence: u64,
        /// Frames actually accepted by the ring this tick.
        frames: usize,
        /// True when the queued frame was synthesized rather than delivered.
        concealed: bool,
    },
    /// Part of an earlier frame that the ring had no room for was accepted
    /// this tick.
    FlushedPending {
        /// Frames accepted this tick.
        frames: usize,
    },
    /// The ring had no room for the frame in hand; the unwritten remainder is
    /// held and retried on the next tick rather than being dropped.
    RingFull {
        /// Frames still waiting to be accepted by the ring.
        pending_frames: usize,
    },
    /// Nothing is due for this tick.
    Waiting,
    /// The scheduler is paused until an explicit rebuffer.
    AwaitingRebuffer,
    /// The scheduler has stopped and will produce no further frames.
    Stopped,
}

/// Drives one [`PlaybackScheduler`] into one [`RenderRingProducer`],
/// converting each scheduled frame from interleaved PCM16 into the ring's
/// interleaved float32 format.
///
/// This is the non-real-time half of playback: it runs on an ordinary worker
/// thread and may allocate, while the real-time audio callback only ever
/// reads from the ring through the narrow C ABI. Frames the ring cannot
/// accept immediately are held and retried, never discarded — silently
/// dropping the unwritten remainder of a partial write is audible as a
/// corrupted note rather than a clean gap.
#[derive(Debug)]
pub struct PlaybackPump {
    scheduler: PlaybackScheduler,
    producer: RenderRingProducer,
    config: PlaybackPumpConfig,
    /// Interleaved float32 samples converted but not yet accepted by the ring.
    pending: Vec<f32>,
}

impl PlaybackPump {
    /// Creates a pump driving `scheduler` into `producer`.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackPumpConfigErrorKind::InvalidVolume`] when `volume` is
    /// not finite and within `0.0..=1.0`, or
    /// [`PlaybackPumpConfigErrorKind::ChannelCountMismatch`] when the
    /// scheduler's stream is not the ring's fixed interleaved channel count.
    pub fn new(
        scheduler: PlaybackScheduler,
        producer: RenderRingProducer,
        config: PlaybackPumpConfig,
    ) -> Result<Self, PlaybackPumpConfigError> {
        if !config.volume.is_finite() || config.volume < 0.0 || config.volume > 1.0 {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::InvalidVolume,
                message: format!(
                    "volume of {} must be finite and within 0.0..=1.0",
                    config.volume
                ),
            });
        }
        let channels = scheduler.channels();
        if usize::from(channels) != RENDER_CHANNELS {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::ChannelCountMismatch,
                message: format!(
                    "stream has {channels} channels but the render ring is fixed at \
                     {RENDER_CHANNELS}"
                ),
            });
        }
        Ok(Self {
            scheduler,
            producer,
            config,
            pending: Vec::new(),
        })
    }

    /// The scheduler this pump drives, for submitting packets and applying
    /// clock-offset updates.
    pub const fn scheduler_mut(&mut self) -> &mut PlaybackScheduler {
        &mut self.scheduler
    }

    /// Frames converted but not yet accepted by the ring.
    #[must_use]
    pub fn pending_frames(&self) -> usize {
        self.pending.len() / RENDER_CHANNELS
    }

    /// Frames currently queued in the ring, written but not yet consumed.
    #[must_use]
    pub fn queued_frames(&self) -> usize {
        self.producer.capacity_frames() - self.producer.free_frames()
    }

    /// Advances playback by one tick at the given local monotonic time.
    pub fn tick(&mut self, local_now_ms: u64) -> PumpTick {
        if !self.pending.is_empty() {
            // Never poll for new audio while an earlier frame is still
            // partly unwritten: that would reorder the stream.
            let flushed = self.flush_pending();
            return if self.pending.is_empty() {
                PumpTick::FlushedPending { frames: flushed }
            } else {
                PumpTick::RingFull {
                    pending_frames: self.pending_frames(),
                }
            };
        }

        match self.scheduler.poll(local_now_ms) {
            SchedulerPoll::Buffering { buffered_ms } => PumpTick::Buffering { buffered_ms },
            SchedulerPoll::Waiting { .. } => PumpTick::Waiting,
            SchedulerPoll::AwaitingRebuffer => PumpTick::AwaitingRebuffer,
            SchedulerPoll::Stopped => PumpTick::Stopped,
            SchedulerPoll::Frame { frame, .. } => {
                let sequence = frame.sequence;
                let concealed = frame.concealed;
                let frames = self.enqueue_frame(&frame);
                if self.pending.is_empty() {
                    PumpTick::Queued {
                        sequence,
                        frames,
                        concealed,
                    }
                } else {
                    PumpTick::RingFull {
                        pending_frames: self.pending_frames(),
                    }
                }
            }
        }
    }

    /// Drains the scheduler's buffered tail into the ring and stops it.
    ///
    /// Returns the number of frames accepted by the ring. The ring may not
    /// have room for all of it at once; whatever does not fit is held pending
    /// and can be flushed with further [`Self::tick`] calls before the output
    /// is torn down. The held amount is bounded by the drained tail itself,
    /// which the jitter buffer already bounds.
    pub fn finish(&mut self) -> usize {
        let drained = self.scheduler.drain_remaining();
        let mut written = 0;
        for frame in &drained {
            written += self.enqueue_frame(frame);
        }
        self.scheduler.stop();
        written
    }

    /// Converts one frame and appends it to the pending queue, then pushes as
    /// much of that queue as the ring will take. Appending rather than
    /// replacing keeps playback order intact when an earlier frame was only
    /// partly accepted.
    fn enqueue_frame(&mut self, frame: &ScheduledFrame) -> usize {
        self.pending.reserve(frame.samples.len());
        for &sample in &frame.samples {
            self.pending
                .push(f32::from(sample) / PCM16_FULL_SCALE * self.config.volume);
        }
        self.flush_pending()
    }

    /// Pushes as much of `pending` as the ring accepts, retaining the rest.
    fn flush_pending(&mut self) -> usize {
        if self.pending.is_empty() {
            return 0;
        }
        let written = self.producer.push_frames(&self.pending);
        if written > 0 {
            self.pending.drain(..written * RENDER_CHANNELS);
        }
        written
    }
}

#[cfg(test)]
mod tests {
    // Exact round trips through the ring's bit-identical float storage, not
    // approximate arithmetic.
    #![allow(clippy::float_cmp)]

    use super::{PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigErrorKind, PumpTick};
    use crate::audio::{
        PlaybackScheduler, RenderRing, RenderRingConfig, RenderRingConsumer, SchedulerConfig,
    };
    use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
    use crate::protocol::{AudioCodec, AudioDatagram};

    const PACKET_DURATION_MS: u32 = 20;
    const SAMPLES_PER_PACKET: u32 = 960;
    const HOST_START_MS: u64 = 1_000;

    fn datagram(sequence: u64, sample_value: i16) -> AudioDatagram {
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

    fn pump_with(capacity_frames: usize) -> (PlaybackPump, RenderRingConsumer) {
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
            capacity_frames,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        let pump = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default())
            .expect("valid pump");
        (pump, consumer)
    }

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
        let (producer, consumer) = ring.split();
        let mut pump = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig { volume: 0.5 })
            .expect("valid pump");
        pump.scheduler_mut()
            .submit_packet(datagram(0, 16_384))
            .expect("accepted");

        pump.tick(HOST_START_MS);

        let mut output = vec![0.0_f32; 960 * 2];
        let _ = consumer.read_frames(&mut output);
        assert_eq!(output[300 * 2], 0.25);
    }

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

        // The ring is now full: the sixth frame cannot be written at all, and
        // must be held rather than partly discarded.
        let blocked = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));
        assert!(matches!(blocked, PumpTick::RingFull { .. }));
        assert_eq!(pump.pending_frames(), 960);

        // Consuming half a packet's worth lets exactly that much through.
        let mut output = vec![0.0_f32; 480 * 2];
        let _ = consumer.read_frames(&mut output);
        let partial = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));
        assert!(matches!(partial, PumpTick::RingFull { .. }));
        assert_eq!(pump.pending_frames(), 480);

        // Consuming the rest clears the backlog with no audio lost.
        let _ = consumer.read_frames(&mut output);
        let cleared = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));
        assert!(matches!(cleared, PumpTick::FlushedPending { frames: 480 }));
        assert_eq!(pump.pending_frames(), 0);
    }

    #[test]
    fn reports_buffering_waiting_and_stopped_states() {
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
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default()).expect("pump");

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

        let error = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig { volume: 1.5 })
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

        let error = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default())
            .expect_err("a mono stream must be rejected by a stereo ring");
        assert_eq!(
            error.kind,
            PlaybackPumpConfigErrorKind::ChannelCountMismatch
        );
    }
}
