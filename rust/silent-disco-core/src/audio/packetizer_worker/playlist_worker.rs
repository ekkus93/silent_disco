//! Continuous playlist packetization, lookahead, and live retargeting.

use super::super::decoder::StreamingDecodeHandle;
use super::super::packetizer::Packetizer;
use super::super::types::{DecodeError, DecodeErrorKind};
use super::{
    PacketizerSummary, PacketizerWorkerError, PacketizerWorkerErrorKind, PacketizerWorkerState,
    PlaylistUpdates, SharedStatistics, StagedPlaylistTrack, StreamingPacketizeConfig,
    StreamingPlaylistTrackFactory, WORKER_POLL_INTERVAL, ensure_not_cancelled,
    merge_decoder_shutdown_result, send_with_backpressure,
};
use crate::protocol::{ControlMessage, MAX_DISPLAY_NAME_BYTES, ProtocolFrame, TrackBoundary};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};

pub(super) fn validate_track_identity(
    entry_id: &str,
    display_name: &str,
) -> Result<(), PacketizerWorkerError> {
    if entry_id.is_empty()
        || entry_id.len() > crate::protocol::MAX_PROTOCOL_STRING_BYTES
        || display_name.is_empty()
        || display_name.len() > MAX_DISPLAY_NAME_BYTES
    {
        return Err(PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::Packetize,
            "playlist track identity exceeds protocol bounds",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_playlist_packetizer_worker(
    mut decoder: StreamingDecodeHandle,
    _current_entry_id: String,
    mut current_display_name: String,
    upcoming: Vec<StreamingPlaylistTrackFactory>,
    updates: &PlaylistUpdates,
    mut packetizer: Packetizer,
    sender: &SyncSender<ProtocolFrame>,
    config: StreamingPacketizeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
) -> Result<PacketizerSummary, PacketizerWorkerError> {
    let mut upcoming: VecDeque<_> = upcoming.into();
    let mut boundary = None;
    loop {
        ensure_not_cancelled(cancellation)?;
        apply_playlist_update(updates, &mut upcoming)?;
        let mut next = open_next_track(&mut upcoming);
        let has_ready_successor = next.as_ref().is_some_and(|(_, opened)| opened.is_ok());
        let track_result = packetize_playlist_track(
            &decoder,
            &mut packetizer,
            sender,
            config,
            cancellation,
            statistics,
            !has_ready_successor,
            boundary.take(),
            updates,
            &mut upcoming,
            &mut next,
        );
        let decoder_result = if track_result.is_err() {
            decoder.cancel_and_join()
        } else {
            decoder.join()
        };
        if let Err(error) = merge_decoder_shutdown_result(track_result, decoder_result) {
            return Err(PacketizerWorkerError::new(
                error.kind,
                format!("{current_display_name}: {}", error.message),
            ));
        }

        let Some(((next_entry_id, next_display_name), next_decoder)) = next else {
            return Ok(PacketizerSummary {
                state: PacketizerWorkerState::Completed,
                emitted_packets: statistics.emitted_packets.load(Ordering::Acquire),
                backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
            });
        };
        decoder = match next_decoder {
            Ok(decoder) => decoder,
            Err(error) => {
                return Err(map_playlist_decode_error(&error, &next_display_name));
            }
        };
        current_display_name.clone_from(&next_display_name);
        boundary = Some(ProtocolFrame::Control(ControlMessage::TrackBoundary(
            TrackBoundary {
                session_id: packetizer.session_id().clone(),
                stream_id: packetizer.stream_id().clone(),
                first_sample_index: crate::domain::SampleIndex::new(
                    packetizer.next_contiguous_sample_index(),
                ),
                entry_id: next_entry_id.clone(),
                display_name: next_display_name.clone(),
            },
        )));
    }
}

fn apply_playlist_update(
    updates: &Mutex<Option<Vec<StreamingPlaylistTrackFactory>>>,
    upcoming: &mut VecDeque<StreamingPlaylistTrackFactory>,
) -> Result<(), PacketizerWorkerError> {
    let replacement = updates
        .lock()
        .map_err(|_| {
            PacketizerWorkerError::new(
                PacketizerWorkerErrorKind::WorkerPanicked,
                "playlist update lock was poisoned",
            )
        })?
        .take();
    if let Some(replacement) = replacement {
        *upcoming = replacement.into();
    }
    Ok(())
}

fn open_next_track(
    upcoming: &mut VecDeque<StreamingPlaylistTrackFactory>,
) -> Option<StagedPlaylistTrack> {
    upcoming.pop_front().map(|factory| {
        let identity = (factory.entry_id, factory.display_name);
        (identity, (factory.open)())
    })
}

#[allow(clippy::too_many_arguments)]
fn packetize_playlist_track(
    decoder: &StreamingDecodeHandle,
    packetizer: &mut Packetizer,
    sender: &SyncSender<ProtocolFrame>,
    config: StreamingPacketizeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
    mut final_track: bool,
    boundary: Option<ProtocolFrame>,
    updates: &Mutex<Option<Vec<StreamingPlaylistTrackFactory>>>,
    upcoming: &mut VecDeque<StreamingPlaylistTrackFactory>,
    next: &mut Option<StagedPlaylistTrack>,
) -> Result<PacketizerSummary, PacketizerWorkerError> {
    let mut pending_boundary = boundary;
    loop {
        ensure_not_cancelled(cancellation)?;
        refresh_pending_playlist_update(updates, upcoming, next, &mut final_track)?;
        let chunk = match decoder.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => {
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::Source,
                    "playlist decoder disconnected before its final chunk",
                ));
            }
        };
        let input_finished = chunk.end_of_stream;
        if let Some(boundary) = pending_boundary.take() {
            send_with_backpressure(
                sender,
                boundary,
                cancellation,
                statistics,
                config.queue_capacity,
            )?;
        }
        let mut chunk = chunk;
        chunk.end_of_stream = input_finished && final_track;
        let outcome = packetizer.push_contiguous_chunk(chunk)?;
        for frame in outcome.frames {
            send_with_backpressure(
                sender,
                frame,
                cancellation,
                statistics,
                config.queue_capacity,
            )?;
        }
        if input_finished {
            if final_track && !outcome.end_of_stream {
                return Err(PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::Packetize,
                    "final playlist track did not finish the packetizer stream",
                ));
            }
            return Ok(PacketizerSummary {
                state: PacketizerWorkerState::Completed,
                emitted_packets: statistics.emitted_packets.load(Ordering::Acquire),
                backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
            });
        }
    }
}

