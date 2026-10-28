//! Bounded replay accounting for future authenticated v4 datagrams.
//!
//! This is NOT a packet authenticator. A caller must verify the AEAD tag,
//! authenticated peer, traffic-key generation and associated data BEFORE
//! committing a sequence here. Legacy-v3 UDP must never use these counters
//! as evidence of cryptographic authenticity.

use std::collections::HashMap;

const REPLAY_BITS: u64 = 128;
const MAX_PEER_CHANNELS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatagramPurpose {
    Audio,
    Synchronization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReplayKey {
    pub peer_fingerprint: [u8; 32],
    pub generation: u64,
    pub purpose: DatagramPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
    InvalidIdentity,
    InvalidGeneration,
    TooManyPeers,
    UnknownKey,
    RetiredKey,
    WrongGeneration,
    Duplicate,
    TooOld,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReplayWindow {
    highest: Option<u64>,
    seen: u128,
}

impl ReplayWindow {
    fn check(&self, sequence: u64) -> Result<(), ReplayError> {
        let Some(highest) = self.highest else {
            return Ok(());
        };
        if sequence > highest {
            return Ok(());
        }
        let distance = highest - sequence;
        if distance >= REPLAY_BITS {
            return Err(ReplayError::TooOld);
        }
        if self.seen & (1_u128 << distance) != 0 {
            return Err(ReplayError::Duplicate);
        }
        Ok(())
    }

    fn commit(&mut self, sequence: u64) -> Result<(), ReplayError> {
        self.check(sequence)?;
        if let Some(highest) = self.highest {
            if sequence > highest {
                let distance = sequence - highest;
                self.seen = if distance >= REPLAY_BITS {
                    1
                } else {
                    (self.seen << distance) | 1
                };
                self.highest = Some(sequence);
            } else {
                self.seen |= 1_u128 << (highest - sequence);
            }
        } else {
            self.highest = Some(sequence);
            self.seen = 1;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PeerChannel {
    generation: u64,
    active: bool,
    window: ReplayWindow,
}

/// Session-local, bounded replay windows for authenticated datagram channels.
///
/// Keep one window per verified peer and traffic purpose. A retired generation
/// cannot be reactivated: new TLS exporter keys must use a higher generation.
/// Retired entries retain only their generation fence until session teardown,
/// preventing old traffic from reopening a route with an empty replay window.
#[derive(Debug, Default)]
pub struct SecureDatagramReplay {
    channels: HashMap<([u8; 32], DatagramPurpose), PeerChannel>,
}

impl SecureDatagramReplay {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a fresh authenticated traffic-key generation.
    ///
    /// # Errors
    /// Rejects zero identities/generations, non-increasing generations, and
    /// attempts to exceed the bounded peer/channel registry.
    pub fn activate(&mut self, key: ReplayKey) -> Result<(), ReplayError> {
        if key.peer_fingerprint == [0; 32] {
            return Err(ReplayError::InvalidIdentity);
        }
        if key.generation == 0 {
            return Err(ReplayError::InvalidGeneration);
        }
        let slot = (key.peer_fingerprint, key.purpose);
        if let Some(previous) = self.channels.get_mut(&slot) {
            if key.generation <= previous.generation {
                return Err(ReplayError::WrongGeneration);
            }
            *previous = PeerChannel {
                generation: key.generation,
                active: true,
                window: ReplayWindow::default(),
            };
        } else {
            if self.channels.len() >= MAX_PEER_CHANNELS {
                return Err(ReplayError::TooManyPeers);
            }
            self.channels.insert(
                slot,
                PeerChannel {
                    generation: key.generation,
                    active: true,
                    window: ReplayWindow::default(),
                },
            );
        }
        Ok(())
    }

    /// Non-mutating early replay check; a successful result is NOT proof of
    /// authenticity. Failed AEAD verification must never call `commit_verified`.
    ///
    /// # Errors
    /// Rejects unknown, retired, mismatched, duplicate and stale packets.
    pub fn check_candidate(&self, key: ReplayKey, sequence: u64) -> Result<(), ReplayError> {
        let channel = self
            .channels
            .get(&(key.peer_fingerprint, key.purpose))
            .ok_or(ReplayError::UnknownKey)?;
        if key.generation != channel.generation {
            return Err(ReplayError::WrongGeneration);
        }
        if !channel.active {
            return Err(ReplayError::RetiredKey);
        }
        channel.window.check(sequence)
    }

    /// Record a sequence ONLY AFTER successful AEAD verification with the
    /// exact traffic key and associated data represented by `key`. This
    /// operation is separate from `check_candidate` so invalid tags cannot
    /// poison the receive window or suppress legitimate later packets.
    ///
    /// # Errors
    /// Rejects unknown/retired generations, duplicate or stale sequences.
    pub fn commit_verified(&mut self, key: ReplayKey, sequence: u64) -> Result<(), ReplayError> {
        self.check_candidate(key, sequence)?;
        let Some(channel) = self.channels.get_mut(&(key.peer_fingerprint, key.purpose)) else {
            return Err(ReplayError::UnknownKey);
        };
        channel.window.commit(sequence)
    }

    /// Retires the exact traffic generation. Older generations remain fenced.
    pub fn retire(&mut self, key: ReplayKey) -> bool {
        let Some(channel) = self.channels.get_mut(&(key.peer_fingerprint, key.purpose)) else {
            return false;
        };
        if channel.generation != key.generation || !channel.active {
            return false;
        }
        channel.active = false;
        true
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(peer: u8, generation: u64, purpose: DatagramPurpose) -> ReplayKey {
        ReplayKey {
            peer_fingerprint: [peer; 32],
            generation,
            purpose,
        }
    }

    #[test]
    fn duplicate_out_of_order_and_window_expiry() {
        let peer = key(1, 1, DatagramPurpose::Audio);
        let mut replay = SecureDatagramReplay::new();
        replay.activate(peer).expect("activate");
        replay.commit_verified(peer, 200).expect("first packet");
        replay.commit_verified(peer, 198).expect("reordered packet");
        assert_eq!(
            replay.commit_verified(peer, 198),
            Err(ReplayError::Duplicate)
        );
        assert_eq!(
            replay.commit_verified(peer, 200),
            Err(ReplayError::Duplicate)
        );
        replay.commit_verified(peer, 327).expect("last in window");
        assert_eq!(replay.commit_verified(peer, 199), Err(ReplayError::TooOld));
        replay
            .commit_verified(peer, 326)
            .expect("new reordered packet");
        replay
            .commit_verified(peer, 455)
            .expect("large forward jump");
        assert_eq!(replay.commit_verified(peer, 327), Err(ReplayError::TooOld));
        assert_eq!(
            replay.commit_verified(peer, 455),
            Err(ReplayError::Duplicate)
        );
    }

    #[test]
    fn failed_tag_does_not_consume_sequence_or_shift_window() {
        let peer = key(2, 3, DatagramPurpose::Synchronization);
        let mut replay = SecureDatagramReplay::new();
        replay.activate(peer).expect("activate");
        replay
            .commit_verified(peer, 1)
            .expect("authenticated first");
        assert_eq!(replay.check_candidate(peer, 1_000), Ok(()));
        // Simulated bad AEAD tag: do NOT commit the candidate.
        replay.commit_verified(peer, 0).expect("valid older packet");
        replay.commit_verified(peer, 2).expect("valid successor");
        assert_eq!(replay.commit_verified(peer, 1), Err(ReplayError::Duplicate));
    }

    #[test]
    fn key_rotation_retirement_and_channel_peer_isolation() {
        let audio = key(3, 5, DatagramPurpose::Audio);
        let sync = key(3, 5, DatagramPurpose::Synchronization);
        let other = key(4, 5, DatagramPurpose::Audio);
        let mut replay = SecureDatagramReplay::new();
        for key in [audio, sync, other] {
            replay.activate(key).expect("activate independent key");
            replay.commit_verified(key, 7).expect("independent counter");
        }
        assert!(replay.retire(audio));
        assert!(!replay.retire(audio));
        assert_eq!(
            replay.commit_verified(audio, 8),
            Err(ReplayError::RetiredKey)
        );
        assert_eq!(replay.activate(audio), Err(ReplayError::WrongGeneration));
        assert_eq!(
            replay.activate(key(3, 4, DatagramPurpose::Audio)),
            Err(ReplayError::WrongGeneration)
        );
        let rotated = key(3, 6, DatagramPurpose::Audio);
        replay.activate(rotated).expect("fresh traffic key");
        replay.commit_verified(rotated, 0).expect("fresh counter");
        assert_eq!(
            replay.commit_verified(audio, 9),
            Err(ReplayError::WrongGeneration)
        );
        assert_eq!(replay.commit_verified(sync, 7), Err(ReplayError::Duplicate));
        replay.commit_verified(sync, 8).expect("sync unaffected");
        replay
            .commit_verified(other, 8)
            .expect("other peer unaffected");
    }

    #[test]
    fn counter_wrap_cannot_reuse_an_authenticated_nonce() {
        let peer = key(5, 1, DatagramPurpose::Audio);
        let mut replay = SecureDatagramReplay::new();
        replay.activate(peer).expect("activate");
        replay
            .commit_verified(peer, u64::MAX - 1)
            .expect("penultimate");
        replay
            .commit_verified(peer, u64::MAX)
            .expect("final counter");
        assert_eq!(replay.commit_verified(peer, 0), Err(ReplayError::TooOld));
        assert_eq!(
            replay.commit_verified(peer, u64::MAX),
            Err(ReplayError::Duplicate)
        );
        replay.retire(peer);
        replay
            .activate(key(5, 2, DatagramPurpose::Audio))
            .expect("rekey");
    }

    #[test]
    fn bounded_registry_and_invalid_contexts_fail_closed() {
        let mut replay = SecureDatagramReplay::new();
        assert_eq!(
            replay.activate(key(0, 1, DatagramPurpose::Audio)),
            Err(ReplayError::InvalidIdentity)
        );
        assert_eq!(
            replay.activate(key(1, 0, DatagramPurpose::Audio)),
            Err(ReplayError::InvalidGeneration)
        );
        assert_eq!(
            replay.commit_verified(key(1, 1, DatagramPurpose::Audio), 0),
            Err(ReplayError::UnknownKey)
        );
        // A 32-byte fingerprint supports more than one-byte test fixtures.
        for n in 0..MAX_PEER_CHANNELS {
            let mut fingerprint = [1; 32];
            fingerprint[0..8].copy_from_slice(&(n as u64).to_be_bytes());
            replay
                .activate(ReplayKey {
                    peer_fingerprint: fingerprint,
                    generation: 1,
                    purpose: DatagramPurpose::Audio,
                })
                .expect("bounded slot");
        }
        assert_eq!(replay.len(), MAX_PEER_CHANNELS);
        assert_eq!(
            replay.activate(key(9, 1, DatagramPurpose::Synchronization)),
            Err(ReplayError::TooManyPeers)
        );
    }
}
