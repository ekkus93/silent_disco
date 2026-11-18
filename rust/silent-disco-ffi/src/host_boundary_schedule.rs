//! The host's pending track boundaries, for the Android stream loop.
//!
//! A thin, thread-safe wrapper over the core's `HostBoundarySchedule`, the
//! same rule the desktop pump uses. Kotlin keeps the clock and the timer that
//! wakes it; it asks this object which boundaries are due and never computes
//! a boundary deadline itself.

use std::sync::{Arc, Mutex, MutexGuard};

use silent_disco_core::audio::{HostBoundarySchedule, PendingHostBoundary};

use crate::host_playlist::FfiHostPlaylistError;

/// A boundary the host has sent and not yet applied to its queue.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiHostBoundary {
    pub entry_id: String,
    pub first_sample_index: u64,
    pub display_name: String,
}

impl From<PendingHostBoundary> for FfiHostBoundary {
    fn from(boundary: PendingHostBoundary) -> Self {
        Self {
            entry_id: boundary.entry_id,
            first_sample_index: boundary.first_sample_index,
            display_name: boundary.display_name,
        }
    }
}

#[derive(Debug, uniffi::Object)]
pub struct FfiHostBoundarySchedule {
    inner: Mutex<HostBoundarySchedule>,
}

impl FfiHostBoundarySchedule {
    fn lock(&self) -> Result<MutexGuard<'_, HostBoundarySchedule>, FfiHostPlaylistError> {
        self.inner.lock().map_err(|_| {
            FfiHostPlaylistError::Closed("host boundary schedule is poisoned".to_owned())
        })
    }
}

#[allow(clippy::missing_errors_doc)] // Every error is a poisoned lock or a refused boundary.
#[uniffi::export]
impl FfiHostBoundarySchedule {
    /// A schedule for a stream that started at `host_start_time_ms` on the
    /// host clock, before any pause offset.
    #[uniffi::constructor]
    pub fn new(
        host_start_time_ms: u64,
        sample_rate: u32,
    ) -> Result<Arc<Self>, FfiHostPlaylistError> {
        let schedule = HostBoundarySchedule::new(host_start_time_ms, sample_rate)
            .map_err(|error| FfiHostPlaylistError::InvalidConfiguration(error.message))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(schedule),
        }))
    }

    /// Adds a boundary the host has just sent. Refuses one that does not come
    /// after every boundary already scheduled, or one past the bound.
    pub fn schedule(
        &self,
        entry_id: String,
        first_sample_index: u64,
        display_name: String,
    ) -> Result<(), FfiHostPlaylistError> {
        self.lock()?
            .schedule(PendingHostBoundary {
                entry_id,
                first_sample_index,
                display_name,
            })
            .map_err(|error| FfiHostPlaylistError::InvalidConfiguration(error.message))
    }

    /// Removes and returns, in order, every boundary due at `now_ms` given the
    /// total pause offset the stream loop has applied to the audio.
    pub fn take_due(
        &self,
        now_ms: u64,
        pause_offset_ms: u64,
    ) -> Result<Vec<FfiHostBoundary>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .take_due(now_ms, pause_offset_ms)
            .into_iter()
            .map(FfiHostBoundary::from)
            .collect())
    }

    /// When the earliest pending boundary is due, if any.
    pub fn next_due_at_ms(
        &self,
        pause_offset_ms: u64,
    ) -> Result<Option<u64>, FfiHostPlaylistError> {
        Ok(self.lock()?.next_due_at_ms(pause_offset_ms))
    }

    /// Removes and returns the boundaries at or after the first sample no
    /// listener was sent.
    pub fn withdraw_from(
        &self,
        first_unsent_sample_index: u64,
    ) -> Result<Vec<FfiHostBoundary>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .withdraw_from(first_unsent_sample_index)
            .into_iter()
            .map(FfiHostBoundary::from)
            .collect())
    }

    /// The boundaries still waiting, in order.
    pub fn pending(&self) -> Result<Vec<FfiHostBoundary>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .pending()
            .cloned()
            .map(FfiHostBoundary::from)
            .collect())
    }

    /// Drops every pending boundary.
    pub fn clear(&self) -> Result<(), FfiHostPlaylistError> {
        self.lock()?.clear();
        Ok(())
    }
}

#[cfg(test)]
#[path = "host_boundary_schedule_tests.rs"]
mod tests;