fn refresh_pending_playlist_update(
    updates: &Mutex<Option<Vec<StreamingPlaylistTrackFactory>>>,
    upcoming: &mut VecDeque<StreamingPlaylistTrackFactory>,
    next: &mut Option<StagedPlaylistTrack>,
    final_track: &mut bool,
) -> Result<(), PacketizerWorkerError> {
    let has_update = updates
        .lock()
        .map_err(|_| {
            PacketizerWorkerError::new(
                PacketizerWorkerErrorKind::WorkerPanicked,
                "playlist update lock was poisoned",
            )
        })?
        .is_some();
    if has_update {
        apply_playlist_update(updates, upcoming)?;
        if let Some((_, Ok(stale))) = next.take() {
            let _ = stale.cancel_and_join();
        }
        *next = open_next_track(upcoming);
        *final_track = next.is_none();
    }
    Ok(())
}

fn map_playlist_decode_error(error: &DecodeError, display_name: &str) -> PacketizerWorkerError {
    let kind = if error.kind == DecodeErrorKind::WorkerPanicked {
        PacketizerWorkerErrorKind::WorkerPanicked
    } else if error.kind == DecodeErrorKind::Cancelled {
        PacketizerWorkerErrorKind::Cancelled
    } else {
        PacketizerWorkerErrorKind::Source
    };
    PacketizerWorkerError::new(
        kind,
        format!("{display_name}: streaming playlist decoder failed: {error}"),
    )
}
