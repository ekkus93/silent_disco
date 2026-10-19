//! Continuous playlist packetization, lookahead, and live retargeting.

use super::super::decoder::StreamingDecodeHandle;
use super::super::packetizer::Packetizer;
use super::super::types::DecodedPcmChunk;
use super::super::types::{DecodeError, DecodeErrorKind};
use super::{
    BoundaryReserver, PacketizerSummary, PacketizerWorkerError, PacketizerWorkerErrorKind,
    PacketizerWorkerState, PlaylistUpdateState, PlaylistUpdates, SharedStatistics,
    StagedPlaylistTrack, StreamingPacketizeConfig, StreamingPlaylistTrackFactory,
    WORKER_POLL_INTERVAL, ensure_not_cancelled, merge_decoder_shutdown_result,
    send_with_backpressure,
};
use crate::domain::SampleIndex;
use crate::protocol::{ControlMessage, MAX_DISPLAY_NAME_BYTES, ProtocolFrame, TrackBoundary};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};

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
/// callers that bypass `HostQueue`. A live updater reserves one current slot,
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

/// The shared terminal result for either natural queue-end path.
fn playlist_completed_summary(statistics: &SharedStatistics) -> PacketizerSummary {
    PacketizerSummary {
        state: PacketizerWorkerState::Completed,
        emitted_packets: statistics.emitted_packets.load(Ordering::Acquire),
        backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
    }
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
    cancellation: &Arc<AtomicBool>,
    statistics: &SharedStatistics,
    boundary_reserver: Option<&BoundaryReserver>,
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
            refresh_pending_playlist_update(updates, &mut upcoming, &mut next, &mut final_track)?;
        }
        // Commit only after consuming every accepted edit. The check and
        // promotion share the updater mutex, preventing a lost retarget.
        while next.as_ref().is_some_and(|(_, opened)| opened.is_ok()) {
            let id = &next.as_ref().expect("checked").0.0;
            if promote_current_entry_if_unmodified(updates, id, boundary_reserver.is_some())? {
                break;
            }
            let mut final_track = false;
            refresh_pending_playlist_update(updates, &mut upcoming, &mut next, &mut final_track)?;
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
                return Ok(playlist_completed_summary(statistics));
            }
            return Ok(playlist_completed_summary(statistics));
        };
        decoder = match next_decoder {
            Ok(decoder) => decoder,
            Err(error) => {
                return Err(map_playlist_decode_error(&error, &next_display_name));
            }
        };
        let committed_boundary = TrackBoundary {
            session_id: packetizer.session_id().clone(),
            stream_id: packetizer.stream_id().clone(),
            first_sample_index: crate::domain::SampleIndex::new(
                packetizer.next_contiguous_sample_index(),
            ),
            entry_id: next_entry_id.clone(),
            display_name: next_display_name.clone(),
        };
        // The actor must first lock this target against competing queue edits.
        // No worker update lock or bounded output-channel slot is held here.
        if let Some(reserve) = boundary_reserver
            && let Err(error) = reserve(&committed_boundary, Arc::clone(cancellation))
        {
            // The successor decoder is already opened and prebuffering.
            // Rejecting its boundary must cancel/join that worker rather
            // than dropping a live decoder after failing closed.
            return merge_decoder_shutdown_result(Err(error), decoder.cancel_and_join());
        }
        current_display_name.clone_from(&next_display_name);
        boundary = Some(ProtocolFrame::Control(ControlMessage::TrackBoundary(
            committed_boundary,
        )));
    }
}

/// Atomically close the updater only if no previously accepted edit remains.
/// A pending append wins the EOF race and must be retargeted before the
/// packetizer is allowed to emit its final frame.
fn seal_final_track_if_unmodified(
    updates: &Mutex<PlaylistUpdateState>,
) -> Result<bool, PacketizerWorkerError> {
    let mut state = updates.lock().map_err(|_| {
        PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::WorkerPanicked,
            "playlist update lock was poisoned",
        )
    })?;
    if state.pending.is_some() {
        return Ok(false);
    }
    state.terminal = true;
    Ok(true)
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

