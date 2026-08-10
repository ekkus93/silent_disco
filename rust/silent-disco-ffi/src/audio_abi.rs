#![allow(unsafe_code)]

//! Narrow, real-time-safe C ABI for reading from a Rust-owned render ring.
//!
//! This is the only Rust module that should ever be called from a native
//! platform audio callback (e.g. Oboe on Android). It must not call `UniFFI`,
//! JNI, `SQLite`, networking, logging, or any other blocking/allocating path.
//!
//! `SilentDiscoAudioEngine*` (declared in
//! `include/silent_disco_audio.h`) is never dereferenced by Rust: it is a
//! plain integer token bit-cast into a pointer-shaped parameter purely to
//! match the C ABI's calling convention. The real registry lookup keyed by
//! that token uses only a `Mutex::try_lock`, never a blocking `lock`, so the
//! real-time functions below never wait on a lock; under the vanishingly
//! rare contention with a concurrent `register`/`release` call, they
//! silence-fill and report [`SilentDiscoAudioStatus::Stopping`] rather than
//! block. The two genuinely `unsafe` operations in this file — turning the
//! caller's raw output pointer into a slice, and writing through the
//! `frames_from_ring` out-pointer — are each a single, minimal, documented
//! line; everything else here, including the token/registry mechanism
//! itself, is ordinary safe Rust.

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock, PoisonError};

use silent_disco_core::audio::{
    RENDER_CHANNELS, RenderRing, RenderRingConfig, RenderRingConfigError, RenderRingConsumer,
    RenderRingProducer,
};

/// ABI contract implemented by this narrow real-time audio C interface.
pub const AUDIO_ABI_VERSION: u16 = 1;

/// Stable status codes matching `SilentDiscoAudioStatus` in
/// `include/silent_disco_audio.h`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SilentDiscoAudioStatus {
    Ok = 0,
    Partial = 1,
    Stopping = 2,
    InvalidArgument = -1,
    InvalidState = -2,
    PanicContained = -3,
}

impl SilentDiscoAudioStatus {
    const fn code(self) -> i32 {
        self as i32
    }
}

enum AudioEngineEntry {
    Active(RenderRingConsumer),
    Released,
    /// Only ever inserted by test code, to verify the panic boundary in
    /// 17.4 without any always-compiled panic path in production code.
    #[cfg(test)]
    PanicOnRead,
}

#[derive(Debug)]
struct AudioEngineRegistry {
    next_token: u64,
    entries: BTreeMap<u64, AudioEngineEntry>,
}

impl Default for AudioEngineRegistry {
    fn default() -> Self {
        Self {
            next_token: 1,
            entries: BTreeMap::new(),
        }
    }
}

impl core::fmt::Debug for AudioEngineEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Active(_) => formatter.write_str("Active"),
            Self::Released => formatter.write_str("Released"),
            #[cfg(test)]
            Self::PanicOnRead => formatter.write_str("PanicOnRead"),
        }
    }
}

static AUDIO_ENGINE_REGISTRY: OnceLock<Mutex<AudioEngineRegistry>> = OnceLock::new();

fn audio_engine_registry() -> &'static Mutex<AudioEngineRegistry> {
    AUDIO_ENGINE_REGISTRY.get_or_init(|| Mutex::new(AudioEngineRegistry::default()))
}

/// Non-blocking registry acquisition for the real-time path. A poisoned
/// mutex (some earlier call panicked while holding the guard, e.g. a
/// contained panic in another engine's read) does not corrupt the registry
/// itself — the panic happens after the map lookup, never mid-mutation — so
/// poison is recovered here rather than treated as unavailable. Only
/// genuine contention (`WouldBlock`) falls back to the caller's default;
/// otherwise a single contained panic would silently degrade every other
/// engine's real-time reads to `Stopping` for the rest of the process.
fn try_lock_registry() -> Option<std::sync::MutexGuard<'static, AudioEngineRegistry>> {
    match audio_engine_registry().try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(poison_error)) => Some(poison_error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Stable failure taxonomy for the audio engine registry's control-plane
/// operations (registration and release; never used by the real-time path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AudioAbiError {
    Config(RenderRingConfigError),
    TokenSpaceExhausted,
    UnknownToken,
    AlreadyReleased,
}

