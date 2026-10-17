use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silent_disco_core::audio::{
    PacketizerWorkerError, PacketizerWorkerErrorKind, StreamingDecodeConfig, StreamingDecodeHandle,
    StreamingPacketizeConfig, StreamingPacketizeHandle, StreamingPlaylistTrackFactory,
    StreamingPlaylistUpdater,
};
use silent_disco_core::domain::{PlaybackState, SessionId, StreamId};
use silent_disco_core::runtime::CoreActorHandle;
use crate::host_control::FfiCoreHandle;
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame, TrackBoundary};

type FfiBoundaryReserver = Arc<
    dyn Fn(&TrackBoundary, Arc<AtomicBool>) -> Result<(), PacketizerWorkerError> + Send + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiPlaylistSource {
    pub entry_id: String,
    pub display_name: String,
    pub staged_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiPlaylistStreamStart {
    pub stream_id: String,
    pub host_start_time_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples_per_packet: u32,
    pub current_entry_id: Option<String>,
    pub current_display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPlaylistFrame {
    Audio {
        stream_id: String,
        sequence: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        first_sample_index: u64,
        host_presentation_time_ms: u64,
        payload: Vec<u8>,
    },
    TrackBoundary {
        stream_id: String,
        first_sample_index: u64,
        entry_id: String,
        display_name: String,
    },
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiHostPlaylistError {
    InvalidConfiguration(String),
    Decode(String),
    Packetizer(String),
    Closed(String),
}

impl std::fmt::Display for FfiHostPlaylistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Decode(message)
            | Self::Packetizer(message)
            | Self::Closed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FfiHostPlaylistError {}

#[derive(uniffi::Object)]
pub struct FfiHostPlaylistStreamer {
    inner: Mutex<Option<StreamingPacketizeHandle>>,
    closed: AtomicBool,
    cancellation: Arc<AtomicBool>,
    updater: StreamingPlaylistUpdater,
    stream_start: FfiPlaylistStreamStart,
}

impl std::fmt::Debug for FfiHostPlaylistStreamer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FfiHostPlaylistStreamer")
    }
}

#[allow(clippy::missing_errors_doc)]
#[uniffi::export]
impl FfiHostPlaylistStreamer {
    #[uniffi::constructor]
    pub fn open(
        session_id: String,
        stream_id: String,
        host_start_time_ms: u64,
        sources: Vec<FfiPlaylistSource>,
    ) -> Result<Arc<Self>, FfiHostPlaylistError> {
        Self::open_with_reserver(session_id, stream_id, host_start_time_ms, sources, None)
    }

    /// Production host constructor: the producer reserves each successor on
    /// the serialized actor before it enters the bounded output queue.
    #[uniffi::constructor]
    pub fn open_authoritative(
        session_id: String,
        stream_id: String,
        host_start_time_ms: u64,
        sources: Vec<FfiPlaylistSource>,
        authority: Arc<FfiCoreHandle>,
    ) -> Result<Arc<Self>, FfiHostPlaylistError> {
        let actor = authority
            .actor_handle()
            .map_err(|error| FfiHostPlaylistError::Closed(error.to_string()))?;
        let reserve = Arc::new(move |boundary: &TrackBoundary, cancelled: Arc<AtomicBool>| {
            reserve_authoritative_boundary(&actor, boundary, cancelled)
        });
        Self::open_with_reserver(
            session_id,
            stream_id,
            host_start_time_ms,
            sources,
            Some(reserve),
        )
    }

    pub fn stream_start(&self) -> FfiPlaylistStreamStart {
        self.stream_start.clone()
    }

    pub fn replace_upcoming(
        &self,
        sources: Vec<FfiPlaylistSource>,
    ) -> Result<(), FfiHostPlaylistError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(FfiHostPlaylistError::Closed(
                "playlist stream has already stopped".to_owned(),
            ));
        }
        let decode_config = StreamingDecodeConfig::default();
        let factories = sources
            .into_iter()
            .map(|source| {
                let path = PathBuf::from(source.staged_path);
                StreamingPlaylistTrackFactory::new(
                    source.entry_id,
                    source.display_name,
                    move || StreamingDecodeHandle::open(path, decode_config),
                )
                .map_err(|error| FfiHostPlaylistError::Packetizer(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.updater.replace_upcoming(factories).map_err(|error| {
            if error.kind == PacketizerWorkerErrorKind::Cancelled {
                FfiHostPlaylistError::Closed(error.to_string())
            } else {
                FfiHostPlaylistError::Packetizer(error.to_string())
            }
        })
    }

    /// Returns `None` when no frame arrived before the timeout or after clean EOF.
    pub fn next_frame(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<FfiPlaylistFrame>, FfiHostPlaylistError> {
        // Cancellation may be signalled while another consumer is holding
        // this mutex. Do not deliver frames buffered before Stop.
        if self.closed.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut inner = self.inner.lock().map_err(|_| {
            FfiHostPlaylistError::Closed("playlist stream lock was poisoned".to_owned())
        })?;
        if self.closed.load(Ordering::Acquire) {
            return Ok(None);
        }
        let Some(packetizer) = inner.as_ref() else {
            return Ok(None);
        };
        let received = packetizer.recv_timeout(Duration::from_millis(timeout_ms));
        if self.closed.load(Ordering::Acquire) {
            return Ok(None);
        }
        match received {
            Ok(frame) => frame_to_ffi(frame).map(Some),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.closed.store(true, Ordering::Release);
                let Some(packetizer) = inner.take() else {
                    return Ok(None);
                };
                packetizer
                    .join()
                    .map_err(|error| FfiHostPlaylistError::Packetizer(error.to_string()))?;
                Ok(Some(FfiPlaylistFrame::End))
            }
        }
    }

    pub fn cancel(&self) -> Result<(), FfiHostPlaylistError> {
        self.closed.store(true, Ordering::Release);
        // Wake an actor-reservation wait before contending with next_frame.
        self.cancellation.store(true, Ordering::Release);
        let mut inner = self.inner.lock().map_err(|_| {
            FfiHostPlaylistError::Closed("playlist stream lock was poisoned".to_owned())
        })?;
        if let Some(packetizer) = inner.take()
            && let Err(error) = packetizer.cancel_and_join()
            && error.kind != PacketizerWorkerErrorKind::Cancelled
        {
            return Err(FfiHostPlaylistError::Packetizer(error.to_string()));
        }
        Ok(())
    }
}

impl FfiHostPlaylistStreamer {
    fn open_with_reserver(
        session_id: String,
        stream_id: String,
        host_start_time_ms: u64,
        sources: Vec<FfiPlaylistSource>,
        reserve: Option<FfiBoundaryReserver>,
    ) -> Result<Arc<Self>, FfiHostPlaylistError> {
        if sources.is_empty() {
            return Err(FfiHostPlaylistError::InvalidConfiguration(
                "host playlist is empty".to_owned(),
            ));
        }
        let session_id = SessionId::new(session_id)
            .map_err(|error| FfiHostPlaylistError::InvalidConfiguration(error.to_string()))?;
        let stream_id = StreamId::new(stream_id)
            .map_err(|error| FfiHostPlaylistError::InvalidConfiguration(error.to_string()))?;
        let mut sources = sources.into_iter();
        let current = sources.next().ok_or_else(|| {
            FfiHostPlaylistError::InvalidConfiguration("host playlist is empty".to_owned())
        })?;
        let decode_config = StreamingDecodeConfig::default();
        let decoder =
            StreamingDecodeHandle::open(&current.staged_path, decode_config).map_err(|error| {
                FfiHostPlaylistError::Decode(format!("{}: {error}", current.display_name))
            })?;
        let upcoming = sources
            .map(|source| {
                let path = PathBuf::from(source.staged_path);
                StreamingPlaylistTrackFactory::new(
                    source.entry_id,
                    source.display_name,
                    move || StreamingDecodeHandle::open(path, decode_config),
                )
                .map_err(|error| FfiHostPlaylistError::Packetizer(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let packetizer = match reserve {
            Some(reserve) => StreamingPacketizeHandle::spawn_playlist_reserved(
                decoder,
                current.entry_id.clone(),
                current.display_name.clone(),
                upcoming,
                session_id,
                stream_id,
                silent_disco_core::domain::MonotonicMillis::new(host_start_time_ms),
                StreamingPacketizeConfig::default(),
                reserve,
            ),
            None => StreamingPacketizeHandle::spawn_playlist(
                decoder,
                current.entry_id.clone(),
                current.display_name.clone(),
                upcoming,
                session_id,
                stream_id,
                silent_disco_core::domain::MonotonicMillis::new(host_start_time_ms),
                StreamingPacketizeConfig::default(),
            ),
        }
        .map_err(|error| FfiHostPlaylistError::Packetizer(error.to_string()))?;
        let ProtocolFrame::Control(ControlMessage::StreamStart(start)) =
            packetizer.stream_start_message()
        else {
            return Err(FfiHostPlaylistError::Packetizer(
                "playlist packetizer returned an invalid start frame".to_owned(),
            ));
        };
        let cancellation = packetizer.cancellation_signal();
        let stream_start = FfiPlaylistStreamStart {
            stream_id: start.stream_id.to_string(),
            host_start_time_ms: start.host_start_time_ms.get(),
            sample_rate: start.sample_rate,
            channels: start.channels,
            samples_per_packet: start.samples_per_packet,
            current_entry_id: start.current_entry_id.clone(),
            current_display_name: start.current_display_name.clone(),
        };
        let updater = packetizer.playlist_updater().ok_or_else(|| {
            FfiHostPlaylistError::Packetizer(
                "playlist packetizer did not provide an update handle".to_owned(),
            )
        })?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(packetizer)),
            closed: AtomicBool::new(false),
            cancellation,
            updater,
            stream_start,
        }))
    }
}

/// This callback runs on the Rust producer thread, never on Kotlin's frame
/// consumer. It holds neither the decoder queue nor the output-channel lock.
fn reserve_authoritative_boundary(
    actor: &CoreActorHandle,
    boundary: &TrackBoundary,
    cancelled: Arc<AtomicBool>,
) -> Result<(), PacketizerWorkerError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(PacketizerWorkerError {
                kind: PacketizerWorkerErrorKind::Cancelled,
                message: "host boundary reservation cancelled".to_owned(),
            });
        }
        let playback_state = actor
            .current_snapshot()
            .map_err(|error| playlist_reservation_error(&error.to_string()))?
            .playback_state;
        match playback_state {
            PlaybackState::Playing => break,
            PlaybackState::Stopped | PlaybackState::Error => {
                return Err(playlist_reservation_error("playback stopped before reservation"));
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    actor.reserve_host_track_boundary(
        boundary.stream_id.clone(),
        boundary.entry_id.clone(),
        boundary.first_sample_index.get(),
        cancelled,
    )
    .map_err(|error| playlist_reservation_error(&error.to_string()))
}

fn playlist_reservation_error(message: &str) -> PacketizerWorkerError {
    PacketizerWorkerError {
        kind: PacketizerWorkerErrorKind::Packetize,
        message: format!("authoritative host boundary reservation failed: {message}"),
    }
}

fn frame_to_ffi(frame: ProtocolFrame) -> Result<FfiPlaylistFrame, FfiHostPlaylistError> {
    match frame {
        ProtocolFrame::Audio(audio) => Ok(FfiPlaylistFrame::Audio {
            stream_id: audio.stream_id.to_string(),
            sequence: audio.sequence.get(),
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            samples_per_packet: audio.samples_per_packet,
            first_sample_index: audio.first_sample_index.get(),
            host_presentation_time_ms: audio.host_presentation_time_ms.get(),
            payload: audio.payload,
        }),
        ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)) => {
            Ok(FfiPlaylistFrame::TrackBoundary {
                stream_id: boundary.stream_id.to_string(),
                first_sample_index: boundary.first_sample_index.get(),
                entry_id: boundary.entry_id,
                display_name: boundary.display_name,
            })
        }
        _ => Err(FfiHostPlaylistError::Packetizer(
            "playlist packetizer emitted an unsupported frame".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests;