/// Returns false if a pending accepted edit must be applied first.
fn promote_current_entry_if_unmodified(
    updates: &Mutex<PlaylistUpdateState>,
    entry_id: &str,
    reserved: bool,
) -> Result<bool, PacketizerWorkerError> {
    let mut state = updates.lock().map_err(|_| {
        PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::WorkerPanicked,
            "playlist update lock was poisoned",
        )
    })?;
    if state.pending.is_some() {
        return Ok(false);
    }
    state.current_entry_id.clear();
    state.current_entry_id.push_str(entry_id);
    state.reservation_shadow_entry_id = reserved.then(|| entry_id.to_owned());
    Ok(true)
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
        // The final decoded chunk is the last chance to accept an append
        // without restarting the stream. Atomically seal the updater before
        // marking the packetizer finished: edits accepted before the seal
        // are applied, and edits arriving afterwards receive Closed instead
        // of being silently lost between EOF and worker termination.
        if input_finished && final_track {
            loop {
                if seal_final_track_if_unmodified(updates)? {
                    break;
                }
                refresh_pending_playlist_update(updates, upcoming, next, &mut final_track)?;
                if !final_track {
                    break;
                }
            }
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
            return Ok(playlist_completed_summary(statistics));
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
            opened.is_ok()
                && upcoming
                    .front()
                    .is_some_and(|factory| same_staged_identity(id, name, factory))
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
    use std::sync::Arc;

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
    fn accepted_append_wins_before_final_eof_seal() {
        let state = Mutex::new(PlaylistUpdateState {
            current_entry_id: "one".to_owned(),
            reservation_shadow_entry_id: None,
            pending: Some(vec![factory("appended")]),
            terminal: false,
        });
        assert!(!seal_final_track_if_unmodified(&state).expect("pending edit wins"));
        assert!(!state.lock().unwrap().terminal);
        let mut upcoming = VecDeque::new();
        apply_playlist_update(&state, &mut upcoming).expect("consume accepted append");
        assert_eq!(upcoming.front().unwrap().entry_id, "appended");
        // A live successor means the current track must not be sealed.
        assert!(!state.lock().unwrap().terminal);
    }

    #[test]
    fn final_eof_seal_rejects_late_edits_before_worker_exit() {
        let state = Arc::new(Mutex::new(PlaylistUpdateState {
            current_entry_id: "one".to_owned(),
            reservation_shadow_entry_id: None,
            pending: None,
            terminal: false,
        }));
        assert!(seal_final_track_if_unmodified(&state).expect("seal EOF"));
        let updater = super::super::StreamingPlaylistUpdater { pending: state };
        let error = updater
            .replace_upcoming(vec![factory("too-late")])
            .expect_err("completed packetizer cannot acknowledge an edit");
        assert_eq!(error.kind, PacketizerWorkerErrorKind::Cancelled);
    }

    #[test]
    fn concurrent_final_eof_seal_never_loses_an_accepted_append() {
        use std::sync::Barrier;
        use std::thread;

        // Exercise the real updater and producer admission functions under
        // contention, rather than asserting only a sequential ordering.
        // Exactly one operation wins: an accepted edit stays pending for the
        // worker, or a sealed EOF rejects the edit as terminal.
        for _ in 0..32 {
            let state = Arc::new(Mutex::new(PlaylistUpdateState {
                current_entry_id: "one".to_owned(),
                reservation_shadow_entry_id: None,
                pending: None,
                terminal: false,
            }));
            let gate = Arc::new(Barrier::new(3));
            let seal_state = Arc::clone(&state);
            let seal_gate = Arc::clone(&gate);
            let sealer = thread::spawn(move || {
                seal_gate.wait();
                seal_final_track_if_unmodified(&seal_state).expect("seal")
            });
            let updater = super::super::StreamingPlaylistUpdater {
                pending: Arc::clone(&state),
            };
            let edit_gate = Arc::clone(&gate);
            let editor = thread::spawn(move || {
                edit_gate.wait();
                updater.replace_upcoming(vec![factory("appended")])
            });
            gate.wait();
            let did_seal = sealer.join().expect("sealer thread");
            let accepted = editor.join().expect("editor thread").is_ok();
            let final_state = state.lock().expect("state lock");
            assert_ne!(
                did_seal, accepted,
                "EOF and an accepted append cannot both win or both lose"
            );
            if accepted {
                assert!(!final_state.terminal);
                assert_eq!(
                    final_state.pending.as_ref().unwrap()[0].entry_id,
                    "appended"
                );
            } else {
                assert!(final_state.terminal);
                assert!(final_state.pending.is_none());
            }
        }
    }

    #[test]
    fn reserved_shadow_retargets_later_entries_without_permitting_duplicates() {
        let state = Arc::new(Mutex::new(PlaylistUpdateState {
            current_entry_id: "reserved".to_owned(),
            reservation_shadow_entry_id: Some("reserved".to_owned()),
            pending: None,
            terminal: false,
        }));
        let updater = super::super::StreamingPlaylistUpdater {
            pending: Arc::clone(&state),
        };
        updater
            .replace_upcoming(vec![factory("reserved"), factory("replacement")])
            .expect("leading producer-reserved ID is a shadow, not a new entry");
        let state_lock = state.lock().expect("state");
        assert_eq!(
            state_lock
                .pending
                .as_ref()
                .expect("pending")
                .iter()
                .map(|entry| entry.entry_id.as_str())
                .collect::<Vec<_>>(),
            ["replacement"]
        );
        drop(state_lock);
        for invalid in [
            vec![factory("reserved"), factory("reserved")],
            vec![factory("replacement"), factory("reserved")],
        ] {
            assert_eq!(
                updater
                    .replace_upcoming(invalid)
                    .expect_err("duplicate current ID")
                    .kind,
                PacketizerWorkerErrorKind::Packetize
            );
        }
    }

    #[test]
    fn accepted_retarget_wins_before_promotion() {
        let state = Mutex::new(PlaylistUpdateState {
            current_entry_id: "one".to_owned(),
            reservation_shadow_entry_id: None,
            pending: Some(vec![factory("replacement"), factory("three")]),
            terminal: false,
        });
        assert!(!promote_current_entry_if_unmodified(&state, "two", false).expect("pending edit"));
        assert_eq!(state.lock().expect("lock").current_entry_id, "one");
        let mut upcoming = VecDeque::new();
        apply_playlist_update(&state, &mut upcoming).expect("apply accepted edit");
        assert!(
            promote_current_entry_if_unmodified(&state, "replacement", false).expect("promotion")
        );
        assert_eq!(state.lock().expect("lock").current_entry_id, "replacement");
        assert_eq!(
            upcoming
                .iter()
                .map(|track| track.entry_id.as_str())
                .collect::<Vec<_>>(),
            ["replacement", "three"]
        );
    }
}
