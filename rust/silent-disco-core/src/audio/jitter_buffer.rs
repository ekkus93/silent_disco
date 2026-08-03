use core::fmt;
use std::collections::BTreeMap;
use std::error::Error;

use crate::domain::{SessionId, StreamId};
use crate::protocol::AudioDatagram;

/// Default bound on how many packets ahead of the next-expected sequence the
/// buffer accepts before rejecting a too-far-future arrival as unreorderable.
pub const DEFAULT_MAX_REORDER_WINDOW: u32 = 64;
/// Hard ceiling on [`JitterBufferConfig::max_reorder_window`].
pub const MAX_REORDER_WINDOW_LIMIT: u32 = 4_096;
/// Default bound on the total buffered presentation-time span, in milliseconds.
pub const DEFAULT_MAX_BUFFERED_DURATION_MS: u64 = 2_000;
/// Hard ceiling on [`JitterBufferConfig::max_buffered_duration_ms`].
pub const MAX_BUFFERED_DURATION_LIMIT_MS: u64 = 60_000;
/// Consecutive far-future arrivals required before the buffer accepts that
/// the stream has moved beyond its reorder window and adopts the new
/// position. One stray sequence must never be able to strand playback.
const FAR_FUTURE_RESYNC_PACKETS: u32 = 3;

/// Fixed identity and bounds for one jitter buffer instance, covering exactly
/// one host stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitterBufferConfig {
    /// Session this buffer accepts packets for; every other session is rejected.
    pub session_id: SessionId,
    /// Stream this buffer accepts packets for; every other stream is rejected
    /// as stale (an earlier or unrelated stream generation).
    pub stream_id: StreamId,
    /// Maximum sequence distance ahead of the next-expected sequence that a
    /// packet may occupy before it is rejected as unreorderable.
    pub max_reorder_window: u32,
    /// Maximum presentation-time span, across every currently buffered
    /// packet, that this buffer will hold before rejecting further arrivals.
    pub max_buffered_duration_ms: u64,
}

impl JitterBufferConfig {
    /// Convenience constructor using the recommended default bounds.
    #[must_use]
    pub const fn new(session_id: SessionId, stream_id: StreamId) -> Self {
        Self {
            session_id,
            stream_id,
            max_reorder_window: DEFAULT_MAX_REORDER_WINDOW,
            max_buffered_duration_ms: DEFAULT_MAX_BUFFERED_DURATION_MS,
        }
    }
}

/// Stable failure taxonomy for jitter buffer configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterBufferConfigErrorKind {
    /// `max_reorder_window` exceeds [`MAX_REORDER_WINDOW_LIMIT`].
    ReorderWindowTooLarge,
    /// `max_buffered_duration_ms` is zero or exceeds [`MAX_BUFFERED_DURATION_LIMIT_MS`].
    BufferedDurationOutOfRange,
}

/// Typed jitter buffer configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitterBufferConfigError {
    /// Stable semantic failure category.
    pub kind: JitterBufferConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for JitterBufferConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for JitterBufferConfigError {}

/// Stable rejection taxonomy for one arriving packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterBufferRejectionKind {
    /// The packet's `session_id` does not match this buffer's configured session.
    WrongSession,
    /// The packet's `stream_id` does not match this buffer's configured
    /// stream; covers packets from an earlier or unrelated stream generation.
    WrongStream,
    /// A packet with this exact sequence is already buffered, awaiting emission.
    Duplicate,
    /// The packet's sequence is at or before the next sequence already
    /// emitted; it arrived too late to ever be delivered in order.
    AlreadyEmitted,
    /// The packet's sequence is farther ahead of the next-expected sequence
    /// than [`JitterBufferConfig::max_reorder_window`] permits.
    ReorderWindowExceeded,
    /// Accepting the packet would make the buffered presentation-time span
    /// exceed [`JitterBufferConfig::max_buffered_duration_ms`].
    BufferedDurationExceeded,
}

