//! When a queued track boundary takes effect on the host.
//!
//! The host sends each `TrackBoundary` to listeners ahead of time, then
//! applies it to its own queue when the boundary's first sample is due on the
//! host timeline. Both hosts (desktop pump, Android stream loop) used to compute
//! that deadline separately; this is now the one implementation (gapless TODO
//! §5 virtual tests, owner decision 2026-11-18: move the Android host's
//! boundary timing into Rust).
//!
//! The deadline is the stream's original host start time, plus the boundary's
//! sample position at the stream rate (rounded down to a millisecond), plus the
//! total pause offset the stream loop has applied to the audio so far. The
//! caller passes the pause offset on every call rather than this type measuring
//! pauses: a boundary must move by exactly what the audio moved. While the
//! stream is paused the caller does not ask for due boundaries.

use std::collections::VecDeque;
use std::fmt;

use super::CANONICAL_SAMPLE_RATE_HZ;

/// Boundaries waiting at once. The producer only reserves a boundary inside
/// its bounded send-ahead window, so this is far above real use; it stops a
/// misbehaving caller from growing the queue without limit.
pub const MAX_PENDING_HOST_BOUNDARIES: usize = 128;

/// A boundary the host has sent and not yet applied to its queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHostBoundary {
    pub entry_id: String,
    pub first_sample_index: u64,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBoundaryScheduleErrorKind {
    /// The stream sample rate was zero.
    InvalidSampleRate,
    /// A boundary did not come after every boundary already scheduled.
    OutOfOrder,
    /// [`MAX_PENDING_HOST_BOUNDARIES`] boundaries are already waiting.
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBoundaryScheduleError {
    pub kind: HostBoundaryScheduleErrorKind,
    pub message: String,
}

impl fmt::Display for HostBoundaryScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HostBoundaryScheduleError {}

/// The host's pending track boundaries for one stream, in sample order.
#[derive(Debug, Clone)]
pub struct HostBoundarySchedule {
    host_start_time_ms: u64,
    sample_rate_hz: u32,
    pending: VecDeque<PendingHostBoundary>,
    /// The latest boundary ever scheduled on this stream, applied or not, so
    /// an old boundary cannot be scheduled again after it was applied.
    last_scheduled_sample_index: Option<u64>,
}

impl HostBoundarySchedule {
    /// A schedule for a stream that started at `host_start_time_ms` on the
    /// host's monotonic clock, before any pause offset.
    ///
    /// # Errors
    /// Returns [`HostBoundaryScheduleErrorKind::InvalidSampleRate`] for a zero
    /// sample rate.
    pub fn new(
        host_start_time_ms: u64,
        sample_rate_hz: u32,
    ) -> Result<Self, HostBoundaryScheduleError> {
        if sample_rate_hz == 0 {
            return Err(HostBoundaryScheduleError {
                kind: HostBoundaryScheduleErrorKind::InvalidSampleRate,
                message: "host boundary schedule needs a nonzero sample rate".to_owned(),
            });
        }
        Ok(Self {
            host_start_time_ms,
            sample_rate_hz,
            pending: VecDeque::new(),
            last_scheduled_sample_index: None,
        })
    }

    /// A schedule at the canonical network rate, which cannot be invalid.
    #[must_use]
    pub fn at_canonical_rate(host_start_time_ms: u64) -> Self {
        const _: () = assert!(CANONICAL_SAMPLE_RATE_HZ > 0);
        Self {
            host_start_time_ms,
            sample_rate_hz: CANONICAL_SAMPLE_RATE_HZ,
            pending: VecDeque::new(),
            last_scheduled_sample_index: None,
        }
    }

    /// When a boundary at `first_sample_index` is due, given the total pause
    /// offset applied to the audio so far. Saturates instead of overflowing.
    #[must_use]
    pub fn due_at_ms(&self, first_sample_index: u64, pause_offset_ms: u64) -> u64 {
        let sample_ms = u128::from(first_sample_index) * 1_000 / u128::from(self.sample_rate_hz);
        let sample_ms = u64::try_from(sample_ms).unwrap_or(u64::MAX);
        self.host_start_time_ms
            .saturating_add(sample_ms)
            .saturating_add(pause_offset_ms)
    }

    /// Adds a boundary the host has just sent.
    ///
    /// # Errors
    /// [`HostBoundaryScheduleErrorKind::OutOfOrder`] unless the boundary comes
    /// after every boundary scheduled on this stream, and
    /// [`HostBoundaryScheduleErrorKind::Full`] when the bound is reached. The
    /// schedule is unchanged on error.
    pub fn schedule(
        &mut self,
        boundary: PendingHostBoundary,
    ) -> Result<(), HostBoundaryScheduleError> {
        if let Some(last) = self.last_scheduled_sample_index
            && boundary.first_sample_index <= last
        {
            return Err(HostBoundaryScheduleError {
                kind: HostBoundaryScheduleErrorKind::OutOfOrder,
                message: format!(
                    "track boundary for {} at sample {} does not follow the boundary already \
                     scheduled at sample {last}",
                    boundary.entry_id, boundary.first_sample_index
                ),
            });
        }
        if self.pending.len() >= MAX_PENDING_HOST_BOUNDARIES {
            return Err(HostBoundaryScheduleError {
                kind: HostBoundaryScheduleErrorKind::Full,
                message: format!(
                    "{MAX_PENDING_HOST_BOUNDARIES} track boundaries are already waiting; \
                     refusing {}",
                    boundary.entry_id
                ),
            });
        }
        self.last_scheduled_sample_index = Some(boundary.first_sample_index);
        self.pending.push_back(boundary);
        Ok(())
    }

    /// Removes and returns, in order, every boundary due at `now_ms`.
    pub fn take_due(&mut self, now_ms: u64, pause_offset_ms: u64) -> Vec<PendingHostBoundary> {
        let mut due = Vec::new();
        while self.pending.front().is_some_and(|boundary| {
            self.due_at_ms(boundary.first_sample_index, pause_offset_ms) <= now_ms
        }) {
            if let Some(boundary) = self.pending.pop_front() {
                due.push(boundary);
            }
        }
        due
    }

    /// When the earliest pending boundary is due, if any.
    #[must_use]
    pub fn next_due_at_ms(&self, pause_offset_ms: u64) -> Option<u64> {
        self.pending
            .front()
            .map(|boundary| self.due_at_ms(boundary.first_sample_index, pause_offset_ms))
    }

    /// Removes and returns the boundaries at or after `first_unsent_sample_index`.
    /// After a failure no listener will hear those tracks, so the host queue
    /// must not advance to them, even though their deadline equals the end of
    /// the audio that was sent.
    pub fn withdraw_from(&mut self, first_unsent_sample_index: u64) -> Vec<PendingHostBoundary> {
        let keep = self
            .pending
            .iter()
            .take_while(|boundary| boundary.first_sample_index < first_unsent_sample_index)
            .count();
        self.pending.split_off(keep).into()
    }

    /// The boundaries still waiting, in order, for replay to listeners after
    /// a resume.
    #[must_use]
    pub fn pending(&self) -> impl ExactSizeIterator<Item = &PendingHostBoundary> {
        self.pending.iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Drops every pending boundary, for Stop and stream teardown.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    #[must_use]
    pub fn host_start_time_ms(&self) -> u64 {
        self.host_start_time_ms
    }

    #[must_use]
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }
}

#[cfg(test)]
#[path = "host_boundary_schedule_tests.rs"]
mod tests;
