#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::single_match_else
)]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, Pause, StreamStart, TrackBoundary};
use crate::runtime::NetworkEndpoint;

use super::super::{HostTransportConfig, HostTransportNode, TransportClock, TransportEvent};
use super::shared::CounterState;

mod authorization;
mod bind;
mod broadcast;
mod lookup;
mod node;
mod peer;

#[cfg(test)]
mod playback_controls_tests;

const MAX_CACHED_TRACK_BOUNDARIES: usize = 100;

#[derive(Default)]
pub(super) struct PlaybackControls {
    stream_start: Option<StreamStart>,
    boundaries: VecDeque<TrackBoundary>,
    pause: Option<Pause>,
}

impl PlaybackControls {
    pub(super) fn update(&mut self, message: &ControlMessage) {
        match message {
            ControlMessage::StreamStart(start) => {
                let is_resume = self
                    .stream_start
                    .as_ref()
                    .is_some_and(|previous| previous.stream_id == start.stream_id);
                if !is_resume {
                    self.boundaries.clear();
                }
                self.stream_start = Some(start.clone());
                self.pause = None;
            }
            ControlMessage::TrackBoundary(boundary)
                if self
                    .stream_start
                    .as_ref()
                    .is_some_and(|start| start.stream_id == boundary.stream_id) =>
            {
                if self.boundaries.back().is_none_or(|previous| {
                    previous.entry_id != boundary.entry_id
                        || previous.first_sample_index != boundary.first_sample_index
                }) {
                    self.boundaries.push_back(boundary.clone());
                    while self.boundaries.len() > MAX_CACHED_TRACK_BOUNDARIES {
                        self.boundaries.pop_front();
                    }
                }
            }
            ControlMessage::Pause(pause)
                if self
                    .stream_start
                    .as_ref()
                    .is_some_and(|start| start.stream_id == pause.stream_id) =>
            {
                self.pause = Some(pause.clone());
            }
            ControlMessage::Stop(stop)
                if self
                    .stream_start
                    .as_ref()
                    .is_some_and(|start| start.stream_id == stop.stream_id) =>
            {
                *self = Self::default();
            }
            _ => {}
        }
    }

    pub(super) fn snapshot(&self) -> Vec<ControlMessage> {
        let Some(start) = self.stream_start.clone() else {
            return Vec::new();
        };
        let mut controls =
            Vec::with_capacity(1 + self.boundaries.len() + usize::from(self.pause.is_some()));
        controls.push(ControlMessage::StreamStart(start));
        controls.extend(
            self.boundaries
                .iter()
                .cloned()
                .map(ControlMessage::TrackBoundary),
        );
        controls.extend(self.pause.clone().map(ControlMessage::Pause));
        controls
    }
}

pub(super) use peer::{PeerRoute, PeerState};

pub struct SocketHostTransport {
    endpoint: NetworkEndpoint,
    session_id: SessionId,
    config: HostTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    playback_controls: Mutex<PlaybackControls>,
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
        let result = self.shutdown();
        super::shared::fail_loud_on_implicit_shutdown(&result, "socket host transport");
    }
}
