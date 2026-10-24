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
    /// already called. Rejects incomplete stereo frames and non-finite PCM
    /// before mutating the native ring; the low-level ring intentionally
    /// ignores trailing half-frames, which must not be silent at this API.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned Vec parameters at the foreign boundary"
    )]
    pub fn push_frames(&self, samples: Vec<f32>) -> Result<u32, FfiAudioOutputError> {
        self.with_inner(|inner| {
            if !samples.len().is_multiple_of(2) {
                return Err(FfiAudioOutputError::InvalidConfiguration(
                    "native output PCM must contain complete interleaved stereo frames".to_owned(),
                ));
            }
            if samples.iter().any(|sample| !sample.is_finite()) {
                return Err(FfiAudioOutputError::InvalidConfiguration(
                    "native output PCM must contain only finite samples".to_owned(),
                ));
            }
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
    fn rejects_incomplete_stereo_frames_without_poisoning_next_packet() {
        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        assert!(matches!(
            handle.push_frames(vec![0.25, -0.25, 0.5]),
            Err(FfiAudioOutputError::InvalidConfiguration(message))
                if message.contains("complete interleaved stereo frames")
        ));
        assert_eq!(handle.queued_frames().expect("unchanged output"), 0);
        assert_eq!(
            handle
                .push_frames(vec![0.25, -0.25])
                .expect("valid recovery"),
            1
        );
        assert_eq!(handle.queued_frames().expect("valid frame retained"), 1);
    }

    #[test]
    fn rejects_nonfinite_pcm_atomically_and_recovers() {
        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                handle.push_frames(vec![0.25, -0.25, invalid, 0.5]),
                Err(FfiAudioOutputError::InvalidConfiguration(message))
                    if message.contains("only finite samples")
            ));
            assert_eq!(handle.queued_frames().expect("no partial write"), 0);
        }
        assert_eq!(
            handle.push_frames(vec![0.5, -0.5]).expect("valid recovery"),
            1
        );
        assert_eq!(handle.queued_frames().expect("recovered output"), 1);
    }

    #[test]
    fn malformed_pcm_preserves_prebuffered_output_and_callback_order() {
        use crate::audio_abi::silent_disco_audio_read_interleaved_f32;

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("live output token");
        let initial = [0.125_f32, -0.25, 0.5, -0.75];
        assert_eq!(handle.push_frames(initial.to_vec()).expect("prebuffer"), 2);

        // Rejected writes must not advance the producer cursor or overwrite
        // valid PCM already queued for the real-time callback.
        for invalid in [
            vec![0.25, -0.25, 0.5],
            vec![0.25, -0.25, f32::NAN, 0.5],
            vec![0.25, -0.25, f32::INFINITY, 0.5],
            vec![0.25, -0.25, f32::NEG_INFINITY, 0.5],
        ] {
            assert!(matches!(
                handle.push_frames(invalid),
                Err(FfiAudioOutputError::InvalidConfiguration(_))
            ));
            assert_eq!(handle.queued_frames().expect("buffer preserved"), 2);
        }

        let engine = usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void;
        let mut callback_pcm = [-9.0_f32; 4];
        let mut supplied = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            engine,
            callback_pcm.as_mut_ptr(),
            2,
            2,
            &raw mut supplied,
        );
        assert_eq!(status, 0, "prebuffered frames must render without silence");
        assert_eq!(supplied, 2);
        assert_eq!(callback_pcm, initial);
        assert_eq!(handle.queued_frames().expect("drained"), 0);

        // A valid packet following rejected writes must still render in order.
        assert_eq!(handle.push_frames(vec![0.75, -0.875]).expect("recovery"), 1);
        let mut resumed_pcm = [-9.0_f32; 2];
        let status = silent_disco_audio_read_interleaved_f32(
            engine,
            resumed_pcm.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        );
        assert_eq!(status, 0);
        assert_eq!(supplied, 1);
        assert_eq!(resumed_pcm, [0.75, -0.875]);
        assert_eq!(handle.queued_frames().expect("fully drained"), 0);
    }

    #[test]
    fn full_ring_backpressure_retries_without_reordering_native_callback_pcm() {
        use crate::audio_abi::{
            silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
            silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
        };

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("live output token");
        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
        let first = [0.25_f32, -0.25].repeat(4_800);
        let deferred = vec![0.75_f32, -0.75, 0.875, -0.875];
        assert_eq!(handle.push_frames(first.clone()).expect("fill ring"), 4_800);
        assert_eq!(handle.queued_frames().expect("full ring"), 4_800);
        assert_eq!(
            handle.push_frames(deferred.clone()).expect("backpressure"),
            0
        );
        assert_eq!(handle.queued_frames().expect("unchanged full ring"), 4_800);

        let mut first_callback = vec![f32::NAN; 2_400 * 2];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                first_callback.as_mut_ptr(),
                2_400,
                2,
                &raw mut supplied,
            ),
            0,
        );
        assert_eq!(supplied, 2_400);
        assert_eq!(first_callback, first[..4_800]);

        assert_eq!(handle.push_frames(deferred.clone()).expect("retry"), 2);
        let mut second_callback = vec![f32::NAN; 2_402 * 2];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                second_callback.as_mut_ptr(),
                2_402,
                2,
                &raw mut supplied,
            ),
            0,
        );
        assert_eq!(supplied, 2_402);
        assert_eq!(second_callback[..4_800], first[4_800..]);
        assert_eq!(second_callback[4_800..], deferred);
        assert_eq!(handle.queued_frames().expect("drained"), 0);
        assert_eq!(handle.engine_token().expect("same stream"), token);
        assert_eq!(silent_disco_audio_callback_count(engine), 2);
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    }

    #[test]
    fn partial_backpressure_retries_only_unwritten_frames_across_ring_wrap() {
        use crate::audio_abi::{
            silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
            silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
        };

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("live output token");
        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;

        // Leave exactly one frame free: the first frame of the successor
        // packet is accepted, but the remaining two must be retried later.
        let prefill = [0.125_f32, -0.125].repeat(4_799);
        let successor = [0.25_f32, -0.25, 0.5, -0.5, 0.75, -0.75];
        assert_eq!(handle.push_frames(prefill.clone()).expect("prefill"), 4_799);
        assert_eq!(
            handle
                .push_frames(successor.to_vec())
                .expect("partial push"),
            1
        );
        assert_eq!(handle.queued_frames().expect("full ring"), 4_800);

        let mut first_callback = vec![f32::NAN; 4_799 * 2];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                first_callback.as_mut_ptr(),
                4_799,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 4_799);
        assert_eq!(first_callback, prefill);
        assert_eq!(handle.queued_frames().expect("successor head retained"), 1);

        // Retry only the two unwritten frames, not the whole packet. The
        // callback must cross the physical ring wrap without a gap or repeat.
        assert_eq!(
            handle
                .push_frames(successor[2..].to_vec())
                .expect("retry suffix"),
            2
        );
        let mut second_callback = [f32::NAN; 6];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                second_callback.as_mut_ptr(),
                3,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 3);
        assert_eq!(second_callback, successor);
        assert_eq!(handle.queued_frames().expect("drained"), 0);
        assert_eq!(handle.engine_token().expect("same stream"), token);
        assert_eq!(silent_disco_audio_callback_count(engine), 2);
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    }

    #[test]
    fn partial_callback_underrun_recovers_on_same_native_output_token() {
        use crate::audio_abi::{
            silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
            silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
        };

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = handle.engine_token().expect("live output token");
        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
        assert_eq!(
            handle
                .push_frames(vec![0.125, -0.125, 0.25, -0.25])
                .expect("first packet"),
            2
        );

        let mut starving_callback = [f32::NAN; 6];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                starving_callback.as_mut_ptr(),
                3,
                2,
                &raw mut supplied,
            ),
            1,
        );
        assert_eq!(supplied, 2);
        assert_eq!(starving_callback, [0.125, -0.125, 0.25, -0.25, 0.0, 0.0]);
        assert_eq!(handle.queued_frames().expect("starved ring"), 0);

        assert_eq!(
            handle
                .push_frames(vec![0.5, -0.5, 0.75, -0.75])
                .expect("replenish"),
            2
        );
        let mut recovered_callback = [f32::NAN; 4];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                recovered_callback.as_mut_ptr(),
                2,
                2,
                &raw mut supplied,
            ),
            0,
        );
        assert_eq!(supplied, 2);
        assert_eq!(recovered_callback, [0.5, -0.5, 0.75, -0.75]);
        assert_eq!(handle.engine_token().expect("no output reopen"), token);
        assert_eq!(handle.queued_frames().expect("recovered ring drained"), 0);
        assert_eq!(silent_disco_audio_callback_count(engine), 2);
        assert_eq!(silent_disco_audio_underrun_count(engine), 1);
        assert_eq!(silent_disco_audio_silence_filled_frames(engine), 1);
    }

    #[test]
    fn callback_capture_overflow_does_not_block_audio_and_recovers_after_drain() {
        use std::sync::Arc;

        use crate::audio_abi::{
            attach_callback_capture, detach_callback_capture, silent_disco_audio_callback_count,
            silent_disco_audio_frames_rendered, silent_disco_audio_read_interleaved_f32,
            silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
        };
        use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = u64::try_from(handle.engine_token().expect("live token")).expect("token");
        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
        let tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("tap"));
        attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");

        // Fill the diagnostic tap without losing any native callback audio.
        let first = [0.125_f32, -0.125].repeat(MIN_CAPTURE_FRAMES);
        assert_eq!(handle.push_frames(first.clone()).expect("prefill"), 4_800);
        let mut first_output = vec![f32::NAN; first.len()];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                first_output.as_mut_ptr(),
                4_800,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 4_800);
        assert_eq!(first_output, first);

        // The diagnostic tap is full. Its overflow must be explicit, but it
        // must never backpressure, silence, or reorder the live audio output.
        let dropped_from_tap = [0.25_f32, -0.25, 0.375, -0.375];
        assert_eq!(
            handle
                .push_frames(dropped_from_tap.to_vec())
                .expect("second packet"),
            2
        );
        let mut second_output = [f32::NAN; 4];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                second_output.as_mut_ptr(),
                2,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 2);
        assert_eq!(second_output, dropped_from_tap);
        assert_eq!(tap.counters(), (4_800, 2, 2));
        assert_eq!(tap.drain(MIN_CAPTURE_FRAMES), first);

        // Once the non-real-time capture consumer catches up, later audio
        // must be recorded normally without resurrecting the lost frames.
        let recovered = [0.5_f32, -0.5, 0.75, -0.75];
        assert_eq!(handle.push_frames(recovered.to_vec()).expect("recovery"), 2);
        let mut third_output = [f32::NAN; 4];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                third_output.as_mut_ptr(),
                2,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 2);
        assert_eq!(third_output, recovered);
        assert_eq!(tap.drain(2), recovered);
        assert_eq!(tap.counters(), (4_802, 2, 3));
        assert_eq!(handle.queued_frames().expect("drained ring"), 0);
        assert_eq!(silent_disco_audio_callback_count(engine), 3);
        assert_eq!(silent_disco_audio_frames_rendered(engine), 4_804);
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);

        detach_callback_capture(token);
        handle.release();
    }

    #[test]
    fn diagnostic_capture_detach_and_reattach_preserve_live_callback_pcm() {
        use std::sync::Arc;

        use crate::audio_abi::{
            attach_callback_capture, detach_callback_capture, silent_disco_audio_callback_count,
            silent_disco_audio_frames_rendered, silent_disco_audio_read_interleaved_f32,
            silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
        };
        use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};

        let _guard = registry_test_guard();
        let handle = FfiAudioOutputHandle::open(4_800, 1).expect("valid config");
        let token = u64::try_from(handle.engine_token().expect("live token")).expect("token");
        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
        let first_tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("first tap"));
        let second_tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("second tap"));
        attach_callback_capture(token, Arc::clone(&first_tap)).expect("start capture");

        let packets = [
            [0.125_f32, -0.125, 0.25, -0.25],
            [0.375_f32, -0.375, 0.5, -0.5],
            [0.625_f32, -0.625, 0.75, -0.75],
        ];
        for (index, packet) in packets.iter().enumerate() {
            if index == 1 {
                detach_callback_capture(token);
            } else if index == 2 {
                attach_callback_capture(token, Arc::clone(&second_tap))
                    .expect("restart capture on same output");
            }
            assert_eq!(handle.push_frames(packet.to_vec()).expect("live PCM"), 2);
            let mut callback_pcm = [f32::NAN; 4];
            let mut supplied = u32::MAX;
            assert_eq!(
                silent_disco_audio_read_interleaved_f32(
                    engine,
                    callback_pcm.as_mut_ptr(),
                    2,
                    2,
                    &raw mut supplied,
                ),
                0
            );
            assert_eq!(supplied, 2);
            assert_eq!(
                callback_pcm, *packet,
                "capture changes cannot alter playback"
            );
        }

        assert_eq!(first_tap.drain(4_800), packets[0]);
        assert_eq!(first_tap.counters(), (2, 0, 1));
        assert_eq!(second_tap.drain(4_800), packets[2]);
        assert_eq!(second_tap.counters(), (2, 0, 1));
        assert_eq!(
            handle.engine_token().expect("same output"),
            i64::try_from(token).expect("token")
        );
        assert_eq!(handle.queued_frames().expect("ring drained"), 0);
        assert_eq!(silent_disco_audio_callback_count(engine), 3);
        assert_eq!(silent_disco_audio_frames_rendered(engine), 6);
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);

        // Releasing with capture still attached must invalidate the old
        // callback token without writing artificial silence into the tap.
        handle.release();
        let mut stale_pcm = [f32::NAN; 4];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                stale_pcm.as_mut_ptr(),
                2,
                2,
                &raw mut supplied,
            ),
            2
        );
        assert_eq!(supplied, 0);
        assert_eq!(stale_pcm, [0.0; 4]);
        assert_eq!(second_tap.counters(), (2, 0, 1));
        assert!(second_tap.drain(2).is_empty());

        // Retired-token cleanup must not detach or pollute a fresh replay tap.
        let replay = FfiAudioOutputHandle::open(4_800, 1).expect("replay output");
        let replay_token =
            u64::try_from(replay.engine_token().expect("replay token")).expect("replay token");
        assert_ne!(token, replay_token);
        let replay_tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("replay tap"));
        assert!(attach_callback_capture(token, Arc::clone(&replay_tap)).is_err());
        attach_callback_capture(replay_token, Arc::clone(&replay_tap))
            .expect("attach replay capture");
        detach_callback_capture(token);

        let replay_pcm = [0.875_f32, -0.875];
        assert_eq!(
            replay.push_frames(replay_pcm.to_vec()).expect("replay PCM"),
            1
        );
        let replay_engine =
            usize::try_from(replay_token).expect("replay token") as *mut core::ffi::c_void;
        let mut replay_output = [f32::NAN; 2];
        let mut replay_supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                replay_engine,
                replay_output.as_mut_ptr(),
                1,
                2,
                &raw mut replay_supplied,
            ),
            0
        );
        assert_eq!(replay_supplied, 1);
        assert_eq!(replay_output, replay_pcm);
        assert_eq!(replay_tap.drain(1), replay_pcm);
        assert_eq!(replay_tap.counters(), (1, 0, 1));
        assert_eq!(second_tap.counters(), (2, 0, 1));
        assert_eq!(replay.queued_frames().expect("drained"), 0);
        detach_callback_capture(replay_token);
        replay.release();
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
