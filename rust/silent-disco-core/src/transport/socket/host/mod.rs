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

use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{AudioDatagram, ControlMessage, Pause, StreamStart, TrackBoundary};
use crate::runtime::NetworkEndpoint;

use super::super::{HostTransportConfig, HostTransportNode, TransportClock, TransportEvent};
use super::shared::CounterState;

#[cfg(test)]
mod audio_generation_tests;
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
mod socket_multitrack_audio_reconnect_tests;

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
    // After Stop, a new Play must begin at packet zero before accepting
    // subsequent UDP. This fences future-dated packets from a retired
    // same-ID producer that were buffered beyond the replay start anchor.
    awaiting_replay_first_packet: bool,
    // After a Stop, require contiguous packet/sample progression for the
    // entire replay generation, not just its first packet. This rejects
    // delayed duplicate and out-of-order UDP from a retired same-ID sender.
    replay_next_packet: Option<(u64, u64)>,
    // Once a paused stream resumes with a new anchor, previously audible
    // boundaries must not be replayed or reintroduced by late retransmits.
    last_applied_boundary_sample_index: Option<u64>,
}

impl PlaybackControls {
    fn advance_paused_track(&mut self, resumed: &StreamStart) {
        let (Some(previous), Some(pause), Some(current_entry)) = (
            &self.stream_start,
            &self.pause,
            resumed.current_entry_id.as_deref(),
        ) else {
            return;
        };
        // The host actor's resumed current-entry ID is authoritative. A
        // send-ahead boundary can be in the cache without having applied to
        // that actor yet, so clock time ALONE must not retire it. Only prune
        // through a boundary confirmed both audible at Pause and current
        // in the host's resumed metadata.
        let elapsed_ms = pause
            .host_pause_time_ms
            .get()
            .saturating_sub(previous.host_start_time_ms.get());
        let played_samples = u128::from(elapsed_ms) * u128::from(previous.sample_rate);
        let confirmed = self
            .boundaries
            .iter()
            .filter(|boundary| {
                boundary.entry_id == current_entry
                    && u128::from(boundary.first_sample_index.get()) * 1_000 <= played_samples
            })
            .map(|boundary| boundary.first_sample_index.get())
            .last();
        let Some(confirmed_index) = confirmed else {
            return;
        };
        while self.boundaries.front().is_some_and(|boundary| {
            boundary.first_sample_index.get() <= confirmed_index
        }) {
            let applied = self.boundaries.pop_front().expect("front boundary exists");
            self.last_applied_boundary_sample_index = Some(applied.first_sample_index.get());
        }
    }

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
                let mut current_start = start.clone();
                if is_resume {
                    self.advance_paused_track(&current_start);
                } else {
                    self.boundaries.clear();
                    self.last_applied_boundary_sample_index = None;
                    self.last_pause_time_ms = None;
                    if self.last_stopped_control.is_some() {
                        self.awaiting_replay_first_packet = true;
                        self.replay_next_packet = None;
                    }
                }
                self.stream_start = Some(current_start);
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
                if self
                    .last_applied_boundary_sample_index
                    .is_some_and(|applied| boundary.first_sample_index.get() <= applied)
                    || self.boundaries.back().is_some_and(|previous| {
                        boundary.first_sample_index.get() <= previous.first_sample_index.get()
                    })
                {
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
                self.last_applied_boundary_sample_index = None;
                self.pause = None;
                self.last_pause_time_ms = None;
                self.awaiting_replay_first_packet = true;
                self.replay_next_packet = None;
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
                    self.awaiting_replay_first_packet = true;
                    self.replay_next_packet = None;
                    true
                }
            }
            ControlMessage::TrackBoundary(_)
            | ControlMessage::Pause(_)
            | ControlMessage::Stop(_) => false,
            _ => true,
        }
    }

    /// Audio is bound to the same authoritative generation as TCP controls.
    /// Older low-level transport callers can send fixture audio before any
    /// `StreamStart`, but once playback controls are established, a paused or
    /// retired producer must never leak its buffered UDP frames.
    pub(super) fn permits_audio(&mut self, packet: &AudioDatagram) -> bool {
        let allowed = match &self.stream_start {
            Some(start) => {
                start.session_id == packet.session_id
                    && start.stream_id == packet.stream_id
                    && self.pause.is_none()
                    // Stop/Play may intentionally reuse a stream ID. A
                    // delayed UDP datagram from the retired generation must
                    // not enter the newly authorized listener timeline.
                    // Only apply this generation fence after a Stop; older
                    // transport fixtures can send audio before an initial
                    // StreamStart presentation anchor.
                    && !self.last_stopped_control.as_ref().is_some_and(|(session, _)| {
                        session == &packet.session_id
                            && packet.host_presentation_time_ms.get()
                                < start.host_start_time_ms.get()
                    })
            }
            None => self.last_stopped_control.is_none(),
        };
        if !allowed {
            return false;
        }
        if self.awaiting_replay_first_packet
            && (packet.sequence.get() != 0 || packet.first_sample_index.get() != 0)
        {
            return false;
        }
        if self.replay_next_packet.is_some_and(|(sequence, sample)| {
            packet.sequence.get() != sequence || packet.first_sample_index.get() != sample
        }) {
            return false;
        }
        if self.awaiting_replay_first_packet || self.replay_next_packet.is_some() {
            let Some(sequence) = packet.sequence.get().checked_add(1) else {
                return false;
            };
            let Some(sample) = packet
                .first_sample_index
                .get()
                .checked_add(u64::from(packet.samples_per_packet))
            else {
                return false;
            };
            self.awaiting_replay_first_packet = false;
            self.replay_next_packet = Some((sequence, sample));
        }
        true
    }

    /// A reconnecting listener needs the track audible *now*, not the track
    /// that was current when `StreamStart` was originally broadcast. Host
    /// send-ahead can queue future boundaries, so derive the position from
    /// the presentation clock rather than the most recently sent UDP packet.
    /// While paused, freeze at the pause deadline; Resume reanchors Start.
    pub(super) fn snapshot_at(&self, now: MonotonicMillis) -> Vec<ControlMessage> {
        let Some(mut start) = self.stream_start.clone() else {
            return Vec::new();
        };
        let effective_now = self.pause.as_ref().map_or(now.get(), |pause| {
            now.get().min(pause.host_pause_time_ms.get())
        });
        let elapsed_ms = effective_now.saturating_sub(start.host_start_time_ms.get());
        let elapsed_samples = u128::from(elapsed_ms) * u128::from(start.sample_rate);
        let mut upcoming = Vec::new();
        for boundary in &self.boundaries {
            if u128::from(boundary.first_sample_index.get()) * 1_000 <= elapsed_samples {
                start.current_entry_id = Some(boundary.entry_id.clone());
                start.current_display_name = Some(boundary.display_name.clone());
            } else {
                upcoming.push(ControlMessage::TrackBoundary(boundary.clone()));
            }
        }
        let mut controls =
            Vec::with_capacity(1 + upcoming.len() + usize::from(self.pause.is_some()));
        controls.push(ControlMessage::StreamStart(start));
        controls.extend(upcoming);
        controls.extend(self.pause.clone().map(ControlMessage::Pause));
        controls
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Vec<ControlMessage> {
        self.snapshot_at(MonotonicMillis::new(0))
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
