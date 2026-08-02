//! UniFFI-facing control plane for one Rust-owned render ring.
//!
//! This is ordinary, non-real-time control-plane surface: Kotlin calls
//! [`FfiAudioOutputHandle::open`] once per stream start, feeds already
//! decoded and scheduled interleaved float32 stereo frames via
//! [`FfiAudioOutputHandle::push_frames`] from its own scheduling thread (not
//! the real-time audio callback), and hands [`FfiAudioOutputHandle::engine_token`]
//! down to native audio setup code, which uses it only with the narrow C ABI
//! in `include/silent_disco_audio.h` from then on. This handle never touches
//! the real-time read path itself.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use silent_disco_core::audio::{RenderRingConfig, RenderRingProducer};

use crate::audio_abi::{AudioAbiError, register_render_ring, release_render_ring};

/// Explicit, distinguishable failure exposed to the Android binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiAudioOutputError {
    /// The requested ring capacity/target-fill configuration was rejected.
    InvalidConfiguration(String),
    /// The handle was already closed.
    Closed(String),
}

impl fmt::Display for FfiAudioOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) | Self::Closed(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for FfiAudioOutputError {}

impl From<AudioAbiError> for FfiAudioOutputError {
    fn from(error: AudioAbiError) -> Self {
        Self::InvalidConfiguration(format!("{error:?}"))
    }
}

struct Inner {
    producer: RenderRingProducer,
    token: u64,
}

/// Opaque handle owning one render ring's producer half and the token its
/// consumer half was registered under.
///
/// Kotlin holds this for the lifetime of one output stream: it pushes
/// already-decided PCM frames through it and passes [`Self::engine_token`]
/// to native audio setup code exactly once, at stream start.
#[derive(uniffi::Object)]
pub struct FfiAudioOutputHandle {
    inner: Mutex<Option<Inner>>,
}

impl fmt::Debug for FfiAudioOutputHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FfiAudioOutputHandle")
    }
}

#[uniffi::export]
impl FfiAudioOutputHandle {
    /// Opens a fresh render ring with the given capacity and startup target
    /// (both in frames) and registers it with the real-time C ABI's engine
    /// registry.
    ///
    /// # Errors
    ///
    /// Returns [`FfiAudioOutputError::InvalidConfiguration`] when the
    /// requested capacity/target-fill bounds are rejected.
    #[uniffi::constructor]
    pub fn open(
        capacity_frames: u32,
        target_fill_frames: u32,
    ) -> Result<Arc<Self>, FfiAudioOutputError> {
        let config = RenderRingConfig {
            capacity_frames: usize::try_from(capacity_frames).unwrap_or(usize::MAX),
            target_fill_frames: usize::try_from(target_fill_frames).unwrap_or(usize::MAX),
        };
        let (producer, token) = register_render_ring(config)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(Inner { producer, token })),
        }))
    }

    /// Pushes interleaved float32 stereo frames into the render ring,
    /// returning the count actually written (see
    /// [`silent_disco_core::audio::RenderRingProducer::push_frames`]).
    ///
    /// Called from Kotlin's own decode/scheduling thread, never from the
    /// real-time audio callback.
    ///
    /// # Errors
    ///
    /// Returns [`FfiAudioOutputError::Closed`] if [`Self::release`] was
    /// already called.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned Vec parameters at the foreign boundary"
    )]
    pub fn push_frames(&self, samples: Vec<f32>) -> Result<u32, FfiAudioOutputError> {
        self.with_inner(|inner| {
            Ok(u32::try_from(inner.producer.push_frames(&samples)).unwrap_or(u32::MAX))
        })
    }

    /// The opaque token to hand to native audio setup code exactly once, at
    /// stream start; never dereferenced by Rust (see
    /// `include/silent_disco_audio.h`).
    ///
    /// # Errors
    ///
    /// Returns [`FfiAudioOutputError::Closed`] if [`Self::release`] was
    /// already called.
    pub fn engine_token(&self) -> Result<i64, FfiAudioOutputError> {
        self.with_inner(|inner| Ok(i64::try_from(inner.token).unwrap_or(i64::MAX)))
    }

    /// Frames currently queued in the ring: written by the producer but not
    /// yet consumed by the real-time reader. Lets the Kotlin pacing loop hold
    /// the ring at an intentional depth (its jitter cushion) instead of
    /// either writing at presentation time (near-zero depth, underrun-fragile)
    /// or relying on full-ring backpressure (pinned depth, maximal latency,
    /// constant write stalls).
    ///
    /// # Errors
    ///
    /// Returns [`FfiAudioOutputError::Closed`] if [`Self::release`] was
    /// already called.
    pub fn queued_frames(&self) -> Result<u32, FfiAudioOutputError> {
        self.with_inner(|inner| {
            let queued = inner
                .producer
                .capacity_frames()
                .saturating_sub(inner.producer.free_frames());
            Ok(u32::try_from(queued).unwrap_or(u32::MAX))
        })
    }

    /// Explicitly releases this render ring's consumer registration. Future
    /// real-time reads for this token report a stopping state and silence
    /// rather than being treated as unknown. Idempotent: closing an
    /// already-closed handle is a no-op, not an error.
    pub fn release(&self) {
        let mut guard = self.lock();
        if let Some(inner) = guard.take() {
            // A double release is only reachable if this same token were
            // somehow registered twice, which the registry itself prevents;
            // an error here would only ever indicate an internal bug, not a
            // caller mistake, so it is not surfaced as a distinct failure.
            let _ = release_render_ring(inner.token);
        }
    }
}