impl From<RenderRingConfigError> for AudioAbiError {
    fn from(error: RenderRingConfigError) -> Self {
        Self::Config(error)
    }
}

/// Creates a render ring, registers its consumer half under a fresh token,
/// and returns the producer half plus that token. Tokens are never reused
/// across the process lifetime, so there is no ABA risk: a token either
/// refers to the engine that was registered under it, refers to that same
/// engine after release (a distinct, explicit released state), or has never
/// existed at all.
///
/// Not part of the real-time C ABI; this is the control-plane entry point a
/// non-real-time caller ([`crate::audio_output::FfiAudioOutputHandle`]) uses
/// once per stream start.
/// Serializes every test that touches the process-global
/// `AUDIO_ENGINE_REGISTRY`. Registration hands out tokens from one shared
/// counter, `token_space_exhaustion_is_a_typed_failure_not_a_panic`
/// deliberately rewrites that counter, and several tests assert on cumulative
/// telemetry that a concurrently registered sibling engine would perturb.
/// Every test in this crate that registers, releases, or reads through a
/// token — including the `audio_output` handle tests, which reach the same
/// registry through [`FfiAudioOutputHandle`](crate::audio_output::FfiAudioOutputHandle)
/// — must hold this guard.
#[cfg(test)]
pub(crate) fn registry_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn register_render_ring(
    config: RenderRingConfig,
) -> Result<(RenderRingProducer, u64), AudioAbiError> {
    let ring = RenderRing::new(config)?;
    let (producer, consumer) = ring.split();

    let mut registry = audio_engine_registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let token = registry.next_token;
    if token == 0 {
        return Err(AudioAbiError::TokenSpaceExhausted);
    }
    registry
        .entries
        .insert(token, AudioEngineEntry::Active(consumer));
    // `checked_add` overflowing means `token` was `u64::MAX`, the last
    // representable token; store the reserved `0` sentinel so every future
    // call fails explicitly instead of wrapping around to a reused value.
    registry.next_token = token.checked_add(1).unwrap_or(0);
    Ok((producer, token))
}

/// Marks a previously registered token as released. Future reads for this
/// token return [`SilentDiscoAudioStatus::Stopping`] rather than treating it
/// as unknown. Releasing an unknown or already-released token is an
/// explicit failure, not a silent no-op.
///
/// See [`register_render_ring`].
pub(crate) fn release_render_ring(token: u64) -> Result<(), AudioAbiError> {
    let mut registry = audio_engine_registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match registry.entries.get_mut(&token) {
        Some(entry @ AudioEngineEntry::Active(_)) => {
            *entry = AudioEngineEntry::Released;
            Ok(())
        }
        Some(AudioEngineEntry::Released) => Err(AudioAbiError::AlreadyReleased),
        #[cfg(test)]
        Some(AudioEngineEntry::PanicOnRead) => Err(AudioAbiError::AlreadyReleased),
        None => Err(AudioAbiError::UnknownToken),
    }
}

/// Runs `f` against the active consumer for `token`, or returns `default`
/// for an unknown, released, or momentarily lock-contended token. Never
/// blocks: uses `try_lock`, never `lock`.
fn with_active_consumer<T>(token: u64, default: T, f: impl FnOnce(&RenderRingConsumer) -> T) -> T {
    let Some(registry) = try_lock_registry() else {
        return default;
    };
    match registry.entries.get(&token) {
        Some(AudioEngineEntry::Active(consumer)) => f(consumer),
        _ => default,
    }
}

fn record_contained_panic(token: u64) {
    with_active_consumer(token, (), RenderRingConsumer::record_contained_panic);
}

