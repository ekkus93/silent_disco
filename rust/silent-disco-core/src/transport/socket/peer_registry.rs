//! Shared handles every host control peer needs.
//!
//! The plaintext accept path and the TLS admission path both register peers
//! here, so a listener admitted by either route gets the same peer table entry,
//! the same event ordering, the same terminal disconnect and the same route
//! cleanup. Nothing in this module decides admission; it only carries state.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::domain::DeviceId;

use super::super::{HostTransportConfig, TransportClock, TransportEvent};
use super::host::{PeerRoute, PeerState, terminal_mailbox::TerminalEventMailbox};
use super::shared::{ControlSender, ControlWrite, CounterState};

#[derive(Clone)]
pub(super) struct PeerRegistry {
    pub(super) config: HostTransportConfig,
    /// Host-wide shutdown flag.
    pub(super) stop: Arc<AtomicBool>,
    pub(super) counters: Arc<CounterState>,
    pub(super) peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    pub(super) terminal_mailbox: Arc<TerminalEventMailbox>,
    pub(super) routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    pub(super) workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub(super) event_sender: SyncSender<TransportEvent>,
    pub(super) clock: Arc<dyn TransportClock>,
    pub(super) next_peer: Arc<AtomicU64>,
}

impl PeerRegistry {
    /// True when another peer would exceed `max_peers`. Retained terminal
    /// events count, because they still hold their admission slots until the
    /// actor collects them.
    pub(super) fn is_at_capacity(&self) -> bool {
        let current = self
            .peers
            .lock()
            .map_or(self.config.max_peers, |peers| peers.len());
        current.saturating_add(self.terminal_mailbox.pending()) >= self.config.max_peers
    }

    /// Creates the provisional peer for a freshly accepted connection. The
    /// writer queue feeds whichever transport task drains it; `shutdown` is a
    /// handle the host can use to close that connection.
    pub(super) fn new_peer(
        &self,
        number: u64,
        remote: SocketAddr,
        write_sender: SyncSender<ControlWrite>,
        shutdown: TcpStream,
    ) -> Arc<PeerState> {
        Arc::new(PeerState {
            number,
            remote,
            identity: Mutex::new(None),
            authorized: AtomicBool::new(false),
            admission_denied: AtomicBool::new(false),
            join_forwarded: AtomicBool::new(false),
            active: AtomicBool::new(true),
            consecutive_failures: AtomicU32::new(0),
            sender: ControlSender::new(
                write_sender,
                self.config.operation_timeout,
                self.counters.clone(),
            ),
            stop: Arc::new(AtomicBool::new(false)),
            shutdown_stream: Mutex::new(shutdown),
            // Use registration time so a new peer is not already stale.
            last_inbound_millis: AtomicU64::new(self.clock.now().get()),
            silence_armed: AtomicBool::new(false),
        })
    }

    /// Bounded writer queue for a new peer. Its capacity is the configured
    /// outbound queue size.
    pub(super) fn writer_queue(
        &self,
    ) -> (
        SyncSender<ControlWrite>,
        std::sync::mpsc::Receiver<ControlWrite>,
    ) {
        sync_channel(self.config.outbound_queue_capacity)
    }
}