impl FfiAudioOutputHandle {
    fn lock(&self) -> MutexGuard<'_, Option<Inner>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn with_inner<T>(
        &self,
        action: impl FnOnce(&mut Inner) -> Result<T, FfiAudioOutputError>,
    ) -> Result<T, FfiAudioOutputError> {
        let mut guard = self.lock();
        let inner = guard.as_mut().ok_or_else(|| {
            FfiAudioOutputError::Closed("audio output handle is closed".to_owned())
        })?;
        action(inner)
    }
}

impl Drop for FfiAudioOutputHandle {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    // Both comparisons below check an exact silence fill (zeroed slots), not
    // approximate arithmetic — exact equality is the correct check here.
    #![allow(clippy::float_cmp)]

    use super::{FfiAudioOutputError, FfiAudioOutputHandle};
    use crate::audio_abi::registry_test_guard;

    #[test]
    fn open_push_and_read_token_round_trip() {
        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("open handle has a token");
        assert!(token > 0);

        let written = handle
            .push_frames(vec![1.0, 2.0, 3.0, 4.0])
            .expect("open handle accepts frames");
        assert_eq!(written, 2);
    }

    #[test]
    fn queued_frames_tracks_pushed_but_unconsumed_frames() {
        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        assert_eq!(handle.queued_frames().expect("open handle"), 0);

        handle
            .push_frames(vec![1.0, 2.0, 3.0, 4.0])
            .expect("open handle accepts frames");
        assert_eq!(handle.queued_frames().expect("open handle"), 2);

        handle.release();
        assert_eq!(
            handle.queued_frames(),
            Err(FfiAudioOutputError::Closed(
                "audio output handle is closed".to_owned()
            ))
        );
    }

    #[test]
    fn release_is_idempotent_and_subsequent_calls_report_closed() {
        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        handle.release();
        handle.release();

        assert_eq!(
            handle.push_frames(vec![0.0, 0.0]),
            Err(FfiAudioOutputError::Closed(
                "audio output handle is closed".to_owned()
            ))
        );
        assert_eq!(
            handle.engine_token(),
            Err(FfiAudioOutputError::Closed(
                "audio output handle is closed".to_owned()
            ))
        );
    }

    #[test]
    fn rejects_an_invalid_configuration() {
        let _guard = registry_test_guard();
        let error = FfiAudioOutputHandle::open(0, 0).expect_err("zero capacity must be rejected");
        assert!(matches!(
            error,
            FfiAudioOutputError::InvalidConfiguration(_)
        ));
    }

    #[test]
    fn dropping_the_handle_releases_its_token() {
        use crate::audio_abi::silent_disco_audio_read_interleaved_f32;

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("open handle has a token");
        drop(handle);

        let engine = usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void;
        let mut output = [1.0_f32; 2];
        let mut frames_from_ring = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            engine,
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );

        assert_eq!(
            status, 2, /* STOPPING */
            "the dropped handle's token must report stopping, not be silently still active"
        );
        assert_eq!(output, [0.0, 0.0]);
    }
}