/// Core read logic once the caller-facing pointers have already been
/// validated and converted into a checked slice. Never panics under normal
/// operation; `#[cfg(test)]` registrations may deliberately panic here to
/// exercise the FFI boundary's panic containment.
fn read_interleaved_f32_impl(token: u64, output: &mut [f32]) -> (i32, u32) {
    let Some(registry) = try_lock_registry() else {
        output.fill(0.0);
        return (SilentDiscoAudioStatus::Stopping.code(), 0);
    };
    match registry.entries.get(&token) {
        Some(AudioEngineEntry::Active(consumer)) => {
            consumer.record_callback_invocation();
            let outcome = consumer.read_frames(output);
            let frames_from_ring = u32::try_from(outcome.frames_supplied).unwrap_or(u32::MAX);
            let status = if outcome.frames_silence_filled > 0 {
                SilentDiscoAudioStatus::Partial
            } else {
                SilentDiscoAudioStatus::Ok
            };
            (status.code(), frames_from_ring)
        }
        Some(AudioEngineEntry::Released) => {
            output.fill(0.0);
            (SilentDiscoAudioStatus::Stopping.code(), 0)
        }
        #[cfg(test)]
        Some(AudioEngineEntry::PanicOnRead) => {
            panic!("test-only injected panic in silent_disco_audio_read_interleaved_f32");
        }
        None => {
            output.fill(0.0);
            (SilentDiscoAudioStatus::InvalidState.code(), 0)
        }
    }
}

/// Returns the stable ABI contract version exposed to native callers.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn silent_disco_audio_abi_version() -> u32 {
    u32::from(AUDIO_ABI_VERSION)
}

/// Real-time-safe interleaved float read. See `include/silent_disco_audio.h`
/// for the full contract.
///
/// # Safety
///
/// `output`, if non-null, must point to at least
/// `requested_frames * output_channels` valid, properly aligned, writable
/// `f32` slots for the entire duration of this call. `frames_from_ring`, if
/// non-null, must point to one valid, writable `u32` for the entire
/// duration of this call. `engine` is never dereferenced; it is only
/// compared as an opaque bit pattern against previously registered tokens.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn silent_disco_audio_read_interleaved_f32(
    engine: *mut c_void,
    output: *mut f32,
    requested_frames: u32,
    output_channels: u32,
    frames_from_ring: *mut u32,
) -> i32 {
    if output.is_null() || frames_from_ring.is_null() || requested_frames == 0 {
        return SilentDiscoAudioStatus::InvalidArgument.code();
    }
    if output_channels != u32::try_from(RENDER_CHANNELS).unwrap_or(u32::MAX) {
        return SilentDiscoAudioStatus::InvalidArgument.code();
    }
    let Some(total_samples) = usize::try_from(requested_frames).ok().and_then(|frames| {
        usize::try_from(output_channels)
            .ok()
            .and_then(|channels| frames.checked_mul(channels))
    }) else {
        return SilentDiscoAudioStatus::InvalidArgument.code();
    };

    let token = engine as usize as u64;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `output` is non-null and the caller contracts that it
        // points to at least `total_samples` valid, writable `f32` slots
        // for this call's duration (see this function's `# Safety` doc).
        let output_slice = unsafe { std::slice::from_raw_parts_mut(output, total_samples) };
        read_interleaved_f32_impl(token, output_slice)
    }));

    let (status_code, frames_supplied) = if let Ok(result) = outcome {
        result
    } else {
        // SAFETY: same justification as above; the panicking closure's
        // slice reference no longer exists once it has unwound, so
        // constructing a fresh one over the same validated buffer here
        // does not overlap with any other live reference.
        let output_slice = unsafe { std::slice::from_raw_parts_mut(output, total_samples) };
        output_slice.fill(0.0);
        record_contained_panic(token);
        (SilentDiscoAudioStatus::PanicContained.code(), 0)
    };

    // SAFETY: `frames_from_ring` is non-null (checked above) and the caller
    // contracts that it points to one valid, writable `u32`.
    unsafe {
        *frames_from_ring = frames_supplied;
    }
    status_code
}

macro_rules! telemetry_query_export {
    ($function_name:ident, $return_type:ty, $accessor:expr) => {
        #[must_use]
        #[unsafe(no_mangle)]
        pub extern "C" fn $function_name(engine: *const c_void) -> $return_type {
            let token = engine as usize as u64;
            catch_unwind(AssertUnwindSafe(|| {
                with_active_consumer(token, 0, $accessor)
            }))
            .unwrap_or(0)
        }
    };
}

