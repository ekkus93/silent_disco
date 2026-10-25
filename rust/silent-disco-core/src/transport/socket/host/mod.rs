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
mod reconnect_snapshot_tests;

#[cfg(test)]
mod playback_controls_format_tests;
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
    // A delayed StreamStart must not resurrect playback after Stop. Preserve
    // the most recently retired session's start-time fence across idle state.
    last_stopped_start: Option<(SessionId, u64)>,
    // The wall-clock control time of the last Stop, independent of its
    // future presentation anchor. A replay may reuse the stream ID; delayed
    // Pause/Stop from the retired generation must not affect the new Play.
    last_stopped_control: Option<(SessionId, u64)>,
}

impl PlaybackControls {
    fn retired_control(&self, session: &SessionId, control_time_ms: u64) -> bool {
        self.last_stopped_control
            .as_ref()
            .is_some_and(|(stopped_session, stopped_at)| {
                stopped_session == session && control_time_ms <= *stopped_at
            })
    }

    /// Returns false for stale or unrelated playback controls so that live
    /// listeners cannot receive controls rejected by late-join replay.
    pub(super) fn update(&mut self, message: &ControlMessage) -> bool {
        match message {
            ControlMessage::StreamStart(start) => {
                // Stop retires this generation, including its original
                // StreamStart. A fresh Play must advertise a later anchor;
                // the stream ID alone cannot distinguish delayed delivery.
                if self
                    .last_stopped_start
                    .as_ref()
                    .is_some_and(|(session, anchor)| {
                        *session == start.session_id && start.host_start_time_ms.get() <= *anchor
                    })
                {
                    return false;
                }
                // A different stream ID does not make an earlier start
                // legitimate: a delayed Play from before Stop can have an
                // anchor later than the retired stream's original anchor.
                if self.retired_control(&start.session_id, start.host_start_time_ms.get()) {
                    return false;
                }
                // A delayed start from an older generation must not rewind
                // the active clock or clear its paused state, even when a
                // subsequent Play assigned a different stream ID.
                if self.stream_start.as_ref().is_some_and(|previous| {
                    previous.session_id == start.session_id
                        && start.host_start_time_ms.get() < previous.host_start_time_ms.get()
                }) {
                    return false;
                }
                let is_resume = self.stream_start.as_ref().is_some_and(|previous| {
                    previous.stream_id == start.stream_id && previous.session_id == start.session_id
                });
                // A resume may refresh presentation metadata, but the audio
                // format is immutable for the lifetime of a stream. Reject a
                // malformed same-stream start before it can unpause playback
                // or corrupt the format replayed to a reconnecting listener.
                if is_resume
                    && self.stream_start.as_ref().is_some_and(|previous| {
                        previous.sample_rate != start.sample_rate
                            || previous.channels != start.channels
                            || previous.samples_per_packet != start.samples_per_packet
                    })
                {
                    return false;
                }
                // Resume advances the presentation anchor by the pause
                // duration. A delayed retransmission of the original start
                // (including a metadata variant) must never clear Pause or
                // release a held track boundary to a reconnecting listener.
                if is_resume
                    && self.pause.is_some()
                    && self.stream_start.as_ref().is_some_and(|previous| {
                        start.host_start_time_ms.get() <= previous.host_start_time_ms.get()
                    })
                {
                    return false;
                }
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
                if self.retired_control(&pause.session_id, pause.host_pause_time_ms.get()) {
                    return false;
                }
                // A StreamStart is a *future audio presentation deadline*, not
                // the time playback controls become valid. The host can Pause
                // during its startup/send-ahead window, before that deadline.
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
                if self.retired_control(&stop.session_id, stop.host_stop_time_ms.get()) {
                    return false;
                }
                // Stop is valid before the first scheduled sample is audible.
                // The stream ID, not the future presentation timestamp,
                // distinguishes a delayed Stop from a retired stream.
                let retired = self
                    .stream_start
                    .take()
                    .expect("active stream checked above");
                self.last_stopped_start =
                    Some((retired.session_id.clone(), retired.host_start_time_ms.get()));
                self.last_stopped_control =
                    Some((retired.session_id, stop.host_stop_time_ms.get()));
                self.boundaries.clear();
                self.pause = None;
                self.last_pause_time_ms = None;
                true
            }
            // Explicit Stop must disarm idle listener liveness even when the
            // transport did not cache an earlier StreamStart. A Stop for a
            // different active stream, however, must not interrupt playback.
            ControlMessage::Stop(stop) if self.stream_start.is_none() => {
                if self.retired_control(&stop.session_id, stop.host_stop_time_ms.get()) {
                    false
                } else {
                    // Stop may race ahead of StreamStart during admission or
                    // reconnect. Remember the idle Stop watermark so a late
                    // start cannot resurrect playback for a new listener.
                    self.last_stopped_control =
                        Some((stop.session_id.clone(), stop.host_stop_time_ms.get()));
                    true
                }
            }
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