/// Typed rejection of one arriving packet, with a user-safe diagnostic message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitterBufferRejection {
    /// Stable semantic rejection category.
    pub kind: JitterBufferRejectionKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for JitterBufferRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for JitterBufferRejection {}

impl JitterBufferRejection {
    fn new(kind: JitterBufferRejectionKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Cumulative counters describing everything this buffer has observed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitterBufferStatistics {
    /// Packets accepted into the buffer.
    pub accepted: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::WrongSession`].
    pub wrong_session_rejections: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::WrongStream`].
    pub wrong_stream_rejections: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::Duplicate`].
    pub duplicate_rejections: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::AlreadyEmitted`].
    pub late_rejections: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::ReorderWindowExceeded`].
    pub reorder_window_rejections: u64,
    /// Packets rejected as [`JitterBufferRejectionKind::BufferedDurationExceeded`].
    pub buffered_duration_rejections: u64,
    /// Packets emitted in order via [`JitterBuffer::pop_in_order`].
    pub emitted: u64,
    /// Sequences the caller gave up waiting for and advanced past without
    /// emitting, via [`JitterBuffer::skip_expected_sequence`] or
    /// [`JitterBuffer::skip_to_earliest_buffered`].
    pub skipped: u64,
    /// Times the buffer adopted a far-ahead sequence because the stream had
    /// moved beyond the reorder window entirely.
    pub resynchronisations: u64,
}

/// Bounded, ordered holding area for validated protocol audio packets
/// belonging to exactly one host stream.
///
/// The jitter buffer reorders packets by sequence number and tracks missing
/// sequences, but does not itself decide when to conceal a gap or how to pace
/// emission against a presentation clock; that is layered on top by the
/// concealment policy and playback scheduler.
#[derive(Debug)]
pub struct JitterBuffer {
    config: JitterBufferConfig,
    packets: BTreeMap<u64, AudioDatagram>,
    next_expected_sequence: u64,
    statistics: JitterBufferStatistics,
    /// Consecutive far-future arrivals with nothing accepted in between.
    far_future_run: u32,
    /// Lowest sequence seen during the current far-future run; adopting the
    /// lowest keeps an outlier from dragging the stream further than the
    /// real stream position.
    lowest_far_future: Option<u64>,
}

impl JitterBuffer {
    /// Creates a jitter buffer for one stream, validating the configured
    /// bounds against this buffer's hard safety ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`JitterBufferConfigErrorKind::ReorderWindowTooLarge`] or
    /// [`JitterBufferConfigErrorKind::BufferedDurationOutOfRange`] when the
    /// configured bounds exceed what this buffer will ever hold.
    pub fn new(config: JitterBufferConfig) -> Result<Self, JitterBufferConfigError> {
        if config.max_reorder_window > MAX_REORDER_WINDOW_LIMIT {
            return Err(JitterBufferConfigError {
                kind: JitterBufferConfigErrorKind::ReorderWindowTooLarge,
                message: format!(
                    "reorder window of {} packets exceeds the {MAX_REORDER_WINDOW_LIMIT}-packet limit",
                    config.max_reorder_window
                ),
            });
        }
        if config.max_buffered_duration_ms == 0
            || config.max_buffered_duration_ms > MAX_BUFFERED_DURATION_LIMIT_MS
        {
            return Err(JitterBufferConfigError {
                kind: JitterBufferConfigErrorKind::BufferedDurationOutOfRange,
                message: format!(
                    "buffered duration of {}ms must be nonzero and within the {MAX_BUFFERED_DURATION_LIMIT_MS}ms limit",
                    config.max_buffered_duration_ms
                ),
            });
        }
        Ok(Self {
            config,
            packets: BTreeMap::new(),
            next_expected_sequence: 0,
            statistics: JitterBufferStatistics::default(),
            far_future_run: 0,
            lowest_far_future: None,
        })
    }

    /// Validates and inserts one arriving packet, or reports why it was
    /// rejected. Rejection never panics and never affects previously
    /// buffered packets.
    ///
    /// # Errors
    ///
    /// Returns a [`JitterBufferRejection`] for a wrong session, a wrong
    /// (stale) stream, a duplicate of an already-buffered sequence, a
    /// sequence already emitted, a sequence beyond the configured reorder
    /// window, or an insertion that would exceed the configured buffered
    /// duration.
    pub fn accept(&mut self, datagram: AudioDatagram) -> Result<(), JitterBufferRejection> {
        if datagram.session_id != self.config.session_id {
            self.statistics.wrong_session_rejections += 1;
            return Err(JitterBufferRejection::new(
                JitterBufferRejectionKind::WrongSession,
                "packet session does not match this jitter buffer's configured session",
            ));
        }
        if datagram.stream_id != self.config.stream_id {
            self.statistics.wrong_stream_rejections += 1;
            return Err(JitterBufferRejection::new(
                JitterBufferRejectionKind::WrongStream,
                "packet stream does not match this jitter buffer's configured stream",
            ));
        }

        let sequence = datagram.sequence.get();

        if sequence < self.next_expected_sequence {
            self.statistics.late_rejections += 1;
            return Err(JitterBufferRejection::new(
                JitterBufferRejectionKind::AlreadyEmitted,
                "packet sequence was already emitted in order",
            ));
        }
        if self.packets.contains_key(&sequence) {
            self.statistics.duplicate_rejections += 1;
            return Err(JitterBufferRejection::new(
                JitterBufferRejectionKind::Duplicate,
                "packet sequence is already buffered awaiting emission",
            ));
        }
        if sequence - self.next_expected_sequence > u64::from(self.config.max_reorder_window) {
            // A packet this far ahead is not a reordering problem: the stream
            // has moved on without us, because the listener was starved past
            // the window or joined a stream already in progress. Rejecting it
            // is unrecoverable — nothing else advances the expected sequence,
            // so every later packet is rejected too and the stream is silent
            // until teardown. Adopt the new position instead, and let the
            // caller re-accumulate its startup buffer from there.
            self.statistics.reorder_window_rejections += 1;
            self.far_future_run = self.far_future_run.saturating_add(1);
            self.lowest_far_future = Some(
                self.lowest_far_future
                    .map_or(sequence, |lowest| lowest.min(sequence)),
            );
            // One stray sequence proves nothing and must not be able to move
            // the stream — a single corrupt or hostile packet would otherwise
            // strand playback forever. A genuinely advanced stream corroborates
            // itself within a few packets.
            let corroborated = self.far_future_run >= FAR_FUTURE_RESYNC_PACKETS;
            if !(corroborated && self.packets.is_empty()) {
                return Err(JitterBufferRejection::new(
                    JitterBufferRejectionKind::ReorderWindowExceeded,
                    "packet sequence is too far ahead of the next-expected sequence to reorder",
                ));
            }
            self.statistics.resynchronisations += 1;
            self.next_expected_sequence = self.lowest_far_future.unwrap_or(sequence);
            self.far_future_run = 0;
            self.lowest_far_future = None;
        }
        if let Some(earliest) = self.packets.values().next() {
            let earliest_time_ms = earliest.host_presentation_time_ms.get();
            let candidate_time_ms = datagram.host_presentation_time_ms.get();
            let span_ms = earliest_time_ms.abs_diff(candidate_time_ms);
            if span_ms > self.config.max_buffered_duration_ms {
                self.statistics.buffered_duration_rejections += 1;
                return Err(JitterBufferRejection::new(
                    JitterBufferRejectionKind::BufferedDurationExceeded,
                    "accepting this packet would exceed the maximum buffered duration",
                ));
            }
        }

        self.far_future_run = 0;
        self.lowest_far_future = None;
        self.packets.insert(sequence, datagram);
        self.statistics.accepted += 1;
        Ok(())
    }

    /// Removes and returns the next packet only if it is exactly the
    /// next-expected sequence; returns `None` when that sequence has not yet
    /// arrived, without skipping ahead.
    pub fn pop_in_order(&mut self) -> Option<AudioDatagram> {
        let datagram = self.packets.remove(&self.next_expected_sequence)?;
        self.next_expected_sequence += 1;
        self.statistics.emitted += 1;
        Some(datagram)
    }

    /// Forcibly advances past the next-expected sequence without emitting a
    /// packet, for a caller (concealment policy or scheduler) that has
    /// decided to stop waiting for it. Any later arrival for the skipped
    /// sequence is rejected as already-emitted.
    pub fn skip_expected_sequence(&mut self) {
        self.next_expected_sequence += 1;
        self.statistics.skipped += 1;
    }

    /// Forcibly advances to the earliest currently buffered sequence,
    /// abandoning every missing sequence before it, and returns how many were
    /// abandoned. Used when a gap is too wide to be worth covering packet by
    /// packet: synthesizing a frame per missing slot would queue the whole
    /// outage ahead of the audio that actually arrived, dragging playback
    /// behind its deadlines for the rest of the stream.
    ///
    /// Returns zero and changes nothing when the buffer is empty or the
    /// next-expected sequence is already buffered.
    pub fn skip_to_earliest_buffered(&mut self) -> u64 {
        let skipped = self.missing_sequence_count();
        if skipped > 0 {
            self.next_expected_sequence += skipped;
            self.statistics.skipped += skipped;
        }
        skipped
    }

    /// Removes and returns every buffered packet in sequence order, ignoring
    /// presentation deadlines, and advances past the last of them.
    ///
    /// For a stream that is stopping: everything still buffered already
    /// arrived over the network in time, so it is real content rather than
    /// backlog and discarding it would silently truncate the stream's tail.
    /// Gaps within the drained range are the caller's to handle — the
    /// sequence numbers are preserved so they remain visible.
    pub fn drain_all(&mut self) -> Vec<AudioDatagram> {
        let drained: Vec<AudioDatagram> =
            core::mem::take(&mut self.packets).into_values().collect();
        if let Some(last) = drained.last() {
            self.next_expected_sequence = last.sequence.get() + 1;
        }
        self.statistics.emitted += u64::try_from(drained.len()).unwrap_or(u64::MAX);
        drained
    }

    /// Number of sequences between the next-expected sequence and the
    /// earliest currently buffered packet, i.e. packets that have not
    /// arrived yet but are already known to be missing from the gap.
    #[must_use]
    pub fn missing_sequence_count(&self) -> u64 {
        match self.packets.keys().next() {
            Some(&lowest) if lowest > self.next_expected_sequence => {
                lowest - self.next_expected_sequence
            }
            _ => 0,
        }
    }

    /// Presentation-time span, in milliseconds, between the earliest and
    /// latest currently buffered packets. Zero when fewer than two packets
    /// are buffered.
    #[must_use]
    pub fn buffered_span_ms(&self) -> u64 {
        let mut times = self
            .packets
            .values()
            .map(|datagram| datagram.host_presentation_time_ms.get());
        let Some(first) = times.next() else {
            return 0;
        };
        let (min, max) = times.fold((first, first), |(min, max), time| {
            (min.min(time), max.max(time))
        });
        max - min
    }

    /// The sequence this buffer will emit next via [`Self::pop_in_order`].
    #[must_use]
    pub const fn next_expected_sequence(&self) -> u64 {
        self.next_expected_sequence
    }

    /// Number of packets currently buffered, awaiting emission.
    #[must_use]
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    /// True when no packets are currently buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Cumulative counters describing everything this buffer has observed.
    #[must_use]
    pub const fn statistics(&self) -> JitterBufferStatistics {
        self.statistics
    }
}
