//! When the listener shows the next track.
//!
//! The host sends each `TrackBoundary` ahead of time. The listener's audio is
//! already scheduled by the Rust playback runtime; this decides when the
//! listener's *track identity* changes: when the boundary's first sample is
//! presented, mapped from host time onto the listener's monotonic clock with
//! the sync offset (`local = host - offset`). It used to be Kotlin timing state
//! (God-file refactor spec A.4 T1; owner decision 2026-11-18: move it into Rust).
//!
//! The caller passes the listener's clock and the current offset on every
//! call and does not ask while the stream is paused or sync is unknown. A host
//! Pause takes back any boundary presented after the host's pause instant: a
//! replayed paused stream carries its pre-pause anchor, so its upcoming
//! boundary otherwise looks overdue before the Pause arrives, and the host
//! re-sends held boundaries, re-anchored, on resume.

use std::collections::BTreeMap;
use std::fmt;

use super::CANONICAL_SAMPLE_RATE_HZ;

/// Boundaries waiting at once. A host keeps at most 128 pending and replays
/// at most 100 recent ones to a late joiner, so this is not reached in use.
pub const MAX_PENDING_LISTENER_BOUNDARIES: usize = 128;

/// Boundaries remembered after they were presented, so a Pause can take back
/// the ones presented after its instant.
pub const MAX_APPLIED_LISTENER_BOUNDARIES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerBoundary {
    pub first_sample_index: u64,
    pub display_name: String,
}

/// What a host Pause took back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerBoundaryWithdrawal {
    /// How many presented boundaries went back to pending.
    pub count: usize,
    /// The track playing at the pause instant: the last boundary presented
    /// before it, or the track the stream start announced.
    pub track_at_pause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerBoundaryScheduleError {
    pub message: String,
}

impl fmt::Display for ListenerBoundaryScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ListenerBoundaryScheduleError {}

/// The listener's boundaries for the stream it follows.
#[derive(Debug, Clone, Default)]
pub struct ListenerBoundarySchedule {
    /// The stream followed, kept across Stop so a same-stream start after it
    /// counts as a resume.
    stream_id: Option<String>,
    /// The stream's host start time, from the latest stream start; `None`
    /// before the first and after Stop.
    host_start_time_ms: Option<u64>,
    start_track: Option<String>,
    pending: BTreeMap<u64, ListenerBoundary>,
    applied: BTreeMap<u64, ListenerBoundary>,
}

impl ListenerBoundarySchedule {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A boundary's host presentation time, at the canonical stream rate.
    #[must_use]
    pub fn host_time_ms(host_start_time_ms: u64, first_sample_index: u64) -> f64 {
        // Exact for any realistic stream: f64 holds integers to 2^53.
        #[allow(clippy::cast_precision_loss)]
        let start = host_start_time_ms as f64;
        #[allow(clippy::cast_precision_loss)]
        let sample = first_sample_index as f64;
        start + sample * 1_000.0 / f64::from(CANONICAL_SAMPLE_RATE_HZ)
    }

    /// A stream start. Returns whether it continues the stream already
    /// followed (a resume), which keeps its pending boundaries; a different
    /// stream drops them. Either way the presented ones are forgotten: the
    /// start announces the current track.
    pub fn stream_started(
        &mut self,
        stream_id: &str,
        host_start_time_ms: u64,
        start_track: Option<String>,
    ) -> bool {
        let same_stream = self.stream_id.as_deref() == Some(stream_id);
        if !same_stream {
            self.pending.clear();
            self.stream_id = Some(stream_id.to_owned());
        }
        self.host_start_time_ms = Some(host_start_time_ms);
        self.start_track = start_track;
        self.applied.clear();
        same_stream
    }

    /// Stop: nothing more is presented until the next stream start.
    pub fn stream_stopped(&mut self) {
        self.host_start_time_ms = None;
        self.start_track = None;
        self.pending.clear();
        self.applied.clear();
    }

    /// The listener stopped following any stream (it left the session or
    /// switched role): the next stream start is never a resume.
    pub fn forget_stream(&mut self) {
        self.stream_stopped();
        self.stream_id = None;
    }

