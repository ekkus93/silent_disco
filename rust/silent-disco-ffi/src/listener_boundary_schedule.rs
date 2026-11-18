//! The listener's track boundaries, for the Android listener.
//!
//! A thread-safe wrapper over the core's `ListenerBoundarySchedule`. Kotlin
//! keeps the clock and the timer that wakes it and asks this object which
//! boundaries are presented; it never computes a boundary's presentation time.

use std::sync::{Arc, Mutex, MutexGuard};

use silent_disco_core::audio::{ListenerBoundary, ListenerBoundarySchedule};

use crate::host_playlist::FfiHostPlaylistError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiListenerBoundary {
    pub first_sample_index: u64,
    pub display_name: String,
}

impl From<ListenerBoundary> for FfiListenerBoundary {
    fn from(boundary: ListenerBoundary) -> Self {
        Self {
            first_sample_index: boundary.first_sample_index,
            display_name: boundary.display_name,
        }
    }
}

/// What a host Pause took back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiListenerBoundaryWithdrawal {
    pub count: u32,
    pub track_at_pause: Option<String>,
}

#[derive(Debug, uniffi::Object)]
pub struct FfiListenerBoundarySchedule {
    inner: Mutex<ListenerBoundarySchedule>,
}

impl FfiListenerBoundarySchedule {
    fn lock(&self) -> Result<MutexGuard<'_, ListenerBoundarySchedule>, FfiHostPlaylistError> {
        self.inner.lock().map_err(|_| {
            FfiHostPlaylistError::Closed("listener boundary schedule is poisoned".to_owned())
        })
    }
}

#[allow(clippy::missing_errors_doc)] // Every error is a poisoned lock or a refused boundary.
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
#[uniffi::export]
impl FfiListenerBoundarySchedule {
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ListenerBoundarySchedule::new()),
        })
    }

    /// A stream start; returns whether it resumes the stream already followed.
    pub fn stream_started(
        &self,
        stream_id: String,
        host_start_time_ms: u64,
        start_track: Option<String>,
    ) -> Result<bool, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .stream_started(&stream_id, host_start_time_ms, start_track))
    }

    pub fn stream_stopped(&self) -> Result<(), FfiHostPlaylistError> {
        self.lock()?.stream_stopped();
        Ok(())
    }

    /// The listener stopped following any stream.
    pub fn forget_stream(&self) -> Result<(), FfiHostPlaylistError> {
        self.lock()?.forget_stream();
        Ok(())
    }

    /// A boundary from the host; false when it is for another stream.
    pub fn add(
        &self,
        stream_id: String,
        first_sample_index: u64,
        display_name: String,
    ) -> Result<bool, FfiHostPlaylistError> {
        self.lock()?
            .add(
                &stream_id,
                ListenerBoundary {
                    first_sample_index,
                    display_name,
                },
            )
            .map_err(|error| FfiHostPlaylistError::InvalidConfiguration(error.message))
    }

    /// Presents and returns, in order, every boundary due by `now_local_ms`.
    pub fn take_due(
        &self,
        now_local_ms: i64,
        host_offset_ms: f64,
    ) -> Result<Vec<FfiListenerBoundary>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .take_due(now_local_ms, host_offset_ms)
            .into_iter()
            .map(FfiListenerBoundary::from)
            .collect())
    }

    /// When the earliest pending boundary is presented on the local clock.
    pub fn next_due_local_ms(
        &self,
        host_offset_ms: f64,
    ) -> Result<Option<f64>, FfiHostPlaylistError> {
        Ok(self.lock()?.next_due_local_ms(host_offset_ms))
    }

    /// A host Pause; boundaries presented after its instant go back to pending.
    pub fn paused(
        &self,
        stream_id: String,
        host_pause_time_ms: u64,
    ) -> Result<Option<FfiListenerBoundaryWithdrawal>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .paused(&stream_id, host_pause_time_ms)
            .map(|withdrawal| FfiListenerBoundaryWithdrawal {
                count: u32::try_from(withdrawal.count).unwrap_or(u32::MAX),
                track_at_pause: withdrawal.track_at_pause,
            }))
    }

    /// The boundaries waiting, in sample order.
    pub fn pending(&self) -> Result<Vec<FfiListenerBoundary>, FfiHostPlaylistError> {
        Ok(self
            .lock()?
            .pending()
            .cloned()
            .map(FfiListenerBoundary::from)
            .collect())
    }

    /// The stream's host start time; `None` before a start and after Stop.
    pub fn host_start_time_ms(&self) -> Result<Option<u64>, FfiHostPlaylistError> {
        Ok(self.lock()?.host_start_time_ms())
    }
}

#[cfg(test)]
#[path = "listener_boundary_schedule_tests.rs"]
mod tests;
