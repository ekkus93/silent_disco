//! Desktop's controlled acquire/release gate around the shared Rust render
//! ring's safe consumer API (Block 32, the Phase 9 local-monitor-audio
//! prerequisite).
//!
//! Desktop links `silent-disco-core` directly (see `Cargo.toml`), so it
//! already has full, ordinary access to `RenderRing::new(..).split()` --
//! unlike Android, it never needs the narrow C ABI token registry in
//! `silent-disco-ffi`'s `audio_abi.rs` (that registry exists only because a
//! native/JNI caller cannot hold a real Rust type; desktop can). What
//! desktop is still missing is a *controlled* acquisition gate: something
//! that guarantees only one producer/consumer pair is ever outstanding at a
//! time, mirroring the C ABI registry's `Active`/`Released` distinction
//! without needing tokens or FFI.
//!
//! This module is exactly that gate and nothing more. It does not schedule
//! anything itself -- production of frames into the ring stays the job of
//! the existing `PlaybackScheduler`/`PlaybackPump`
//! (`silent_disco_core::audio`), reused unchanged by a future local-monitor
//! feature (Block 33+). The gate's whole purpose is to make it structurally
//! impossible for that future feature to end up with two competing
//! producer/consumer pairs feeding two separate scheduling paths.

use silent_disco_core::audio::{
    RenderRing, RenderRingConfig, RenderRingConfigError, RenderRingConsumer, RenderRingProducer,
};
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    Idle,
    Active,
}

/// Controlled acquire/release gate for one shared render ring's
/// producer/consumer pair. At most one [`DesktopRenderConsumerLease`] may be
/// outstanding for a given gate at a time.
pub(crate) struct DesktopRenderRingGate {
    state: Mutex<GateState>,
}

impl Default for DesktopRenderRingGate {
    fn default() -> Self {
        Self {
            state: Mutex::new(GateState::Idle),
        }
    }
}

impl DesktopRenderRingGate {
    #[must_use]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Acquires the one allowed producer/consumer pair for this gate.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopRenderRingError::AlreadyActive`] if a lease is
    /// already outstanding, [`DesktopRenderRingError::Config`] if `config`
    /// is invalid, or [`DesktopRenderRingError::Poisoned`] if a previous
    /// holder panicked while holding the gate's internal lock. In every
    /// failure case the gate's state is left exactly as it was -- a failed
    /// acquire never leaves the gate stuck `Active` with nothing to release.
    pub(crate) fn acquire(
        self: &Arc<Self>,
        config: RenderRingConfig,
    ) -> Result<(RenderRingProducer, DesktopRenderConsumerLease), DesktopRenderRingError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopRenderRingError::Poisoned)?;
        if *state == GateState::Active {
            return Err(DesktopRenderRingError::AlreadyActive);
        }
        // Only ever flips to `Active` after `RenderRing::new` has already
        // succeeded, so a rejected config can never leave the gate stuck.
        let ring = RenderRing::new(config).map_err(DesktopRenderRingError::Config)?;
        let (producer, consumer) = ring.split();
        *state = GateState::Active;
        drop(state);
        Ok((
            producer,
            DesktopRenderConsumerLease {
                consumer: Some(consumer),
                gate: Arc::clone(self),
            },
        ))
    }
}

/// The one outstanding consumer lease a [`DesktopRenderRingGate`] can hand
/// out. Dropping it (explicitly or otherwise) releases the gate, exactly
/// like the C ABI registry's explicit `release_render_ring` -- there is no
/// separate "release" method to forget to call.
pub(crate) struct DesktopRenderConsumerLease {
    consumer: Option<RenderRingConsumer>,
    gate: Arc<DesktopRenderRingGate>,
}

impl DesktopRenderConsumerLease {
    /// Mutable access to the underlying consumer -- e.g. for a future CPAL
    /// output callback (Block 33+) to read frames from.
    pub(crate) fn consumer_mut(&mut self) -> &mut RenderRingConsumer {
        self.consumer
            .as_mut()
            .expect("consumer is only ever taken by Drop, never observably absent")
    }
}

