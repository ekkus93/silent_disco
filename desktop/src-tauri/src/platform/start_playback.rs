//! Orchestrates one `start_playback` command: resolves the staged source,
//! opens the shared decoder, spawns the shared packetizer, and hands the
//! result to [`super::network::DesktopHostNetworkControl::start_playback`].

use super::audio_decode::prepare_staged_audio_source;
use super::file_picker::SelectedSourceRegistry;
use super::network::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use silent_disco_core::audio::{
    DecodeError, PacketizerWorkerError, PacketizerWorkerErrorKind, StreamingDecodeConfig,
    StreamingDecodeHandle, StreamingPacketizeConfig, StreamingPacketizeHandle,
    StreamingPlaylistTrackFactory,
};
use silent_disco_core::domain::{MonotonicMillis, PlaybackState, StreamId};
use silent_disco_core::protocol::TrackBoundary;
use silent_disco_core::runtime::{AudioEvent, CoreActorHandle, QueueEntry};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Startup presentation buffer before the first audio sample plays, matching
/// this project's ~400ms baseline (see `CLAUDE.md`).
const STARTUP_BUFFER_MS: u64 = 400;

/// Resolves the current host draft's audio source, opens the shared
/// decoder, spawns the shared packetizer, and starts real-time delivery.
///
/// # Errors
///
/// Returns a structured error when no audio source is selected, the source
/// is no longer staged, decode/packetize preparation fails, or the actor
/// rejects the resulting `Playing` transition. On any failure after the
/// `Buffering` transition, reports `PlaybackState::Error` to the actor so
/// the failure is visible rather than leaving playback silently stuck.
pub(crate) fn start(
    handle: &CoreActorHandle,
    network: &Arc<DesktopHostNetworkControl>,
    registry: &SelectedSourceRegistry,
) -> Result<(), DesktopErrorDto> {
    let snapshot = handle.current_snapshot().map_err(DesktopErrorDto::from)?;
    let descriptor = snapshot.host_draft.audio_source.clone().ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.playback.no_source_selected",
            "audio",
            "error",
            false,
            "no audio source is selected for this host session",
        )
    })?;
    let queue_entries = if snapshot.host_queue.entries().is_empty() {
        vec![
            QueueEntry::new("single-track", descriptor.clone()).map_err(|error| {
                DesktopErrorDto::new(
                    "desktop.playback.invalid_queue_entry",
                    "audio",
                    "error",
                    false,
                    &error.to_string(),
                )
            })?,
        ]
    } else {
        snapshot.host_queue.entries().to_vec()
    };
    let active = network.active_host_session()?.ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.playback.no_active_session",
            "audio",
            "error",
            true,
            "starting playback requires an active desktop host session",
        )
    })?;
    let session_id = active.advertisement.session_id.clone();

    // A duplicate/stale Start command must be rejected without touching
    // the actor at all: submitting `Buffering` (below) unconditionally and
    // only discovering the duplicate deep inside `network.start_playback`
    // would still corrupt the authoritative snapshot to `Error`, even
    // though the real, already-running stream is unaffected.
    if network.playback_is_active()? {
        return Err(DesktopErrorDto::new(
            "desktop.playback.already_active",
            "audio",
            "error",
            true,
            "playback is already active for this host session",
        ));
    }

    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
        .map_err(DesktopErrorDto::from)?;

    match start_after_buffering(handle, network, registry, &queue_entries, session_id) {
        Ok(()) => Ok(()),
        Err(error) => {
            let state_error = handle
                .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Error))
                .map_err(DesktopErrorDto::from)
                .err();
            Err(error.with_appended_cleanup(state_error))
        }
    }
}

