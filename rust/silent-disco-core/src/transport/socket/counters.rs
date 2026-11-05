//! Lock-free transport counters shared by the socket host and listener workers.
//! Counters never carry peer identities, addresses or payload bytes.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::super::super::{TransportChannel, TransportCounters};

#[derive(Debug, Default)]
pub(in crate::transport::socket) struct CounterState {
    accepted_control_connections: AtomicU64,
    closed_control_connections: AtomicU64,
    control_frames_sent: AtomicU64,
    control_frames_received: AtomicU64,
    sync_datagrams_sent: AtomicU64,
    sync_datagrams_received: AtomicU64,
    audio_datagrams_sent: AtomicU64,
    audio_datagrams_received: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    malformed_frames: AtomicU64,
    oversized_frames: AtomicU64,
    unauthorized_frames: AtomicU64,
    queue_overflows: AtomicU64,
    delivery_failures: AtomicU64,
    best_effort_event_drops: AtomicU64,
    busy_admission_refusals: AtomicU64,
    duplicate_join_suppressions: AtomicU64,
    authoritative_event_failures: AtomicU64,
    worker_panicked: AtomicBool,
}

impl CounterState {
    pub(in crate::transport::socket) fn snapshot(&self) -> TransportCounters {
        TransportCounters {
            accepted_control_connections: self.accepted_control_connections.load(Ordering::Relaxed),
            closed_control_connections: self.closed_control_connections.load(Ordering::Relaxed),
            control_frames_sent: self.control_frames_sent.load(Ordering::Relaxed),
            control_frames_received: self.control_frames_received.load(Ordering::Relaxed),
            sync_datagrams_sent: self.sync_datagrams_sent.load(Ordering::Relaxed),
            sync_datagrams_received: self.sync_datagrams_received.load(Ordering::Relaxed),
            audio_datagrams_sent: self.audio_datagrams_sent.load(Ordering::Relaxed),
            audio_datagrams_received: self.audio_datagrams_received.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            malformed_frames: self.malformed_frames.load(Ordering::Relaxed),
            oversized_frames: self.oversized_frames.load(Ordering::Relaxed),
            unauthorized_frames: self.unauthorized_frames.load(Ordering::Relaxed),
            queue_overflows: self.queue_overflows.load(Ordering::Relaxed),
            delivery_failures: self.delivery_failures.load(Ordering::Relaxed),
            best_effort_event_drops: self.best_effort_event_drops.load(Ordering::Relaxed),
            busy_admission_refusals: self.busy_admission_refusals.load(Ordering::Relaxed),
            duplicate_join_suppressions: self.duplicate_join_suppressions.load(Ordering::Relaxed),
            authoritative_event_failures: self.authoritative_event_failures.load(Ordering::Relaxed),
        }
    }

    pub(in crate::transport::socket) fn accepted(&self) {
        self.accepted_control_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn closed(&self) {
        self.closed_control_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn frame_sent(&self, bytes: usize) {
        self.control_frames_sent.fetch_add(1, Ordering::Relaxed);
        self.add_sent(bytes);
    }

    pub(in crate::transport::socket) fn frame_received(&self, bytes: usize) {
        self.control_frames_received.fetch_add(1, Ordering::Relaxed);
        self.add_received(bytes);
    }

    pub(in crate::transport::socket) fn datagram_sent(
        &self,
        channel: TransportChannel,
        bytes: usize,
    ) {
        match channel {
            TransportChannel::Synchronization => {
                self.sync_datagrams_sent.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Audio => {
                self.audio_datagrams_sent.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Control | TransportChannel::Runtime => {}
        }
        self.add_sent(bytes);
    }

    pub(in crate::transport::socket) fn datagram_received(
        &self,
        channel: TransportChannel,
        bytes: usize,
    ) {
        match channel {
            TransportChannel::Synchronization => {
                self.sync_datagrams_received.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Audio => {
                self.audio_datagrams_received
                    .fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Control | TransportChannel::Runtime => {}
        }
        self.add_received(bytes);
    }

    pub(in crate::transport::socket) fn malformed(&self) {
        self.malformed_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn oversized(&self) {
        self.oversized_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn unauthorized(&self) {
        self.unauthorized_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn queue_overflow(&self) {
        self.queue_overflows.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn delivery_failure(&self) {
        self.delivery_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn best_effort_drop(&self) {
        self.best_effort_event_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn busy_admission_refusal(&self) {
        self.busy_admission_refusals.fetch_add(1, Ordering::Relaxed);
    }

    /// A repeated `JoinRequest` on an already-admitted TCP generation was
    /// ignored instead of creating another approval or enrollment.
    pub(in crate::transport::socket) fn duplicate_join_suppression(&self) {
        self.duplicate_join_suppressions
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn authoritative_event_failure(&self) {
        self.authoritative_event_failures
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::transport::socket) fn mark_worker_panicked(&self) {
        self.worker_panicked.store(true, Ordering::Release);
    }

    pub(in crate::transport::socket) fn has_worker_panic(&self) -> bool {
        self.worker_panicked.load(Ordering::Acquire)
    }

    fn add_sent(&self, bytes: usize) {
        self.bytes_sent
            .fetch_add(saturating_u64(bytes), Ordering::Relaxed);
    }

    fn add_received(&self, bytes: usize) {
        self.bytes_received
            .fetch_add(saturating_u64(bytes), Ordering::Relaxed);
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
