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
#[cfg(test)]
mod authorization_order_tests;
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
    // Highest pause timestamp already observed for this stream. A Resume
    // clears the active pause but must not erase its retransmission fence.
    last_pause_time_ms: Option<u64>,
}

impl PlaybackControls {
    /// Returns false for stale or unrelated playback controls so that live
    /// listeners cannot receive controls rejected by late-join replay.
    pub(super) fn update(&mut self, message: &ControlMessage) -> bool {
        match message {
            ControlMessage::StreamStart(start) => {
                let is_resume = self.stream_start.as_ref().is_some_and(|previous| {
                    previous.stream_id == start.stream_id && previous.session_id == start.session_id
                });
                if !is_resume {
                    self.boundaries.clear();
                    self.last_pause_time_ms = None;
                }
                self.stream_start = Some(start.clone());
                self.pause = None;
                true
            }
            ControlMessage::TrackBoundary(boundary)
                if self.stream_start.as_ref().is_some_and(|start| {
                    start.stream_id == boundary.stream_id && start.session_id == boundary.session_id
                }) =>
            {
                // A delayed or conflicting retransmission must not rewind the
                // timeline replayed to an authorized late-joining listener.
                if self.boundaries.back().is_some_and(|previous| {
                    boundary.first_sample_index.get() <= previous.first_sample_index.get()
                }) {
                    false
                } else {
                    self.boundaries.push_back(boundary.clone());
                    while self.boundaries.len() > MAX_CACHED_TRACK_BOUNDARIES {
                        self.boundaries.pop_front();
                    }
                    true
                }
            }
            ControlMessage::Pause(pause)
                if self.stream_start.as_ref().is_some_and(|start| {
                    start.stream_id == pause.stream_id && start.session_id == pause.session_id
                }) =>
            {
                // Replay must never regress to a delayed or conflicting older
                // pause timestamp from this active stream.
                if self
                    .last_pause_time_ms
                    .is_some_and(|previous| pause.host_pause_time_ms.get() <= previous)
                {
                    false
                } else {
                    self.last_pause_time_ms = Some(pause.host_pause_time_ms.get());
                    self.pause = Some(pause.clone());
                    true
                }
            }
            ControlMessage::Stop(stop)
                if self.stream_start.as_ref().is_some_and(|start| {
                    start.stream_id == stop.stream_id && start.session_id == stop.session_id
                }) =>
            {
                *self = Self::default();
                true
            }
            // Explicit Stop must disarm idle listener liveness even when the
            // transport did not cache an earlier StreamStart. A Stop for a
            // different active stream, however, must not interrupt playback.
            ControlMessage::Stop(_) if self.stream_start.is_none() => true,
            ControlMessage::TrackBoundary(_)
            | ControlMessage::Pause(_)
            | ControlMessage::Stop(_) => false,
            _ => true,
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
    // Serializes control broadcasts with authorization replay. The new peer
    // must receive its cached StreamStart before any live TrackBoundary.
    control_delivery: Arc<Mutex<()>>,
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