telemetry_query_export!(silent_disco_audio_available_frames, u32, |consumer| {
    u32::try_from(consumer.available_frames()).unwrap_or(u32::MAX)
});
telemetry_query_export!(silent_disco_audio_underrun_count, u64, |consumer| {
    consumer.telemetry().underrun_callbacks
});
telemetry_query_export!(silent_disco_audio_frames_rendered, u64, |consumer| {
    consumer.telemetry().frames_supplied_from_ring
});
telemetry_query_export!(silent_disco_audio_silence_filled_frames, u64, |consumer| {
    consumer.telemetry().silence_filled_frames
});
telemetry_query_export!(silent_disco_audio_ring_full_events, u64, |consumer| {
    consumer.telemetry().ring_full_events
});
telemetry_query_export!(silent_disco_audio_callback_count, u64, |consumer| {
    consumer.telemetry().callback_count
});
telemetry_query_export!(silent_disco_audio_contained_panic_count, u64, |consumer| {
    consumer.telemetry().contained_panic_count
});

#[cfg(test)]
mod tests {
    // Every comparison here checks an exact, bit-identical round trip through
    // the ring's `AtomicU32`-backed storage (real writes) or an exact silence
    // fill (zeroed slots), never approximate arithmetic — exact equality is
    // the correct check, not a precision-loss risk.
    #![allow(clippy::float_cmp)]

    use super::{
        AudioAbiError, AudioEngineEntry, audio_engine_registry, record_contained_panic,
        register_render_ring, release_render_ring, silent_disco_audio_abi_version,
        silent_disco_audio_available_frames, silent_disco_audio_callback_count,
        silent_disco_audio_contained_panic_count, silent_disco_audio_frames_rendered,
        silent_disco_audio_read_interleaved_f32, silent_disco_audio_ring_full_events,
        silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
    };
    use silent_disco_core::audio::RenderRingConfig;

    use super::registry_test_guard;

    fn small_ring_config() -> RenderRingConfig {
        RenderRingConfig {
            capacity_frames: silent_disco_core::audio::MIN_RING_CAPACITY_FRAMES,
            target_fill_frames: 1,
        }
    }

    fn token_as_engine(token: u64) -> *mut core::ffi::c_void {
        usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void
    }

    fn register_panicking_test_engine() -> u64 {
        let mut registry = audio_engine_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let token = registry.next_token;
        registry.next_token = token + 1;
        registry
            .entries
            .insert(token, AudioEngineEntry::PanicOnRead);
        token
    }

    #[test]
    fn abi_version_is_stable() {
        let _guard = registry_test_guard();
        assert_eq!(silent_disco_audio_abi_version(), 1);
    }

    #[test]
    fn null_engine_is_reported_as_invalid_state_and_zeroes_output() {
        let _guard = registry_test_guard();
        let mut output = [1.0_f32, 2.0, 3.0, 4.0];
        let mut frames_from_ring = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            2,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, -2 /* INVALID_STATE */);
        assert_eq!(output, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(frames_from_ring, 0);
    }

