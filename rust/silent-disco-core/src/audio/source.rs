use super::resampler::StereoResampler;
use super::types::{
    AudioFormat, DecodeError, DecodeErrorKind, DecodeStreamInfo, MAX_SOURCE_CHANNELS,
    StreamingDecodeConfig,
};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

pub(super) const MAX_DECODED_PACKET_FRAMES: usize = 65_536;
const MAX_ID3V2_METADATA_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct OpenedDecoder {
    pub(super) format: Box<dyn symphonia::core::formats::FormatReader>,
    pub(super) decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    pub(super) track_id: u32,
    pub(super) stream_info: DecodeStreamInfo,
    pub(super) metadata_prefixed: bool,
}

impl OpenedDecoder {
    pub(super) fn open(path: &Path, config: StreamingDecodeConfig) -> Result<Self, DecodeError> {
        let metadata = path
            .metadata()
            .map_err(|error| map_io_error(error, "inspect source"))?;
        if !metadata.file_type().is_file() {
            return Err(DecodeError::new(
                DecodeErrorKind::Io,
                "decoder source is not a regular file",
            ));
        }
        if metadata.len() == 0 {
            return Err(DecodeError::new(
                DecodeErrorKind::EmptySource,
                "decoder source is empty",
            ));
        }
        if metadata.len() > config.max_source_bytes {
            return Err(DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "decoder source exceeds the configured byte limit",
            ));
        }

        let metadata_prefixed = inspect_id3v2_prefix(path, metadata.len())?;
        let source = File::open(path).map_err(|error| map_io_error(error, "open source"))?;
        let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
            hint.with_extension(extension);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|error| map_symphonia_error(error, metadata_prefixed))?;
        let track = format
            .default_track(TrackType::Audio)
            .cloned()
            .ok_or_else(|| {
                DecodeError::new(
                    DecodeErrorKind::UnsupportedFormat,
                    "decoder source has no supported audio track",
                )
            })?;
        let Some(CodecParameters::Audio(audio_parameters)) = track.codec_params.as_ref() else {
            return Err(DecodeError::new(
                DecodeErrorKind::UnsupportedFormat,
                "decoder source has no audio codec parameters",
            ));
        };
        validate_declared_format(
            audio_parameters.sample_rate,
            audio_parameters
                .channels
                .as_ref()
                .map(symphonia::core::audio::Channels::count),
        )?;
        if audio_parameters.max_frames_per_packet.is_some_and(|frames| {
            usize::try_from(frames).map_or(true, |value| value > MAX_DECODED_PACKET_FRAMES)
        }) {
            return Err(DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "declared decoded packet size exceeds the bounded source-frame limit",
            ));
        }
        let decoder_options = AudioDecoderOptions::default().verify(true);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(audio_parameters, &decoder_options)
            .map_err(|error| map_symphonia_error(error, metadata_prefixed))?;
        let source_channels = audio_parameters
            .channels
            .as_ref()
            .map(symphonia::core::audio::Channels::count)
            .map(u16::try_from)
            .transpose()
            .map_err(|_| {
                DecodeError::new(
                    DecodeErrorKind::UnsupportedFormat,
                    "source channel count exceeds the supported range",
                )
            })?;
        Ok(Self {
            format,
            decoder,
            track_id: track.id,
            stream_info: DecodeStreamInfo {
                format: AudioFormat::CANONICAL,
                source_sample_rate_hz: audio_parameters.sample_rate,
                source_channels,
                source_duration_ms: None,
                source_byte_length: metadata.len(),
            },
            metadata_prefixed,
        })
    }
}

pub(super) fn validate_declared_format(
    sample_rate_hz: Option<u32>,
    channels: Option<usize>,
) -> Result<(), DecodeError> {
    if let Some(sample_rate_hz) = sample_rate_hz {
        StereoResampler::new(sample_rate_hz)?;
    }
    if channels.is_some_and(|value| value == 0 || value > MAX_SOURCE_CHANNELS) {
        return Err(DecodeError::new(
            DecodeErrorKind::UnsupportedFormat,
            "only mono and stereo sources are supported",
        ));
    }
    Ok(())
}

