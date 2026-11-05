//! Seeded model-based tests for the authenticated-datagram replay window.
//!
//! A deliberately simple reference model (a set of accepted sequences and the
//! highest one) is run beside `SecureDatagramReplay` over random sequences,
//! including values near `u64::MAX` and gaps wider than the 128-bit window.
//! The real implementation must agree with the model on every decision, never
//! accept a sequence twice, and keep peers, purposes and generations apart.

use std::collections::BTreeSet;

use super::DeterministicPrng;
use super::secure_replay::{DatagramPurpose, ReplayError, ReplayKey, SecureDatagramReplay};

const WINDOW: u64 = 128;

#[derive(Default)]
struct Model {
    accepted: BTreeSet<u64>,
    highest: Option<u64>,
}

impl Model {
    fn expected(&self, sequence: u64) -> Result<(), ReplayError> {
        let Some(highest) = self.highest else {
            return Ok(());
        };
        if sequence > highest {
            return Ok(());
        }
        if highest - sequence >= WINDOW {
            return Err(ReplayError::TooOld);
        }
        if self.accepted.contains(&sequence) {
            return Err(ReplayError::Duplicate);
        }
        Ok(())
    }

    fn commit(&mut self, sequence: u64) {
        self.accepted.insert(sequence);
        self.highest = Some(
            self.highest
                .map_or(sequence, |highest| highest.max(sequence)),
        );
    }
}

fn key(peer: u8, generation: u64, purpose: DatagramPurpose) -> ReplayKey {
    ReplayKey {
        peer_fingerprint: [peer; 32],
        generation,
        purpose,
    }
}

/// Mostly nearby values (to hit duplicates and the window edge), sometimes
/// far jumps and values at the numeric extremes.
fn next_sequence(prng: &mut DeterministicPrng, around: u64) -> u64 {
    match prng.next_below(10) {
        0 => prng.next_u64(),
        1 => u64::MAX - prng.next_below(300) as u64,
        2 => around.saturating_add(WINDOW + prng.next_below(64) as u64),
        3 => around.saturating_sub(WINDOW - 1 + prng.next_below(3) as u64),
        _ => around
            .saturating_add(prng.next_below(40) as u64)
            .saturating_sub(prng.next_below(40) as u64),
    }
}

#[test]
fn window_agrees_with_the_reference_model_over_random_sequences() {
    for seed in 0..40_u64 {
        let mut prng = DeterministicPrng::new(0x5245_504C_4159_0000 ^ seed);
        let channel = key(1, 1, DatagramPurpose::Audio);
        let mut replay = SecureDatagramReplay::new();
        replay.activate(channel).expect("activate");
        let mut model = Model::default();
        let mut around = prng.next_u64() >> 8;
        for _ in 0..2_000 {
            let sequence = next_sequence(&mut prng, around);
            let expected = model.expected(sequence);
            // A candidate check never changes state.
            assert_eq!(
                replay.check_candidate(channel, sequence),
                expected,
                "check seed {seed} seq {sequence}"
            );
            assert_eq!(
                replay.check_candidate(channel, sequence),
                expected,
                "check is not idempotent"
            );
            let actual = replay.commit_verified(channel, sequence);
            assert_eq!(actual, expected, "commit seed {seed} seq {sequence}");
            if expected.is_ok() {
                model.commit(sequence);
                around = sequence;
                // Once accepted, the same sequence can never be accepted again.
                assert_eq!(
                    replay.commit_verified(channel, sequence),
                    Err(ReplayError::Duplicate)
                );
            }
        }
    }
}

