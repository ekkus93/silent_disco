use crate::dto::DesktopErrorDto;
use crate::platform::file_picker::InspectedAudioSource;
use crate::platform::file_picker::SelectedSourceRegistry;
use crate::platform::source_staging_control::SourceStagingOperation;
use silent_disco_core::audio::{
    DecodeError, DecodeErrorKind, StreamingDecodeConfig, StreamingDecodeHandle,
};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

/// Desktop-owned preparation result retaining the exact staged-source identity.
pub(crate) struct PreparedAudioSource {
    descriptor: AudioSourceDescriptor,
    decoder: StreamingDecodeHandle,
}

/// Fully decodes a staged item through the shared bounded decoder before queue acceptance.
/// PCM chunks are discarded immediately; cancellation is checked while waiting for each chunk.
pub(crate) fn validate_staged_audio_source(
    source: &InspectedAudioSource,
    operation: &SourceStagingOperation,
) -> Result<(), DesktopErrorDto> {
    operation.ensure_not_cancelled()?;
    let decoder =
        StreamingDecodeHandle::open(source.canonical_path(), StreamingDecodeConfig::default())
            .map_err(|error| map_shared_decode_error(&error))?;
    loop {
        operation.ensure_not_cancelled()?;
        match decoder.recv_timeout(Duration::from_millis(25)) {
            Ok(chunk) if chunk.end_of_stream => {
                decoder
                    .join()
                    .map_err(|error| map_shared_decode_error(&error))?;
                return Ok(());
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return decoder
                    .join()
                    .map(|_| ())
                    .map_err(|error| map_shared_decode_error(&error));
            }
        }
    }
}

impl PreparedAudioSource {
    #[must_use]
    pub(crate) fn descriptor(&self) -> &AudioSourceDescriptor {
        &self.descriptor
    }

    pub(crate) fn decoder(&self) -> &StreamingDecodeHandle {
        &self.decoder
    }

    pub(crate) fn into_decoder(self) -> StreamingDecodeHandle {
        self.decoder
    }
}

/// Resolves an opaque staged source and starts the shared bounded decoder.
///
/// Native paths remain inside the desktop adapter; callers receive only the
/// original semantic descriptor plus the shared PCM stream handle.
pub(crate) fn prepare_staged_audio_source(
    registry: &SelectedSourceRegistry,
    descriptor: &AudioSourceDescriptor,
    config: StreamingDecodeConfig,
) -> Result<PreparedAudioSource, DesktopErrorDto> {
    let source = registry.resolve(&descriptor.source_id)?.ok_or_else(|| {
        decode_error(
            "desktop.audio_decode.source_not_staged",
            true,
            "the selected audio source is no longer staged in this desktop profile",
        )
    })?;
    if source.descriptor() != descriptor {
        return Err(decode_error(
            "desktop.audio_decode.source_changed",
            true,
            "the staged audio source descriptor changed before preparation",
        ));
    }
    let decoder = StreamingDecodeHandle::open(source.canonical_path(), config)
        .map_err(|error| map_shared_decode_error(&error))?;
    Ok(PreparedAudioSource {
        descriptor: descriptor.clone(),
        decoder,
    })
}

fn map_shared_decode_error(error: &DecodeError) -> DesktopErrorDto {
    let (code, retryable) = match error.kind {
        DecodeErrorKind::InvalidConfiguration => {
            ("desktop.audio_decode.invalid_configuration", false)
        }
        DecodeErrorKind::Io => ("desktop.audio_decode.source_unavailable", true),
        DecodeErrorKind::UnsupportedFormat => ("desktop.audio_decode.unsupported_format", false),
        DecodeErrorKind::CorruptInput => ("desktop.audio_decode.corrupt_source", false),
        DecodeErrorKind::InvalidMetadata => ("desktop.audio_decode.invalid_metadata", false),
        DecodeErrorKind::EmptySource => ("desktop.audio_decode.empty_source", false),
        DecodeErrorKind::ResourceLimit => ("desktop.audio_decode.resource_limit", false),
        DecodeErrorKind::FormatChanged => ("desktop.audio_decode.format_changed", false),
        DecodeErrorKind::Cancelled => ("desktop.audio_decode.cancelled", true),
        DecodeErrorKind::WorkerPanicked => ("desktop.audio_decode.worker_failed", true),
    };
    decode_error(code, retryable, &error.message)
}

fn decode_error(code: &str, retryable: bool, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "audio", "error", retryable, message)
}