    /// A boundary from the host. Ignores one for another stream (returns
    /// false). One sent again replaces the waiting copy.
    ///
    /// # Errors
    /// Refuses a new boundary when [`MAX_PENDING_LISTENER_BOUNDARIES`] wait.
    pub fn add(
        &mut self,
        stream_id: &str,
        boundary: ListenerBoundary,
    ) -> Result<bool, ListenerBoundaryScheduleError> {
        if self.stream_id.as_deref() != Some(stream_id) {
            return Ok(false);
        }
        if !self.pending.contains_key(&boundary.first_sample_index)
            && self.pending.len() >= MAX_PENDING_LISTENER_BOUNDARIES
        {
            return Err(ListenerBoundaryScheduleError {
                message: format!(
                    "{MAX_PENDING_LISTENER_BOUNDARIES} track boundaries are already waiting; \
                     refusing the one at sample {}",
                    boundary.first_sample_index
                ),
            });
        }
        self.pending.insert(boundary.first_sample_index, boundary);
        Ok(true)
    }

    /// When a boundary is presented on the listener's clock.
    #[must_use]
    pub fn local_time_ms(&self, first_sample_index: u64, host_offset_ms: f64) -> Option<f64> {
        self.host_start_time_ms
            .map(|start| Self::host_time_ms(start, first_sample_index) - host_offset_ms)
    }

    /// Moves every boundary presented by `now_local_ms` to the presented set
    /// and returns them in sample order.
    pub fn take_due(&mut self, now_local_ms: i64, host_offset_ms: f64) -> Vec<ListenerBoundary> {
        #[allow(clippy::cast_precision_loss)]
        let now = now_local_ms as f64;
        let due_keys: Vec<u64> = self
            .pending
            .keys()
            .copied()
            .take_while(|&sample| {
                self.local_time_ms(sample, host_offset_ms)
                    .is_some_and(|local| local <= now)
            })
            .collect();
        let mut due = Vec::with_capacity(due_keys.len());
        for sample in due_keys {
            if let Some(boundary) = self.pending.remove(&sample) {
                self.applied.insert(sample, boundary.clone());
                due.push(boundary);
            }
        }
        while self.applied.len() > MAX_APPLIED_LISTENER_BOUNDARIES {
            self.applied.pop_first();
        }
        due
    }

    /// When the earliest pending boundary is presented, if any and if the
    /// stream's start is known.
    #[must_use]
    pub fn next_due_local_ms(&self, host_offset_ms: f64) -> Option<f64> {
        let (&sample, _) = self.pending.first_key_value()?;
        self.local_time_ms(sample, host_offset_ms)
    }

    /// A host Pause of `stream_id` at `host_pause_time_ms`: boundaries
    /// presented after that instant go back to pending. `None` when none did
    /// or the pause is for another stream.
    pub fn paused(
        &mut self,
        stream_id: &str,
        host_pause_time_ms: u64,
    ) -> Option<ListenerBoundaryWithdrawal> {
        if self.stream_id.as_deref() != Some(stream_id) {
            return None;
        }
        let start = self.host_start_time_ms?;
        #[allow(clippy::cast_precision_loss)]
        let pause = host_pause_time_ms as f64;
        let withdrawn: Vec<u64> = self
            .applied
            .keys()
            .copied()
            .filter(|&sample| Self::host_time_ms(start, sample) > pause)
            .collect();
        if withdrawn.is_empty() {
            return None;
        }
        for sample in &withdrawn {
            if let Some(boundary) = self.applied.remove(sample) {
                self.pending.insert(*sample, boundary);
            }
        }
        Some(ListenerBoundaryWithdrawal {
            count: withdrawn.len(),
            track_at_pause: self
                .applied
                .values()
                .next_back()
                .map(|boundary| boundary.display_name.clone())
                .or_else(|| self.start_track.clone()),
        })
    }

    #[must_use]
    pub fn stream_id(&self) -> Option<&str> {
        self.stream_id.as_deref()
    }

    #[must_use]
    pub fn host_start_time_ms(&self) -> Option<u64> {
        self.host_start_time_ms
    }

    /// The boundaries waiting, in sample order.
    #[must_use]
    pub fn pending(&self) -> impl ExactSizeIterator<Item = &ListenerBoundary> {
        self.pending.values()
    }

    /// The boundaries presented since the last stream start, in sample order.
    #[must_use]
    pub fn applied(&self) -> impl ExactSizeIterator<Item = &ListenerBoundary> {
        self.applied.values()
    }
}

#[cfg(test)]
#[path = "listener_boundary_schedule_tests.rs"]
mod tests;
