//! Orchestrates one `start_playback` command: resolves the staged source,
//! opens the shared decoder, spawns the shared packetizer, and hands the
//! result to [`super::network::DesktopHostNetworkControl::start_playback`].

use super::audio_decode::prepare_staged_audio_source;
use super::file_picker::SelectedSourceRegistry;
use super::network::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use silent_disco_core::audio::{
    PacketizerWorkerError, PacketizerWorkerErrorKind, StreamingDecodeConfig,
    StreamingPacketizeConfig, StreamingPacketizeHandle,
};
use silent_disco_core::domain::{MonotonicMillis, PlaybackState, StreamId};
use silent_disco_core::runtime::{AudioEvent, CoreActorHandle};
use std::sync::Arc;

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

    match start_after_buffering(handle, network, registry, &descriptor, session_id) {
        Ok(()) => Ok(()),
        Err(error) => {
            drop(handle.submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Error)));
            Err(error)
        }
    }
}

fn start_after_buffering(
    handle: &CoreActorHandle,
    network: &Arc<DesktopHostNetworkControl>,
    registry: &SelectedSourceRegistry,
    descriptor: &silent_disco_core::runtime::AudioSourceDescriptor,
    session_id: silent_disco_core::domain::SessionId,
) -> Result<(), DesktopErrorDto> {
    let prepared =
        prepare_staged_audio_source(registry, descriptor, StreamingDecodeConfig::default())?;
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
    let packetizer = StreamingPacketizeHandle::spawn(
        decoder,
        session_id.clone(),
        stream_id.clone(),
        host_start_time_ms,
        StreamingPacketizeConfig::default(),
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

fn map_packetizer_error(error: &PacketizerWorkerError) -> DesktopErrorDto {
    let code = match error.kind {
        PacketizerWorkerErrorKind::Source => "desktop.playback.decode_failed",
        PacketizerWorkerErrorKind::Packetize => "desktop.playback.packetize_failed",
        PacketizerWorkerErrorKind::Cancelled => "desktop.playback.cancelled",
        PacketizerWorkerErrorKind::WorkerPanicked => "desktop.playback.worker_failed",
    };
    DesktopErrorDto::new(code, "audio", "error", true, &error.message)
}
