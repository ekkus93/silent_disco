#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::single_match_else
)]
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::domain::{DeviceId, SessionId};
use crate::runtime::NetworkEndpoint;

use super::super::{HostTransportConfig, HostTransportNode, TransportClock, TransportEvent};
use super::shared::CounterState;

mod authorization;
mod bind;
mod broadcast;
mod lookup;
mod node;
mod peer;

pub(super) use peer::{PeerRoute, PeerState};

pub struct SocketHostTransport {
    endpoint: NetworkEndpoint,
    session_id: SessionId,
    config: HostTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    sync_socket: Arc<UdpSocket>,
    audio_socket: Arc<UdpSocket>,
    event_sender: SyncSender<TransportEvent>,
    event_receiver: Receiver<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    shutdown_complete: bool,
}

impl Drop for SocketHostTransport {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}
