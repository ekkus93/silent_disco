//! Continuous playlist packetization, lookahead, and live retargeting.

use super::super::decoder::StreamingDecodeHandle;
use super::super::packetizer::Packetizer;
use super::super::types::DecodedPcmChunk;
use crate::domain::SampleIndex;
use super::super::types::{DecodeError, DecodeErrorKind};
use super::{
    PacketizerSummary, PacketizerWorkerError, PacketizerWorkerErrorKind, PacketizerWorkerState,
    PlaylistUpdateState, PlaylistUpdates, SharedStatistics, StagedPlaylistTrack,
    StreamingPacketizeConfig, StreamingPlaylistTrackFactory, WORKER_POLL_INTERVAL,
    ensure_not_cancelled,
    merge_decoder_shutdown_result, send_with_backpressure,
};
use crate::protocol::{ControlMessage, MAX_DISPLAY_NAME_BYTES, ProtocolFrame, TrackBoundary};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};

/// Marks the updater terminal under the same lock used to admit queue edits.
/// A producer that has exited must not silently accept work it cannot emit.
struct PlaylistTerminalGuard<'a>(&'a PlaylistUpdates);

impl Drop for PlaylistTerminalGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.terminal = true;
            state.pending = None;
        }
    }
}

pub(super) fn validate_track_identity(
    entry_id: &str,
    display_name: &str,
) -> Result<(), PacketizerWorkerError> {
    if entry_id.is_empty()
        || entry_id.len() > crate::runtime::MAX_QUEUE_ENTRY_ID_BYTES
        || entry_id.len() > crate::protocol::MAX_PROTOCOL_STRING_BYTES
        || entry_id.trim() != entry_id
        || entry_id.chars().any(char::is_control)
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

/// Enforce the actor's queue bounds even for direct core/UniFFI packetizer
/// callers that bypass HostQueue. A live updater reserves one current slot,
/// although its current entry ID is owned by the running worker.
pub(super) fn validate_playlist_entries(
    current_entry_id: Option<&str>,
    upcoming: &[StreamingPlaylistTrackFactory],
) -> Result<(), PacketizerWorkerError> {
    if upcoming.len() >= crate::runtime::MAX_HOST_QUEUE_ENTRIES {
        return Err(PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::Packetize,
            format!(
                "playlist exceeds the {}-entry queue limit",
                crate::runtime::MAX_HOST_QUEUE_ENTRIES
            ),
        ));
    }
    let mut ids = HashSet::with_capacity(upcoming.len() + 1);
    if let Some(current) = current_entry_id {
        ids.insert(current);
    }
    for track in upcoming {
        if !ids.insert(track.entry_id.as_str()) {
            return Err(PacketizerWorkerError::new(
                PacketizerWorkerErrorKind::Packetize,
                "playlist contains duplicate entry IDs",
            ));
        }
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
    let _terminal_guard = PlaylistTerminalGuard(updates);
    let mut upcoming: VecDeque<_> = upcoming.into();
    let mut boundary = None;
    loop {
        ensure_not_cancelled(cancellation)?;
        apply_playlist_update(updates, &mut upcoming)?;
        let mut next = open_next_track(&mut upcoming);
        let has_ready_successor = next.as_ref().is_some_and(|(_, opened)| opened.is_ok());
        let outgoing_format = decoder.stream_info().format;
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

        // One last latest-value read closes the EOF-to-promotion window:
        // a host may have accepted a new upcoming order while the final
        // decoded chunk was being sent under backpressure. Do not promote
        // a decoder selected from the old order if an update is pending.
        let had_successor = next.is_some();
        if had_successor {
            let mut final_track = false;
            refresh_pending_playlist_update(
                updates,
                &mut upcoming,
                &mut next,
                &mut final_track,
            )?;
        }
        let Some(((next_entry_id, next_display_name), next_decoder)) = next else {
            if had_successor {
                // The outgoing decoder was processed as non-final, so its
                // final partial packet remains in the packetizer carry.
                // Finalize *without adding samples* at this new queue end.
                if packetizer.is_finished() {
                    return Err(PacketizerWorkerError::new(
                        PacketizerWorkerErrorKind::Packetize,
                        "cannot retarget a playlist whose packetizer already finished",
                    ));
                }
                let outcome = packetizer.push_contiguous_chunk(DecodedPcmChunk {
                    format: outgoing_format,
                    first_sample_index: SampleIndex::new(0),
                    frames: Vec::new(),
                    end_of_stream: true,
                })?;
                for frame in outcome.frames {
                    send_with_backpressure(
                        sender,
                        frame,
                        cancellation,
                        statistics,
                        config.queue_capacity,
                    )?;
                }
                return Ok(PacketizerSummary {
                    state: PacketizerWorkerState::Completed,
                    emitted_packets: statistics.emitted_packets.load(Ordering::Acquire),
                    backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
                });
            }
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
        promote_current_entry(updates, &next_entry_id)?;
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
    updates: &Mutex<PlaylistUpdateState>,
    upcoming: &mut VecDeque<StreamingPlaylistTrackFactory>,
) -> Result<(), PacketizerWorkerError> {
    let mut state = updates.lock().map_err(|_| {
        PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::WorkerPanicked,
            "playlist update lock was poisoned",
        )
    })?;
    if let Some(replacement) = state.pending.take() {
        // Promotion may have happened since the caller submitted this edit.
        super::validate_playlist_entries(Some(&state.current_entry_id), &replacement)?;
        *upcoming = replacement.into();
    }
    Ok(())
}

/// If a pending edit was accepted just before promotion, the newly current
/// entry is no longer upcoming. Keep the remaining order without replaying it.
fn promote_current_entry(
    updates: &Mutex<PlaylistUpdateState>,
    entry_id: &str,
) -> Result<(), PacketizerWorkerError> {
    let mut state = updates.lock().map_err(|_| {
        PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::WorkerPanicked,
            "playlist update lock was poisoned",
        )
    })?;
    state.current_entry_id.clear();
    state.current_entry_id.push_str(entry_id);
    if let Some(pending) = state.pending.as_mut() {
        pending.retain(|track| track.entry_id != entry_id);
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
    updates: &Mutex<PlaylistUpdateState>,
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
    updates: &Mutex<PlaylistUpdateState>,
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
        .pending
        .is_some();
    if has_update {
        apply_playlist_update(updates, upcoming)?;
        // Appending or reordering tracks *behind* the already-prepared
        // successor must not tear down and reopen that decoder. Besides
        // wasting the prebuffer, reopening can introduce a boundary underrun
        // or fail after a staged file becomes temporarily unavailable.
        if next.as_ref().is_some_and(|((id, name), opened)| {
            opened.is_ok() && upcoming.front().is_some_and(|factory| {
                same_staged_identity(id, name, factory)
            })
        }) {
            upcoming.pop_front();
        } else {
            if let Some((_, Ok(stale))) = next.take() {
                let _ = stale.cancel_and_join();
            }
            *next = open_next_track(upcoming);
        }
        *final_track = next.is_none();
    }
    Ok(())
}

fn same_staged_identity(
    entry_id: &str,
    display_name: &str,
    candidate: &StreamingPlaylistTrackFactory,
) -> bool {
    entry_id == candidate.entry_id && display_name == candidate.display_name
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

#[cfg(test)]
mod promotion_identity_tests {
    use super::*;

    fn factory(id: &str) -> StreamingPlaylistTrackFactory {
        StreamingPlaylistTrackFactory::new(id, "Track", || {
            unreachable!("identity-only test never opens a decoder")
        })
        .expect("factory")
    }

    #[test]
    fn unchanged_successor_identity_preserves_the_prebuffer() {
        let same = factory("next");
        let changed = factory("replacement");
        assert!(same_staged_identity("next", "Track", &same));
        assert!(!same_staged_identity("next", "Track", &changed));
        assert!(!same_staged_identity("next", "Renamed track", &same));
    }

    #[test]
    fn promotion_removes_only_new_current_entry_from_pending_edit() {
        let state = Mutex::new(PlaylistUpdateState {
            current_entry_id: "one".to_owned(),
            pending: Some(vec![factory("two"), factory("three")]),
            terminal: false,
        });
        promote_current_entry(&state, "two").expect("promotion");
        let mut upcoming = VecDeque::new();
        apply_playlist_update(&state, &mut upcoming).expect("apply accepted edit");
        assert_eq!(state.lock().expect("lock").current_entry_id, "two");
        assert_eq!(
            upcoming
                .iter()
                .map(|track| track.entry_id.as_str())
                .collect::<Vec<_>>(),
            ["three"]
        );
    }
}
