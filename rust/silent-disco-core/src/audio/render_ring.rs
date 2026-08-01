//! Bounded single-producer/single-consumer render ring.
//!
//! This ring stores only ordered, render-ready interleaved float32 stereo
//! frames; it never grows after construction and contains no protocol
//! objects. [`RenderRing::split`] consumes the ring and hands back exactly
//! one [`RenderRingProducer`] and one [`RenderRingConsumer`] — since neither
//! handle implements `Clone`, the type system (not a runtime check) is what
//! guarantees exactly one producer and exactly one consumer ever exist for a
//! given ring.
//!
//! ## Atomic ordering, documented explicitly
//!
//! Each sample slot is a plain [`AtomicU32`] holding an `f32`'s bit pattern
//! (`f32::to_bits`/`from_bits`), which keeps every read and write of sample
//! data itself safe Rust with no `unsafe` block anywhere in this module.
//! Coordination between the producer and consumer uses the standard
//! "batch release/acquire" technique for lock-free single-producer/
//! single-consumer queues:
//!
//! - The producer is the only writer of `write_index`. It writes sample
//!   slots with `Relaxed` ordering, then publishes them by storing the new
//!   `write_index` with `Release`. It reads its own `write_index` with
//!   `Relaxed` (no concurrent writer) and reads `read_index` with `Acquire`
//!   to receive the consumer's progress; an observed-stale `read_index` can
//!   only under-count free space, never overstate it, so the producer can
//!   never overwrite an unread frame.
//! - The consumer is the only writer of `read_index`. It reads `write_index`
//!   with `Acquire` (pairing with the producer's `Release`) to ensure every
//!   sample slot up to that index is visible before loading it, then loads
//!   sample slots with `Relaxed`, then publishes its progress by storing the
//!   new `read_index` with `Release`. It reads its own `read_index` with
//!   `Relaxed`.
//!
//! Because `write_index` only ever increases and the consumer only ever
//! advances `read_index` by an amount it computed as available under some
//! earlier-or-equal observation of `write_index`, `write_index - read_index`
//! can never underflow from either side's point of view.

use core::fmt;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Interleaved channel count of the fixed internal render format (stereo).
pub const RENDER_CHANNELS: usize = 2;
/// Default ring capacity in frames, equal to one second at 48 kHz.
pub const DEFAULT_RING_CAPACITY_FRAMES: usize = 48_000;
/// Default initial target fill in frames, equal to 400 ms at 48 kHz.
pub const DEFAULT_TARGET_FILL_FRAMES: usize = 19_200;
/// Hard minimum accepted ring capacity, equal to 100 ms at 48 kHz.
pub const MIN_RING_CAPACITY_FRAMES: usize = 4_800;
/// Hard maximum accepted ring capacity, equal to 10 seconds at 48 kHz.
pub const MAX_RING_CAPACITY_FRAMES: usize = 480_000;

/// Fixed capacity and startup-fill target for one render ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRingConfig {
    /// Total frames the ring holds; never grows after construction.
    pub capacity_frames: usize,
    /// Frames the producer is expected to accumulate before playback starts;
    /// purely informational here and enforced by a future scheduler/producer
    /// worker, not by the ring itself.
    pub target_fill_frames: usize,
}

impl RenderRingConfig {
    /// Recommended default: one second of capacity, 400 ms target fill.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            capacity_frames: DEFAULT_RING_CAPACITY_FRAMES,
            target_fill_frames: DEFAULT_TARGET_FILL_FRAMES,
        }
    }
}

impl Default for RenderRingConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable failure taxonomy for render ring configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderRingConfigErrorKind {
    /// `capacity_frames` is outside `[MIN_RING_CAPACITY_FRAMES, MAX_RING_CAPACITY_FRAMES]`.
    CapacityOutOfRange,
    /// `target_fill_frames` is zero or exceeds `capacity_frames`.
    TargetFillOutOfRange,
    /// `capacity_frames * RENDER_CHANNELS` would overflow `usize`.
    CapacityOverflowsAddressSpace,
}

