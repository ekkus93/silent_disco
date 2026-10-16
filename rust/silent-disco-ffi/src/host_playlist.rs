use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silent_disco_core::audio::{
    PacketizerWorkerErrorKind, StreamingDecodeConfig, StreamingDecodeHandle,
    StreamingPacketizeConfig, StreamingPacketizeHandle, StreamingPlaylistTrackFactory,
    StreamingPlaylistUpdater,
};
use silent_disco_core::domain::{SessionId, StreamId};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};

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
        let packetizer = StreamingPacketizeHandle::spawn_playlist(
            decoder,
            current.entry_id.clone(),
            current.display_name.clone(),
            upcoming,
            session_id,
            stream_id,
            silent_disco_core::domain::MonotonicMillis::new(host_start_time_ms),
            StreamingPacketizeConfig::default(),
        )
        .map_err(|error| FfiHostPlaylistError::Packetizer(error.to_string()))?;
        let ProtocolFrame::Control(ControlMessage::StreamStart(start)) =
            packetizer.stream_start_message()
        else {
            return Err(FfiHostPlaylistError::Packetizer(
                "playlist packetizer returned an invalid start frame".to_owned(),
            ));
        };
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
            updater,
            stream_start,
        }))
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
        let mut inner = self.inner.lock().map_err(|_| {
            FfiHostPlaylistError::Closed("playlist stream lock was poisoned".to_owned())
        })?;
        let Some(packetizer) = inner.as_ref() else {
            return Ok(None);
        };
        match packetizer.recv_timeout(Duration::from_millis(timeout_ms)) {
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
mod tests {
    use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn direct_ffi_playlist_cannot_bypass_authoritative_queue_entry_limits() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-queue-bounds-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let source = root.join("source.wav");
        fs::write(&source, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("valid source");
        let staged_path = source.to_string_lossy().into_owned();
        let make_source = |id: String| FfiPlaylistSource {
            entry_id: id,
            display_name: "Track".to_owned(),
            staged_path: staged_path.clone(),
        };

        let oversized = (0..=100)
            .map(|index| make_source(format!("entry-{index}")))
            .collect();
        let error = FfiHostPlaylistStreamer::open(
            "session-ffi-overflow".to_owned(),
            "stream-ffi-overflow".to_owned(),
            5_000,
            oversized,
        )
        .expect_err("direct FFI open must enforce the 100-entry queue bound");
        assert!(error.to_string().contains("100-entry"));

        let duplicate = vec![
            make_source("same-entry".to_owned()),
            make_source("same-entry".to_owned()),
        ];
        let error = FfiHostPlaylistStreamer::open(
            "session-ffi-duplicate".to_owned(),
            "stream-ffi-duplicate".to_owned(),
            5_000,
            duplicate,
        )
        .expect_err("direct FFI open must reject duplicate entry IDs");
        assert!(error.to_string().contains("duplicate entry IDs"));

        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-live-bounds".to_owned(),
            "stream-ffi-live-bounds".to_owned(),
            5_000,
            vec![make_source("current".to_owned())],
        )
        .expect("valid single-track stream");
        let too_many_upcoming = (0..100)
            .map(|index| make_source(format!("upcoming-{index}")))
            .collect();
        let error = stream
            .replace_upcoming(too_many_upcoming)
            .expect_err("live update must reserve one current-track slot");
        assert!(error.to_string().contains("100-entry"));
        let error = stream
            .replace_upcoming(vec![
                make_source("duplicate".to_owned()),
                make_source("duplicate".to_owned()),
            ])
            .expect_err("live update must reject duplicate upcoming IDs");
        assert!(error.to_string().contains("duplicate entry IDs"));
        stream.cancel().expect("cancel stream");
        let error = stream.replace_upcoming(Vec::new())
            .expect_err("closed stream must not silently accept queue edits");
        assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn ffi_update_reports_closed_when_producer_finished_but_end_not_consumed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-unobserved-eof-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let source = root.join("short.wav");
        fs::write(&source, wav_pcm16(48_000, 2, 241, 3_000)).expect("source");
        let stream = FfiHostPlaylistStreamer::open(
            "session-unobserved-eof".to_owned(),
            "stream-unobserved-eof".to_owned(),
            5_000,
            vec![FfiPlaylistSource {
                entry_id: "current".to_owned(),
                display_name: "Short".to_owned(),
                staged_path: source.to_string_lossy().into_owned(),
            }],
        )
        .expect("open short stream");
        // Join the short producer without calling next_frame. This leaves
        // the UniFFI object's consumer-observed closed flag false.
        let worker = stream.inner.lock().expect("lock").take().expect("worker");
        worker.join().expect("producer completed");
        assert!(!stream.closed.load(std::sync::atomic::Ordering::Acquire));
        let error = stream.replace_upcoming(Vec::new())
            .expect_err("completed producer must not accept a phantom update");
        assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn ffi_playlist_stream_keeps_audio_sequence_and_sample_clock_across_boundary() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-playlist-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let first = root.join("first.wav");
        let second = root.join("second.wav");
        fs::write(&first, wav_pcm16(48_000, 2, 240, 3_000)).expect("first source");
        fs::write(&second, wav_pcm16(44_100, 1, 441, 6_000)).expect("second source");

        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-playlist".to_owned(),
            "stream-ffi-playlist".to_owned(),
            5_000,
            vec![
                FfiPlaylistSource {
                    entry_id: "entry-first".to_owned(),
                    display_name: "First".to_owned(),

                    staged_path: first.to_string_lossy().into_owned(),
                },
                FfiPlaylistSource {
                    entry_id: "entry-second".to_owned(),
                    display_name: "Second".to_owned(),
                    staged_path: second.to_string_lossy().into_owned(),
                },
            ],
        )
        .expect("open FFI playlist");
        let start = stream.stream_start();
        assert_eq!(start.current_entry_id.as_deref(), Some("entry-first"));
        assert_eq!(start.current_display_name.as_deref(), Some("First"));

        let mut sequence = Vec::new();
        let mut audio_samples = Vec::new();
        let mut audio_frame_counts = Vec::new();
        let mut saw_boundary = false;
        let mut boundary_position = None;
        let mut next_track_audio_position = None;
        let mut frame_position = 0;
        loop {
            match stream.next_frame(2_000).expect("next playlist frame") {
                Some(FfiPlaylistFrame::Audio {
                    sequence: packet_sequence,
                    first_sample_index,
                    payload,
                    ..
                }) => {
                    if saw_boundary && next_track_audio_position.is_none() {
                        next_track_audio_position = Some(frame_position);
                    }
                    sequence.push(packet_sequence);
                    audio_samples.push(first_sample_index);
                    audio_frame_counts.push(payload.len() as u64 / 4);
                    frame_position += 1;
                }
                Some(FfiPlaylistFrame::TrackBoundary {
                    entry_id,
                    first_sample_index,
                    ..
                }) => {
                    assert_eq!(entry_id, "entry-second");
                    assert_eq!(first_sample_index, 240);
                    saw_boundary = true;
                    boundary_position = Some(frame_position);
                    frame_position += 1;
                }
                Some(FfiPlaylistFrame::End) => break,
                None => panic!("playlist timed out"),
            }
        }
        assert!(saw_boundary);
        assert_eq!(audio_samples.first(), Some(&0));
        for index in 1..audio_samples.len() {
            assert_eq!(
                audio_samples[index],
                audio_samples[index - 1] + audio_frame_counts[index - 1],
                "playlist output must have no inserted or overlapped samples",
            );
        }
        assert_eq!(
            audio_samples
                .iter()
                .zip(&audio_frame_counts)
                .map(|(first, count)| first + count)
                .next_back(),
            Some(720),
        );
        assert!(
            boundary_position.expect("boundary") < next_track_audio_position.expect("next audio")
        );
        assert!(sequence.windows(2).all(|pair| pair[1] == pair[0] + 1));
        let error = stream.replace_upcoming(Vec::new())
            .expect_err("completed stream must not accept queue edits");
        assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn ffi_playlist_can_replace_the_upcoming_source_before_its_boundary() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-playlist-update-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let current = root.join("current.wav");
        let stale = root.join("stale.wav");
        let replacement = root.join("replacement.wav");
        fs::write(&current, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("current source");
        fs::write(&stale, wav_pcm16(48_000, 2, 480, 4_000)).expect("stale source");
        fs::write(&replacement, wav_pcm16(48_000, 2, 480, 6_000)).expect("replacement source");

        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-update".to_owned(),
            "stream-ffi-update".to_owned(),
            5_000,
            vec![
                FfiPlaylistSource {
                    entry_id: "entry-current".to_owned(),
                    display_name: "Current".to_owned(),
                    staged_path: current.to_string_lossy().into_owned(),
                },
                FfiPlaylistSource {
                    entry_id: "entry-stale".to_owned(),
                    display_name: "Stale".to_owned(),
                    staged_path: stale.to_string_lossy().into_owned(),
                },
            ],
        )
        .expect("open FFI playlist");
        let duplicate_error = stream
            .replace_upcoming(vec![FfiPlaylistSource {
                entry_id: "entry-current".to_owned(),
                display_name: "Duplicate current".to_owned(),
                staged_path: current.to_string_lossy().into_owned(),
            }])
            .expect_err("UniFFI must reject a live update that duplicates the active track");
        assert!(duplicate_error.to_string().contains("duplicate entry IDs"));
        stream
            .replace_upcoming(vec![FfiPlaylistSource {
                entry_id: "entry-replacement".to_owned(),
                display_name: "Replacement".to_owned(),
                staged_path: replacement.to_string_lossy().into_owned(),
            }])
            .expect("retarget upcoming queue");

        let mut boundaries = Vec::new();
        loop {
            match stream.next_frame(2_000).expect("read playlist") {
                Some(FfiPlaylistFrame::TrackBoundary { entry_id, .. }) => boundaries.push(entry_id),
                Some(FfiPlaylistFrame::End) => break,
                Some(FfiPlaylistFrame::Audio { .. }) => {}
                None => panic!("updated FFI playlist stalled"),
            }
        }
        assert_eq!(boundaries, ["entry-replacement"]);
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn failed_upcoming_source_finishes_current_track_and_names_failed_item() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-playlist-failure-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let current = root.join("current.wav");
        fs::write(&current, wav_pcm16(48_000, 2, 241, 3_000)).expect("current source");

        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-failure".to_owned(),
            "stream-ffi-failure".to_owned(),
            5_000,
            vec![
                FfiPlaylistSource {
                    entry_id: "entry-current".to_owned(),
                    display_name: "Current song".to_owned(),
                    staged_path: current.to_string_lossy().into_owned(),
                },
                FfiPlaylistSource {
                    entry_id: "entry-unavailable".to_owned(),
                    display_name: "Unavailable song".to_owned(),
                    staged_path: root.join("unavailable.wav").to_string_lossy().into_owned(),
                },
            ],
        )
        .expect("start playlist with lazy upcoming source");

        let mut audio_frames = 0;
        let mut saw_boundary = false;
        let error = loop {
            match stream.next_frame(2_000) {
                Ok(Some(FfiPlaylistFrame::Audio { .. })) => audio_frames += 1,
                Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => saw_boundary = true,
                Ok(Some(FfiPlaylistFrame::End)) => panic!("failed playlist ended cleanly"),
                Ok(None) => {}
                Err(error) => break error,
            }
        };

        assert!(audio_frames > 0, "the valid current track must be emitted");
        assert!(
            !saw_boundary,
            "a failed source must not be announced as current"
        );
        assert!(error.to_string().contains("Unavailable song"));
        let _ = stream.cancel();
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn unsupported_upcoming_source_is_an_item_specific_failure() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-unsupported-playlist-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let current = root.join("current.wav");
        let unsupported = root.join("unsupported.ogg");
        fs::write(&current, wav_pcm16(48_000, 2, 241, 3_000)).expect("current source");
        fs::write(&unsupported, b"OggSunsupported playlist fixture").expect("unsupported source");

        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-unsupported".to_owned(),
            "stream-ffi-unsupported".to_owned(),
            5_000,
            vec![
                FfiPlaylistSource {
                    entry_id: "entry-current".to_owned(),
                    display_name: "Current song".to_owned(),
                    staged_path: current.to_string_lossy().into_owned(),
                },
                FfiPlaylistSource {
                    entry_id: "entry-unsupported".to_owned(),
                    display_name: "Unsupported song".to_owned(),
                    staged_path: unsupported.to_string_lossy().into_owned(),
                },
            ],
        )
        .expect("start playlist with lazy upcoming source");

        let mut audio_frames = 0;
        let error = loop {
            match stream.next_frame(2_000) {
                Ok(Some(FfiPlaylistFrame::Audio { .. })) => audio_frames += 1,
                Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                    panic!("unsupported source must not be announced as current")
                }
                Ok(Some(FfiPlaylistFrame::End)) => panic!("failed playlist ended cleanly"),
                Ok(None) => {}
                Err(error) => break error,
            }
        };

        assert!(audio_frames > 0, "the valid current track must be emitted");
        assert!(error.to_string().contains("Unsupported song"));
        let _ = stream.cancel();
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn decoder_read_failure_after_open_is_reported_instead_of_clean_queue_end() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-late-decode-failure-{unique}"));
        fs::create_dir_all(&root).expect("temp directory");
        let current = root.join("current.wav");
        // Keep enough audio beyond the decoder's bounded read-ahead window
        // that truncation after the first delivered frame forces a later read.
        fs::write(&current, wav_pcm16(48_000, 2, 480_000, 3_000)).expect("current source");
        let stream = FfiHostPlaylistStreamer::open(
            "session-ffi-late-failure".to_owned(),
            "stream-ffi-late-failure".to_owned(),
            5_000,
            vec![FfiPlaylistSource {
                entry_id: "entry-current".to_owned(),
                display_name: "Current song".to_owned(),
                staged_path: current.to_string_lossy().into_owned(),
            }],
        )
        .expect("source opens before the read failure");

        let first = stream.next_frame(2_000).expect("first frame result");
        assert!(matches!(first, Some(FfiPlaylistFrame::Audio { .. })));
        fs::OpenOptions::new()
            .write(true)
            .open(&current)
            .expect("open active source for truncation")
            .set_len(44)
            .expect("truncate active source after playback begins");

        let failure = loop {
            match stream.next_frame(2_000) {
                Ok(Some(FfiPlaylistFrame::Audio { .. }) | None) => {}
                Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                    panic!("failed active source must not announce a later track")
                }
                Ok(Some(FfiPlaylistFrame::End)) => {
                    panic!("decoder read failure must not look like clean queue completion")
                }
                Err(error) => break error,
            }
        };
        assert!(failure.to_string().contains("Current song"));
        let _ = stream.cancel();
        fs::remove_dir_all(root).expect("remove temp directory");
    }

    fn wav_pcm16(sample_rate: u32, channels: u16, frames: u32, sample: i16) -> Vec<u8> {
        let data_bytes = frames * u32::from(channels) * 2;
        let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for _ in 0..frames * u32::from(channels) {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }
}