fn inspect_id3v2_prefix(path: &Path, source_length: u64) -> Result<bool, DecodeError> {
    let mut file = File::open(path).map_err(|error| map_io_error(error, "inspect metadata prefix"))?;
    let mut header = [0_u8; 10];
    let read = file
        .read(&mut header)
        .map_err(|error| map_io_error(error, "inspect metadata prefix"))?;
    if read < 3 || header[..3] != *b"ID3" {
        return Ok(false);
    }
    if read < header.len() {
        return Err(DecodeError::new(
            DecodeErrorKind::InvalidMetadata,
            "ID3v2 metadata header is truncated",
        ));
    }
    if !(2..=4).contains(&header[3]) || header[4] == 0xff {
        return Err(DecodeError::new(
            DecodeErrorKind::InvalidMetadata,
            "ID3v2 metadata version is invalid",
        ));
    }
    let size_bytes = &header[6..10];
    if size_bytes.iter().any(|byte| byte & 0x80 != 0) {
        return Err(DecodeError::new(
            DecodeErrorKind::InvalidMetadata,
            "ID3v2 metadata size is not sync-safe",
        ));
    }
    let payload_length = size_bytes
        .iter()
        .fold(0_u64, |value, byte| (value << 7) | u64::from(*byte));
    if payload_length > MAX_ID3V2_METADATA_BYTES {
        return Err(DecodeError::new(
            DecodeErrorKind::ResourceLimit,
            "ID3v2 metadata exceeds the bounded metadata limit",
        ));
    }
    let total_length = payload_length.checked_add(10).ok_or_else(|| {
        DecodeError::new(
            DecodeErrorKind::ResourceLimit,
            "ID3v2 metadata length overflowed",
        )
    })?;
    if total_length > source_length {
        return Err(DecodeError::new(
            DecodeErrorKind::InvalidMetadata,
            "ID3v2 metadata extends beyond the source file",
        ));
    }
    Ok(true)
}

fn map_io_error(error: io::Error, operation: &str) -> DecodeError {
    let kind = if error.kind() == io::ErrorKind::UnexpectedEof {
        DecodeErrorKind::CorruptInput
    } else {
        DecodeErrorKind::Io
    };
    DecodeError::new(kind, format!("could not {operation}: {error}"))
}

pub(super) fn map_symphonia_error(
    error: SymphoniaError,
    metadata_prefixed: bool,
) -> DecodeError {
    match error {
        SymphoniaError::IoError(error)
            if metadata_prefixed && error.kind() == io::ErrorKind::UnexpectedEof =>
        {
            DecodeError::new(
                DecodeErrorKind::InvalidMetadata,
                format!("invalid source metadata: {error}"),
            )
        }
        SymphoniaError::IoError(error) => map_io_error(error, "decode source"),
        SymphoniaError::DecodeError(detail) => DecodeError::new(
            DecodeErrorKind::CorruptInput,
            format!("corrupt audio source: {detail}"),
        ),
        SymphoniaError::Unsupported(detail) => DecodeError::new(
            DecodeErrorKind::UnsupportedFormat,
            format!("unsupported audio source: {detail}"),
        ),
        SymphoniaError::LimitError(detail) => DecodeError::new(
            DecodeErrorKind::ResourceLimit,
            format!("audio source exceeded a decoder limit: {detail}"),
        ),
        SymphoniaError::ResetRequired => DecodeError::new(
            DecodeErrorKind::FormatChanged,
            "audio source requested an unsupported mid-stream decoder reset",
        ),
        other => DecodeError::new(
            DecodeErrorKind::CorruptInput,
            format!("audio source could not be decoded: {other}"),
        ),
    }
}