/// Typed render ring configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRingConfigError {
    /// Stable semantic failure category.
    pub kind: RenderRingConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for RenderRingConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for RenderRingConfigError {}

/// Cumulative, lock-free atomic counters describing everything a render ring
/// and its eventual real-time callback have observed.
#[derive(Debug, Default)]
pub struct RenderRingTelemetry {
    frames_produced: AtomicU64,
    frames_requested: AtomicU64,
    frames_supplied_from_ring: AtomicU64,
    silence_filled_frames: AtomicU64,
    underrun_callbacks: AtomicU64,
    ring_full_events: AtomicU64,
    callback_count: AtomicU64,
    contained_panic_count: AtomicU64,
}

impl RenderRingTelemetry {
    fn snapshot(&self) -> RenderRingSnapshot {
        RenderRingSnapshot {
            frames_produced: self.frames_produced.load(Ordering::Relaxed),
            frames_requested: self.frames_requested.load(Ordering::Relaxed),
            frames_supplied_from_ring: self.frames_supplied_from_ring.load(Ordering::Relaxed),
            silence_filled_frames: self.silence_filled_frames.load(Ordering::Relaxed),
            underrun_callbacks: self.underrun_callbacks.load(Ordering::Relaxed),
            ring_full_events: self.ring_full_events.load(Ordering::Relaxed),
            callback_count: self.callback_count.load(Ordering::Relaxed),
            contained_panic_count: self.contained_panic_count.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time read of every [`RenderRingTelemetry`] counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderRingSnapshot {
    /// Frames the producer has successfully written across this ring's lifetime.
    pub frames_produced: u64,
    /// Frames a consumer read has requested across this ring's lifetime.
    pub frames_requested: u64,
    /// Frames actually supplied from ring contents (never silence-filled).
    pub frames_supplied_from_ring: u64,
    /// Frames filled with silence because the ring did not hold enough data.
    pub silence_filled_frames: u64,
    /// Consumer reads that had to silence-fill at least one frame.
    pub underrun_callbacks: u64,
    /// Producer writes that could not fit every requested frame.
    pub ring_full_events: u64,
    /// Real-time callback invocations recorded via [`RenderRingConsumer::record_callback_invocation`].
    pub callback_count: u64,
    /// Panics caught and contained at the real-time boundary, recorded via
    /// [`RenderRingConsumer::record_contained_panic`].
    pub contained_panic_count: u64,
}

#[derive(Debug)]
struct RingInner {
    capacity_frames: usize,
    slots: Box<[AtomicU32]>,
    write_index: AtomicUsize,
    read_index: AtomicUsize,
    telemetry: RenderRingTelemetry,
}

/// A preallocated, bounded single-producer/single-consumer render ring.
///
/// Call [`Self::split`] to obtain the producer and consumer halves; this
/// type itself has no read/write methods.
#[derive(Debug)]
pub struct RenderRing {
    inner: Arc<RingInner>,
}

impl RenderRing {
    /// Creates a render ring, preallocating its entire backing storage.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderRingConfigError`] when `capacity_frames` is outside
    /// the hard bounds, `target_fill_frames` is zero or exceeds
    /// `capacity_frames`, or the configured capacity would overflow `usize`
    /// once multiplied by the channel count.
    pub fn new(config: RenderRingConfig) -> Result<Self, RenderRingConfigError> {
        if !(MIN_RING_CAPACITY_FRAMES..=MAX_RING_CAPACITY_FRAMES).contains(&config.capacity_frames)
        {
            return Err(RenderRingConfigError {
                kind: RenderRingConfigErrorKind::CapacityOutOfRange,
                message: format!(
                    "capacity of {} frames is outside the {MIN_RING_CAPACITY_FRAMES}..={MAX_RING_CAPACITY_FRAMES}-frame range",
                    config.capacity_frames
                ),
            });
        }
        if config.target_fill_frames == 0 || config.target_fill_frames > config.capacity_frames {
            return Err(RenderRingConfigError {
                kind: RenderRingConfigErrorKind::TargetFillOutOfRange,
                message: format!(
                    "target fill of {} frames must be nonzero and at most the {}-frame capacity",
                    config.target_fill_frames, config.capacity_frames
                ),
            });
        }
        let slot_count = config
            .capacity_frames
            .checked_mul(RENDER_CHANNELS)
            .ok_or_else(|| RenderRingConfigError {
                kind: RenderRingConfigErrorKind::CapacityOverflowsAddressSpace,
                message: format!(
                    "capacity of {} frames at {RENDER_CHANNELS} channels overflows the platform's address space",
                    config.capacity_frames
                ),
            })?;

        let slots = (0..slot_count)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Ok(Self {
            inner: Arc::new(RingInner {
                capacity_frames: config.capacity_frames,
                slots,
                write_index: AtomicUsize::new(0),
                read_index: AtomicUsize::new(0),
                telemetry: RenderRingTelemetry::default(),
            }),
        })
    }

    /// Consumes this ring, returning its single producer and single consumer
    /// handle. Neither handle is `Clone`, so this is the only way to ever
    /// obtain one, and it can only ever be obtained once per ring.
    #[must_use]
    pub fn split(self) -> (RenderRingProducer, RenderRingConsumer) {
        (
            RenderRingProducer {
                inner: Arc::clone(&self.inner),
            },
            RenderRingConsumer { inner: self.inner },
        )
    }
}

/// The render ring's sole producer handle.
#[derive(Debug)]
pub struct RenderRingProducer {
    inner: Arc<RingInner>,
}

impl RenderRingProducer {
    /// Writes as many complete interleaved frames from `frames` as fit
    /// without overwriting an unread frame, returning the count actually
    /// written. Any trailing samples that do not form a complete frame are
    /// ignored. Never blocks and performs no allocation.
    #[must_use]
    pub fn push_frames(&self, frames: &[f32]) -> usize {
        let available_frames = frames.len() / RENDER_CHANNELS;
        let write_index = self.inner.write_index.load(Ordering::Relaxed);
        let read_index = self.inner.read_index.load(Ordering::Acquire);
        let free_frames = self.inner.capacity_frames - (write_index - read_index);
        let frames_to_write = available_frames.min(free_frames);

        for offset in 0..frames_to_write {
            let slot_frame = (write_index + offset) % self.inner.capacity_frames;
            for channel in 0..RENDER_CHANNELS {
                let sample = frames[offset * RENDER_CHANNELS + channel];
                self.inner.slots[slot_frame * RENDER_CHANNELS + channel]
                    .store(sample.to_bits(), Ordering::Relaxed);
            }
        }
        if frames_to_write > 0 {
            self.inner
                .write_index
                .store(write_index + frames_to_write, Ordering::Release);
        }

        self.record_push_telemetry(frames_to_write, available_frames);
        frames_to_write
    }

    fn record_push_telemetry(&self, frames_written: usize, frames_requested: usize) {
        let telemetry = &self.inner.telemetry;
        telemetry
            .frames_produced
            .fetch_add(u64_from_usize(frames_written), Ordering::Relaxed);
        if frames_written < frames_requested {
            telemetry.ring_full_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Total frame capacity of the ring.
    #[must_use]
    pub fn capacity_frames(&self) -> usize {
        self.inner.capacity_frames
    }

    /// Frames currently free to write without overwriting unread data.
    #[must_use]
    pub fn free_frames(&self) -> usize {
        let write_index = self.inner.write_index.load(Ordering::Relaxed);
        let read_index = self.inner.read_index.load(Ordering::Acquire);
        self.inner.capacity_frames - (write_index - read_index)
    }

    /// Point-in-time read of every telemetry counter.
    #[must_use]
    pub fn telemetry(&self) -> RenderRingSnapshot {
        self.inner.telemetry.snapshot()
    }
}

/// Outcome of one [`RenderRingConsumer::read_frames`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderReadOutcome {
    /// Frames copied from ring contents.
    pub frames_supplied: usize,
    /// Frames filled with silence because the ring did not hold enough data.
    pub frames_silence_filled: usize,
}

/// The render ring's sole consumer handle.
#[derive(Debug)]
pub struct RenderRingConsumer {
    inner: Arc<RingInner>,
}

impl RenderRingConsumer {
    /// Reads up to `output.len() / RENDER_CHANNELS` interleaved frames into
    /// `output`, filling any remaining requested frames with silence. Any
    /// trailing samples in `output` that do not form a complete frame are
    /// left untouched. Never blocks and performs no allocation.
    #[must_use]
    pub fn read_frames(&self, output: &mut [f32]) -> RenderReadOutcome {
        let requested_frames = output.len() / RENDER_CHANNELS;
        let write_index = self.inner.write_index.load(Ordering::Acquire);
        let read_index = self.inner.read_index.load(Ordering::Relaxed);
        let available_frames = write_index - read_index;
        let frames_to_supply = requested_frames.min(available_frames);

        for offset in 0..frames_to_supply {
            let slot_frame = (read_index + offset) % self.inner.capacity_frames;
            for channel in 0..RENDER_CHANNELS {
                let bits = self.inner.slots[slot_frame * RENDER_CHANNELS + channel]
                    .load(Ordering::Relaxed);
                output[offset * RENDER_CHANNELS + channel] = f32::from_bits(bits);
            }
        }
        for offset in frames_to_supply..requested_frames {
            for channel in 0..RENDER_CHANNELS {
                output[offset * RENDER_CHANNELS + channel] = 0.0;
            }
        }
        if frames_to_supply > 0 {
            self.inner
                .read_index
                .store(read_index + frames_to_supply, Ordering::Release);
        }

        let frames_silence_filled = requested_frames - frames_to_supply;
        self.record_read_telemetry(requested_frames, frames_to_supply, frames_silence_filled);

        RenderReadOutcome {
            frames_supplied: frames_to_supply,
            frames_silence_filled,
        }
    }

    fn record_read_telemetry(&self, requested: usize, supplied: usize, silence_filled: usize) {
        let telemetry = &self.inner.telemetry;
        telemetry
            .frames_requested
            .fetch_add(u64_from_usize(requested), Ordering::Relaxed);
        telemetry
            .frames_supplied_from_ring
            .fetch_add(u64_from_usize(supplied), Ordering::Relaxed);
        telemetry
            .silence_filled_frames
            .fetch_add(u64_from_usize(silence_filled), Ordering::Relaxed);
        if silence_filled > 0 {
            telemetry.underrun_callbacks.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Records one invocation of the real-time callback that owns this
    /// consumer, for a future C ABI boundary to call once per callback.
    pub fn record_callback_invocation(&self) {
        self.inner
            .telemetry
            .callback_count
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records one panic caught and contained at the real-time boundary,
    /// for a future C ABI boundary to call from its `catch_unwind` guard.
    pub fn record_contained_panic(&self) {
        self.inner
            .telemetry
            .contained_panic_count
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Total frame capacity of the ring.
    #[must_use]
    pub fn capacity_frames(&self) -> usize {
        self.inner.capacity_frames
    }

    /// Frames currently available to read.
    #[must_use]
    pub fn available_frames(&self) -> usize {
        let write_index = self.inner.write_index.load(Ordering::Acquire);
        let read_index = self.inner.read_index.load(Ordering::Relaxed);
        write_index - read_index
    }

    /// Point-in-time read of every telemetry counter.
    #[must_use]
    pub fn telemetry(&self) -> RenderRingSnapshot {
        self.inner.telemetry.snapshot()
    }
}

fn u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
