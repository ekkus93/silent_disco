//! [`PlaybackPump`] itself: construction, and the trivial accessors that do
//! not belong to any of the sync/scheduling/conversion/recording/diagnostics
//! clusters implemented in the sibling files.

use crate::audio::{DebugPcmRecorder, PlaybackScheduler, RENDER_CHANNELS, RenderRingProducer};

use super::config::{PlaybackPumpConfig, PlaybackPumpConfigError, PlaybackPumpConfigErrorKind};

/// See the [module documentation](super) for the real-time/non-real-time
/// boundary this type sits on and the "never discard a partial write"
/// invariant its `impl` (spread across sibling files) upholds.
///
/// Fields are `pub(super)`, not private: [`super::sync`], [`super::scheduling`],
/// [`super::conversion`], [`super::recording`], and [`super::diagnostics`]
/// each implement one cluster of this struct's methods and need direct field
/// access to do it without an accessor for every private field.
#[derive(Debug)]
pub struct PlaybackPump {
    pub(super) scheduler: PlaybackScheduler,
    pub(super) producer: RenderRingProducer,
    pub(super) config: PlaybackPumpConfig,
    /// Interleaved float32 samples converted but not yet accepted by the ring.
    pub(super) pending: Vec<f32>,
    /// Cleared once the stream-start alignment prefill has been queued.
    pub(super) awaiting_prefill: bool,
    /// Silence frames queued to align the first frame with its deadline.
    pub(super) prefill_frames: usize,
    /// Set once a real clock-offset estimate has been applied.
    pub(super) sync_locked: bool,
    /// The offset currently mapping host presentation times onto local time.
    pub(super) offset_ms: f64,
    /// Largest ring depth observed, in frames.
    pub(super) peak_queued_frames: usize,
    /// Packets discarded because they arrived before sync locked.
    pub(super) dropped_before_sync: u64,
    /// Render-ring underrun count observed on the previous pump tick.
    ///
    /// Before the first frame has been placed on the timeline this is only a
    /// baseline: startup/pre-roll silence is expected and must not be mistaken
    /// for mid-stream drift.
    pub(super) last_observed_underrun_callbacks: u64,
    /// True after the first scheduled frame has been handed toward the render
    /// ring. Only then can a later underrun represent accumulated playout
    /// drift that needs timeline realignment.
    pub(super) timeline_started: bool,
    /// Times a clock-offset jump too large to splice forced a rebuffer --
    /// the counterpart to `ConcealmentStatistics::hard_resync_signals`, which
    /// only counts the *other* rebuffer cause (the consecutive-concealment
    /// bound). Before A4.4 this was produced (`SyncApplyOutcome::Rebuffered`)
    /// but never counted anywhere, so `hard_resync_signals` under-reported:
    /// a stream could rebuffer repeatedly from offset jumps alone and still
    /// read `hardResyncs=0`. See `PlaybackDiagnostics::hard_resync_signals`.
    pub(super) offset_driven_rebuffers: u64,
    /// Optional capture of exactly what was released toward the ring.
    pub(super) recorder: Option<DebugPcmRecorder>,
    /// First recorder failure, kept so a broken capture is visible rather than
    /// silently producing a truncated file.
    pub(super) recorder_error: Option<String>,
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
        if config.target_depth_frames == 0
            || config.target_depth_frames > producer.capacity_frames()
        {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::InvalidTargetDepth,
                message: format!(
                    "target depth of {} frames must be nonzero and within the ring's {}-frame \
                     capacity",
                    config.target_depth_frames,
                    producer.capacity_frames()
                ),
            });
        }
        let last_observed_underrun_callbacks = producer.telemetry().underrun_callbacks;
        Ok(Self {
            scheduler,
            producer,
            config,
            pending: Vec::new(),
            awaiting_prefill: true,
            prefill_frames: 0,
            sync_locked: false,
            offset_ms: 0.0,
            peak_queued_frames: 0,
            dropped_before_sync: 0,
            last_observed_underrun_callbacks,
            timeline_started: false,
            offset_driven_rebuffers: 0,
            recorder: None,
            recorder_error: None,
        })
    }

    /// The scheduler this pump drives, for submitting packets and applying
    /// clock-offset updates.
    pub const fn scheduler_mut(&mut self) -> &mut PlaybackScheduler {
        &mut self.scheduler
    }

    /// Updates the linear output gain used for subsequently converted frames.
    ///
    /// Already-converted samples waiting in the render ring or pending queue keep
    /// the gain they were converted with; the new value applies at the next frame
    /// conversion boundary.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackPumpConfigErrorKind::InvalidVolume`] when `volume` is
    /// NaN, infinite, or outside `0.0..=1.0`.
    pub fn set_volume(&mut self, volume: f32) -> Result<(), PlaybackPumpConfigError> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::InvalidVolume,
                message: format!("volume of {volume} must be finite and within 0.0..=1.0"),
            });
        }
        self.config.volume = volume;
        Ok(())
    }

    /// Sample rate of the stream this pump serves.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.scheduler.sample_rate()
    }

    /// Silence frames queued at stream start to align the first frame with its
    /// presentation deadline.
    #[must_use]
    pub const fn prefill_frames(&self) -> usize {
        self.prefill_frames
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
}