fn start_after_buffering(
    handle: &CoreActorHandle,
    network: &Arc<DesktopHostNetworkControl>,
    registry: &SelectedSourceRegistry,
    queue_entries: &[QueueEntry],
    session_id: silent_disco_core::domain::SessionId,
) -> Result<(), DesktopErrorDto> {
    let current = queue_entries.first().ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.playback.no_source_selected",
            "audio",
            "error",
            false,
            "no audio source is queued for this host session",
        )
    })?;
    let prepared =
        prepare_staged_audio_source(registry, &current.source, StreamingDecodeConfig::default())?;
    let upcoming = prepare_upcoming_sources(registry, queue_entries)?;
    let host_start_time_ms =
        MonotonicMillis::new(network.transport_now()?.get() + STARTUP_BUFFER_MS);
    let stream_id =
        StreamId::new(format!("desktop-stream-{}", host_start_time_ms.get())).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.playback.invalid_stream_id",
                "audio",
                "error",
                false,
                &error.to_string(),
            )
        })?;
    let decoder = prepared.into_decoder();
    // Taken before the decoder is consumed by the packetizer worker below --
    // Block 35.1 "decoder/source queues" diagnostics need a live reader
    // independent of the handle's own ownership, which the packetizer
    // worker exclusively holds from this point on.
    let decode_diagnostics = decoder.statistics_reader();
    // The producer must reserve each boundary on the same serialized actor
    // that admits host queue edits. Unlike ordinary submit_audio_event, the
    // reservation waits for explicit actor application before successor PCM
    // can enter the packetizer's bounded output channel.
    let authority = handle.clone();
    let reserve = Arc::new(
        move |boundary: &TrackBoundary, cancelled: Arc<AtomicBool>| {
            // A short first track may finish decoding while the host is still
            // binding its stream identity. Wait without holding any decoder or
            // actor locks; cancellation must also unblock an aborted start.
            loop {
                // An aborted startup must never wait on the actor for a stream
                // that can no longer transition into Playing.
                if cancelled.load(Ordering::Acquire) {
                    return Err(PacketizerWorkerError {
                        kind: PacketizerWorkerErrorKind::Cancelled,
                        message: "boundary reservation cancelled during playback startup".into(),
                    });
                }
                let state = authority
                    .current_snapshot()
                    .map_err(|error| reservation_error(&error.to_string(), &cancelled))?
                    .playback_state;
                match state {
                    // A producer reservation also remains valid across a
                    // host pause; the actor still forbids application until
                    // playback resumes.
                    PlaybackState::Playing | PlaybackState::Paused => break,
                    PlaybackState::Stopped | PlaybackState::Error => {
                        return Err(reservation_error(
                            "host playback stopped before boundary reservation",
                            &cancelled,
                        ));
                    }
                    _ => std::thread::sleep(Duration::from_millis(20)),
                }
            }
            authority
                .reserve_host_track_boundary(
                    boundary.stream_id.clone(),
                    boundary.entry_id.clone(),
                    boundary.first_sample_index.get(),
                    Arc::clone(&cancelled),
                )
                .map_err(|error| reservation_error(&error.to_string(), &cancelled))
        },
    );
    let packetizer = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        current.entry_id.clone(),
        current.source.display_name.clone(),
        upcoming,
        session_id.clone(),
        stream_id.clone(),
        host_start_time_ms,
        StreamingPacketizeConfig::default(),
        reserve,
    )
    .map_err(|error| map_packetizer_error(&error))?;
    // Same reasoning: taken before the packetizer is moved into its own
    // real-time pump thread.
    let packetize_diagnostics = packetizer.statistics_reader();
    network.start_playback(
        packetizer,
        session_id,
        stream_id,
        handle.clone(),
        decode_diagnostics,
        packetize_diagnostics,
    )
}

fn prepare_upcoming_sources(
    registry: &SelectedSourceRegistry,
    queue_entries: &[QueueEntry],
) -> Result<Vec<StreamingPlaylistTrackFactory>, DesktopErrorDto> {
    let mut upcoming = Vec::with_capacity(queue_entries.len().saturating_sub(1));
    for entry in queue_entries.iter().skip(1) {
        let source = registry
            .resolve(&entry.source.source_id)?
            .filter(|source| source.descriptor() == &entry.source)
            .ok_or_else(|| {
                DesktopErrorDto::new(
                    "desktop.playback.queued_source_not_staged",
                    "audio",
                    "error",
                    true,
                    &format!(
                        "{} is no longer staged in this desktop profile",
                        entry.source.display_name
                    ),
                )
            })?;
        let path = source.canonical_path().to_owned();
        let display_name = entry.source.display_name.clone();
        let factory = StreamingPlaylistTrackFactory::new(
            entry.entry_id.clone(),
            display_name.clone(),
            move || {
                StreamingDecodeHandle::open(path, StreamingDecodeConfig::default()).map_err(
                    |error| DecodeError {
                        kind: error.kind,
                        message: format!("{display_name}: {}", error.message),
                    },
                )
            },
        )
        .map_err(|error| map_packetizer_error(&error))?;
        upcoming.push(factory);
    }
    Ok(upcoming)
}

fn map_packetizer_error(error: &PacketizerWorkerError) -> DesktopErrorDto {
    let code = match error.kind {
        PacketizerWorkerErrorKind::Source => "desktop.playback.decode_failed",
        PacketizerWorkerErrorKind::Packetize => "desktop.playback.packetize_failed",
        PacketizerWorkerErrorKind::Cancelled => "desktop.playback.cancelled",
        PacketizerWorkerErrorKind::WorkerPanicked => "desktop.playback.worker_failed",
    };
    DesktopErrorDto::new(code, "audio", "error", true, &error.message)
}

fn reservation_error(message: &str, cancelled: &AtomicBool) -> PacketizerWorkerError {
    let cancelled = cancelled.load(Ordering::Acquire);
    PacketizerWorkerError {
        kind: if cancelled {
            PacketizerWorkerErrorKind::Cancelled
        } else {
            PacketizerWorkerErrorKind::Packetize
        },
        message: format!(
            "authoritative playlist boundary reservation {}: {message}",
            if cancelled { "cancelled" } else { "failed" }
        ),
    }
}

#[cfg(test)]
mod reservation_cancellation_tests {
    use super::*;

    #[test]
    fn cancellation_while_actor_reply_is_pending_is_not_packetizer_failure() {
        let signal = AtomicBool::new(false);
        let rejected = reservation_error("actor refused boundary", &signal);
        assert_eq!(rejected.kind, PacketizerWorkerErrorKind::Packetize);
        signal.store(true, Ordering::Release);
        let interrupted = reservation_error("actor reply interrupted", &signal);
        assert_eq!(interrupted.kind, PacketizerWorkerErrorKind::Cancelled);
        assert!(interrupted.message.contains("cancelled"));
    }
}