impl Drop for DesktopRenderConsumerLease {
    fn drop(&mut self) {
        // Drop the real consumer first, then release the gate -- so a
        // fresh acquire() that immediately follows never observes a gate
        // that says "idle" while the previous consumer is still alive.
        self.consumer = None;
        if let Ok(mut state) = self.gate.state.lock() {
            *state = GateState::Idle;
        }
        // A poisoned gate mutex is left poisoned rather than papered over:
        // every subsequent acquire() will explicitly surface `Poisoned`,
        // consistent with this codebase's other lock-guarded state (see
        // `platform::network::DesktopNetworkError::poisoned`) rather than
        // silently recovering from a panic that happened elsewhere while
        // holding this lock.
    }
}

/// Failure while acquiring a render ring consumer lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DesktopRenderRingError {
    /// A lease is already outstanding for this gate.
    AlreadyActive,
    /// The requested ring configuration is invalid.
    Config(RenderRingConfigError),
    /// A previous holder of the gate's internal lock panicked.
    Poisoned,
}

impl fmt::Display for DesktopRenderRingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => formatter.write_str(
                "a render ring consumer is already active; only one may be outstanding at a time",
            ),
            Self::Config(error) => write!(formatter, "invalid render ring configuration: {error}"),
            Self::Poisoned => {
                formatter.write_str("the desktop render ring gate's internal lock is poisoned")
            }
        }
    }
}

impl std::error::Error for DesktopRenderRingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::AlreadyActive | Self::Poisoned => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopRenderRingError, DesktopRenderRingGate};
    use silent_disco_core::audio::{RENDER_CHANNELS, RenderRingConfig};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn acquiring_once_yields_a_working_producer_consumer_pair() {
        let gate = DesktopRenderRingGate::new();
        let (producer, mut lease) = gate.acquire(RenderRingConfig::default()).expect("acquire");

        let frames = [0.1_f32, -0.1, 0.2, -0.2];
        assert_eq!(producer.push_frames(&frames), frames.len() / RENDER_CHANNELS);

        let mut output = [0.0_f32; 4];
        let outcome = lease.consumer_mut().read_frames(&mut output);
        assert_eq!(outcome.frames_supplied * RENDER_CHANNELS, frames.len());
        assert_eq!(output, frames);
    }

    #[test]
    fn a_second_acquire_while_active_is_rejected() {
        let gate = DesktopRenderRingGate::new();
        let _first = gate.acquire(RenderRingConfig::default()).expect("first acquire");

        assert_eq!(
            gate.acquire(RenderRingConfig::default()).err(),
            Some(DesktopRenderRingError::AlreadyActive),
        );
    }

    #[test]
    fn dropping_the_lease_allows_a_fresh_acquire() {
        let gate = DesktopRenderRingGate::new();
        let first = gate.acquire(RenderRingConfig::default()).expect("first acquire");
        drop(first);

        assert!(gate.acquire(RenderRingConfig::default()).is_ok());
    }

    #[test]
    fn an_invalid_config_is_rejected_without_leaving_the_gate_stuck() {
        let gate = DesktopRenderRingGate::new();
        let invalid = RenderRingConfig {
            capacity_frames: 0,
            target_fill_frames: 0,
        };

        assert!(matches!(
            gate.acquire(invalid),
            Err(DesktopRenderRingError::Config(_))
        ));
        // The rejected attempt must not have left the gate `Active`.
        assert!(gate.acquire(RenderRingConfig::default()).is_ok());
    }

    /// Every `acquire()` call serializes on the gate's own lock, so this is
    /// a genuine proof there is no time-of-check/time-of-use gap between
    /// observing `Idle` and committing to `Active` -- not merely that the
    /// happy path works under no contention.
    ///
    /// Deliberately keeps every `acquire()` result (including any winning
    /// lease) alive in `results` until after counting: dropping a winning
    /// lease early would release the gate mid-test and let a later
    /// contender "win" too, which would prove nothing about the actual
    /// race at acquisition time.
    #[test]
    fn concurrent_acquire_attempts_yield_exactly_one_winner() {
        const ATTEMPTS: usize = 16;
        let gate = DesktopRenderRingGate::new();
        let barrier = Arc::new(Barrier::new(ATTEMPTS));

        let handles: Vec<_> = (0..ATTEMPTS)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    gate.acquire(RenderRingConfig::default())
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread panicked"))
            .collect();

        let successes = results.iter().filter(|result| result.is_ok()).count();
        assert_eq!(successes, 1);
    }
}