    #[test]
    fn null_output_is_reported_as_invalid_argument() {
        let _guard = registry_test_guard();
        let (_producer, token) = register_render_ring(small_ring_config()).expect("registered");
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            std::ptr::null_mut(),
            2,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, -1 /* INVALID_ARGUMENT */);
    }

    #[test]
    fn zero_requested_frames_is_reported_as_invalid_argument() {
        let _guard = registry_test_guard();
        let (_producer, token) = register_render_ring(small_ring_config()).expect("registered");
        let mut output = [0.0_f32; 2];
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            0,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, -1 /* INVALID_ARGUMENT */);
    }

    #[test]
    fn wrong_channel_count_is_reported_as_invalid_argument() {
        let _guard = registry_test_guard();
        let (_producer, token) = register_render_ring(small_ring_config()).expect("registered");
        let mut output = [0.0_f32; 4];
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            2,
            1, /* mono, but the engine is fixed stereo */
            &raw mut frames_from_ring,
        );
        assert_eq!(status, -1 /* INVALID_ARGUMENT */);
    }

    #[test]
    fn partial_read_fills_remaining_frames_with_silence_and_reports_partial() {
        let _guard = registry_test_guard();
        let (producer, token) = register_render_ring(small_ring_config()).expect("registered");
        assert_eq!(producer.push_frames(&[1.0, 2.0]), 1);

        let mut output = [-1.0_f32; 4];
        let mut frames_from_ring = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            2,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, 1 /* PARTIAL */);
        assert_eq!(frames_from_ring, 1);
        assert_eq!(output, [1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn full_read_reports_ok_with_every_frame_from_the_ring() {
        let _guard = registry_test_guard();
        let (producer, token) = register_render_ring(small_ring_config()).expect("registered");
        assert_eq!(producer.push_frames(&[1.0, 2.0, 3.0, 4.0]), 2);

        let mut output = [-1.0_f32; 4];
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            2,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, 0 /* OK */);
        assert_eq!(frames_from_ring, 2);
        assert_eq!(output, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn released_token_reports_stopping_and_silence() {
        let _guard = registry_test_guard();
        let (_producer, token) = register_render_ring(small_ring_config()).expect("registered");
        release_render_ring(token).expect("released");

        let mut output = [1.0_f32; 2];
        let mut frames_from_ring = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, 2 /* STOPPING */);
        assert_eq!(output, [0.0, 0.0]);
        assert_eq!(frames_from_ring, 0);

        assert_eq!(
            release_render_ring(token),
            Err(AudioAbiError::AlreadyReleased)
        );
    }

    #[test]
    fn releasing_an_unknown_token_is_an_explicit_failure() {
        let _guard = registry_test_guard();
        assert_eq!(
            release_render_ring(u64::MAX),
            Err(AudioAbiError::UnknownToken)
        );
    }

    #[test]
    fn contained_panic_zeroes_output_and_increments_the_panic_counter() {
        let _guard = registry_test_guard();
        let token = register_panicking_test_engine();

        let mut output = [1.0_f32; 2];
        let mut frames_from_ring = 99_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, -3 /* PANIC_CONTAINED */);
        assert_eq!(output, [0.0, 0.0]);
        assert_eq!(frames_from_ring, 0);
        // The panicking test engine never becomes `Active`, so there is no
        // consumer to record the panic against; confirm this does not
        // itself panic or deadlock the registry.
        record_contained_panic(token);
    }

    #[test]
    fn contained_panic_on_a_real_engine_increments_its_panic_counter() {
        let _guard = registry_test_guard();
        let (_producer, token) = register_render_ring(small_ring_config()).expect("registered");
        assert_eq!(
            silent_disco_audio_contained_panic_count(token_as_engine(token)),
            0
        );
        record_contained_panic(token);
        assert_eq!(
            silent_disco_audio_contained_panic_count(token_as_engine(token)),
            1
        );
    }

    #[test]
    fn abi_version_mismatch_is_detectable_by_the_caller() {
        let _guard = registry_test_guard();
        assert_ne!(silent_disco_audio_abi_version(), 0);
        assert_ne!(silent_disco_audio_abi_version(), 2);
    }

    #[test]
    fn telemetry_queries_reflect_real_ring_activity_and_default_to_zero_when_unknown() {
        let _guard = registry_test_guard();
        let (producer, token) = register_render_ring(small_ring_config()).expect("registered");
        assert_eq!(producer.push_frames(&[1.0, 2.0, 3.0, 4.0]), 2);

        let mut output = [0.0_f32; 2];
        let mut frames_from_ring = 0_u32;
        let _ = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );

        assert_eq!(
            silent_disco_audio_frames_rendered(token_as_engine(token)),
            1
        );
        assert_eq!(silent_disco_audio_callback_count(token_as_engine(token)), 1);
        assert_eq!(
            silent_disco_audio_available_frames(token_as_engine(token)),
            1
        );
        assert_eq!(silent_disco_audio_underrun_count(token_as_engine(token)), 0);
        assert_eq!(
            silent_disco_audio_silence_filled_frames(token_as_engine(token)),
            0
        );
        assert_eq!(
            silent_disco_audio_ring_full_events(token_as_engine(token)),
            0
        );

        let unknown = token_as_engine(u64::MAX);
        assert_eq!(silent_disco_audio_frames_rendered(unknown), 0);
        assert_eq!(silent_disco_audio_available_frames(unknown), 0);
    }

    #[test]
    fn token_space_exhaustion_is_a_typed_failure_not_a_panic() {
        let _guard = registry_test_guard();
        // Directly drive the registry to the edge of the token space rather
        // than actually registering u64::MAX engines.
        {
            let mut registry = audio_engine_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry.next_token = u64::MAX;
        }
        let (_producer, last_token) = register_render_ring(small_ring_config())
            .expect("the final valid token must still succeed");
        assert_eq!(last_token, u64::MAX);
        let second = register_render_ring(small_ring_config());
        assert_eq!(second.unwrap_err(), AudioAbiError::TokenSpaceExhausted);

        // Restore the registry so later tests in this process (which may
        // run before or after this one) are not affected by this
        // deliberately-exhausted token space or the token it registered:
        // `releasing_an_unknown_token_is_an_explicit_failure` specifically
        // relies on `u64::MAX` never having been registered.
        let mut registry = audio_engine_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.entries.remove(&last_token);
        registry.next_token = 1_000_000;
    }

    /// Block 24: a real-time reader thread (standing in for the Oboe/JNI
    /// callback) hammers `silent_disco_audio_read_interleaved_f32` on a live
    /// token while the control-plane thread releases it mid-flight, repeated
    /// many times with a fresh ring each iteration to vary the interleaving.
    /// Every observed status must be one of the three the real-time path is
    /// allowed to report (`Ok`, `Partial`, `Stopping`); `PanicContained`
    /// would mean the release itself made the read path panic, and any other
    /// value would mean a released token was misreported. Once
    /// `release_render_ring` returns, every subsequent read on that token
    /// must report `Stopping`, never fall back to `InvalidState` (which would
    /// mean the registry lost track of a token that really did exist).
    #[test]
    fn concurrent_release_races_with_concurrent_reads_and_never_reports_panic_or_invalid_state() {
        let _guard = registry_test_guard();
        for iteration in 0_u32..200 {
            let (producer, token) = register_render_ring(small_ring_config()).expect("registered");
            // Give the reader something real to drain on some iterations, and
            // nothing on others, so both a full-ring race and an empty-ring
            // race against release are exercised.
            if iteration % 2 == 0 {
                let _ = producer.push_frames(&[1.0, 2.0]);
            }

            let reader_token = token;
            let reader = std::thread::spawn(move || {
                for _ in 0..500 {
                    let mut output = [0.0_f32; 2];
                    let mut frames_from_ring = 0_u32;
                    let status = silent_disco_audio_read_interleaved_f32(
                        token_as_engine(reader_token),
                        output.as_mut_ptr(),
                        1,
                        2,
                        &raw mut frames_from_ring,
                    );
                    assert!(
                        (0..=2).contains(&status),
                        "read during a release race reported {status}, not Ok/Partial/Stopping"
                    );
                }
            });

            release_render_ring(token).expect("released exactly once");
            reader.join().expect("reader thread must not panic");

            // A read taken strictly after release must be Stopping, never
            // InvalidState (which would mean the token's identity was lost,
            // not just its liveness).
            let mut output = [0.0_f32; 2];
            let mut frames_from_ring = 0_u32;
            let status_after = silent_disco_audio_read_interleaved_f32(
                token_as_engine(token),
                output.as_mut_ptr(),
                1,
                2,
                &raw mut frames_from_ring,
            );
            assert_eq!(status_after, 2 /* STOPPING */);
        }
    }

    /// Block 24: many threads concurrently register and release rings
    /// against the shared, process-global registry. No two threads may ever
    /// be handed the same token (the registry's `next_token` counter must
    /// stay correctly serialized under contention), and no thread's release
    /// may fail as though its own registration never happened.
    #[test]
    fn concurrent_register_and_release_across_many_threads_never_collide_or_leak() {
        let _guard = registry_test_guard();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    let mut tokens = Vec::with_capacity(100);
                    for _ in 0..100 {
                        let (_producer, token) =
                            register_render_ring(small_ring_config()).expect("registered");
                        tokens.push(token);
                    }
                    for token in &tokens {
                        release_render_ring(*token).expect("released exactly once");
                    }
                    tokens
                })
            })
            .collect();

        let mut all_tokens = Vec::new();
        for thread in threads {
            all_tokens.extend(thread.join().expect("worker thread must not panic"));
        }

        let unique: std::collections::BTreeSet<_> = all_tokens.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all_tokens.len(),
            "two threads were handed the same token concurrently"
        );
    }
}
