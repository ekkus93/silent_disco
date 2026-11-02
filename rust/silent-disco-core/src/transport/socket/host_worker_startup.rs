use super::*;

/// A failed worker spawn happens *after* PeerAccepted was delivered to the
/// host actor. Close the provisional socket and emit a matching terminal
/// event before releasing its peer slot; otherwise the actor retains a ghost
/// listener and repeated failures exhaust max_peers. The writer may already
/// be running when reader startup fails, so close its socket as well.
#[allow(clippy::too_many_arguments)]
pub(super) fn rollback_failed_peer_start(
    peer: &Arc<PeerState>,
    peers: &Mutex<HashMap<u64, Arc<PeerState>>>,
    event_sender: &SyncSender<TransportEvent>,
    global_stop: &AtomicBool,
    counters: &CounterState,
    received_at: crate::domain::MonotonicMillis,
    timeout: Duration,
    error: &TransportError,
) {
    drop(peer.close());
    // Keep the slot reserved until the actor observes the terminal event.
    // On a stalled/shutdown actor, fail within the bounded deadline and
    // release the slot rather than leaving an unobservable live socket.
    send_authoritative_event_bounded(
        event_sender,
        global_stop,
        counters,
        TransportEvent::PeerDisconnected {
            peer: peer.transport_peer(),
            error: Some(error.clone()),
            received_at,
        },
        timeout,
    );
    if let Ok(mut registry) = peers.lock() {
        registry.remove(&peer.number);
    }
    counters.closed();
}