#[test]
fn peers_purposes_and_generations_never_share_a_window() {
    let mut prng = DeterministicPrng::new(0x5245_504C_4159_0001);
    let mut replay = SecureDatagramReplay::new();
    let audio_a = key(1, 1, DatagramPurpose::Audio);
    let sync_a = key(1, 1, DatagramPurpose::Synchronization);
    let audio_b = key(2, 1, DatagramPurpose::Audio);
    for channel in [audio_a, sync_a, audio_b] {
        replay.activate(channel).expect("activate");
    }
    let mut sequence = prng.next_u64() >> 2;
    for _ in 0..500 {
        // Ascending, so every first use is inside the window.
        sequence += 1 + prng.next_below(5) as u64;
        replay
            .commit_verified(audio_a, sequence)
            .expect("first use on audio A");
        // The same number is fresh on every other channel.
        replay
            .check_candidate(sync_a, sequence)
            .expect("other purpose is independent");
        replay
            .check_candidate(audio_b, sequence)
            .expect("other peer is independent");
        assert_eq!(
            replay.commit_verified(audio_a, sequence),
            Err(ReplayError::Duplicate)
        );
        // A stale generation key is refused even for an unused sequence.
        assert_eq!(
            replay.check_candidate(key(1, 2, DatagramPurpose::Audio), sequence.wrapping_add(1)),
            Err(ReplayError::WrongGeneration)
        );
        let _ = replay.commit_verified(sync_a, sequence);
        let _ = replay.commit_verified(audio_b, sequence);
    }
}

#[test]
fn rekeying_resets_the_window_only_for_a_newer_generation_and_old_keys_are_dead() {
    let mut prng = DeterministicPrng::new(0x5245_504C_4159_0002);
    for _ in 0..200 {
        let mut replay = SecureDatagramReplay::new();
        let first = key(7, 1, DatagramPurpose::Audio);
        replay.activate(first).expect("activate");
        let sequences: Vec<u64> = (0..20).map(|_| prng.next_u64() >> 2).collect();
        for sequence in &sequences {
            let _ = replay.commit_verified(first, *sequence);
        }
        // Re-activating the same or an older generation must not reset replay state.
        assert_eq!(replay.activate(first), Err(ReplayError::WrongGeneration));
        assert_eq!(
            replay.activate(key(7, 0, DatagramPurpose::Audio)),
            Err(ReplayError::InvalidGeneration)
        );
        let second = key(7, 2, DatagramPurpose::Audio);
        replay.activate(second).expect("newer generation");
        // The old generation can no longer be used to commit anything.
        for sequence in &sequences {
            assert_eq!(
                replay.commit_verified(first, *sequence),
                Err(ReplayError::WrongGeneration)
            );
        }
        // The new generation starts fresh, so a number used earlier is accepted once.
        replay
            .commit_verified(second, sequences[0])
            .expect("fresh window");
        assert_eq!(
            replay.commit_verified(second, sequences[0]),
            Err(ReplayError::Duplicate)
        );
        // A retired key stays dead and cannot be retired twice.
        assert!(replay.retire(second));
        assert!(!replay.retire(second));
        assert_eq!(
            replay.check_candidate(second, u64::MAX),
            Err(ReplayError::RetiredKey)
        );
    }
}

#[test]
fn the_peer_channel_registry_is_bounded_and_invalid_identities_are_refused() {
    let mut replay = SecureDatagramReplay::new();
    let mut accepted = 0_usize;
    let mut refused = 0_usize;
    for index in 0..1_200_u32 {
        let mut fingerprint = [0_u8; 32];
        fingerprint[..4].copy_from_slice(&(index + 1).to_le_bytes());
        let outcome = replay.activate(ReplayKey {
            peer_fingerprint: fingerprint,
            generation: 1,
            purpose: DatagramPurpose::Audio,
        });
        match outcome {
            Ok(()) => accepted += 1,
            Err(ReplayError::TooManyPeers) => refused += 1,
            Err(other) => panic!("unexpected refusal {other:?}"),
        }
    }
    assert_eq!(accepted, 512, "the registry cap is exactly 512 channels");
    assert_eq!(refused, 1_200 - 512);
    assert_eq!(replay.len(), 512);
    assert_eq!(
        replay.activate(ReplayKey {
            peer_fingerprint: [0; 32],
            generation: 1,
            purpose: DatagramPurpose::Audio
        }),
        Err(ReplayError::InvalidIdentity)
    );
}
