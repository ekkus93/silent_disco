use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

#[derive(Debug)]
struct Arguments {
    path: PathBuf,
    cancel_after_frames: Option<u64>,
}

#[derive(Debug)]
struct DecodeReport {
    path: PathBuf,
    status: &'static str,
    error_class: Option<&'static str>,
    error_detail: Option<String>,
    startup_micros: u128,
    decode_micros: u128,
    packets: u64,
    frames: u64,
    sample_rate_hz: Option<u32>,
    channels: Option<usize>,
    source_sample_format: String,
    codec: String,
    cancellation_requested: bool,
}

#[derive(Debug, Default)]
struct DecodeProgress {
    packets: u64,
    frames: u64,
    sample_rate_hz: Option<u32>,
    channels: Option<usize>,
}

#[derive(Debug)]
struct DecodeMetrics {
    startup_micros: u128,
    decode_micros: u128,
    progress: DecodeProgress,
}

#[derive(Debug)]
enum DecodeDisposition {
    Decoded,
    Cancelled,
    Failed {
        class: &'static str,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopOutcome {
    Complete,
    Cancelled,
}

fn main() -> ExitCode {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    println!("{}", decode_file(&arguments).to_json());
    ExitCode::SUCCESS
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut arguments = env::args_os().skip(1);
    let path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        "usage: silent-disco-decoder-spike <path> [--cancel-after-frames N]".to_owned()
    })?;

    let mut cancel_after_frames = None;
    while let Some(argument) = arguments.next() {
        if argument != "--cancel-after-frames" {
            return Err(format!("unknown argument: {}", argument.to_string_lossy()));
        }
        let value = arguments
            .next()
            .ok_or_else(|| "--cancel-after-frames requires a value".to_owned())?;
        cancel_after_frames = Some(
            value
                .to_string_lossy()
                .parse::<u64>()
                .map_err(|error| format!("invalid cancellation frame count: {error}"))?,
        );
    }

    Ok(Arguments {
        path,
        cancel_after_frames,
    })
}

fn decode_file(arguments: &Arguments) -> DecodeReport {
    let started = Instant::now();
    match open_decoder(&arguments.path) {
        Ok(opened) => decode_opened(arguments, opened, started.elapsed().as_micros()),
        Err(error) => open_failure_report(arguments, started.elapsed().as_micros(), &error),
    }
}

fn decode_opened(
    arguments: &Arguments,
    mut opened: OpenedDecoder,
    startup_micros: u128,
) -> DecodeReport {
    let decode_started = Instant::now();
    let mut progress = DecodeProgress::default();
    let disposition = match decode_packets(arguments, &mut opened, &mut progress) {
        Ok(LoopOutcome::Cancelled) => DecodeDisposition::Cancelled,
        Ok(LoopOutcome::Complete) if progress.frames == 0 => DecodeDisposition::Failed {
            class: "empty_stream",
            detail: "decoder reached end-of-stream without PCM frames".to_owned(),
        },
        Ok(LoopOutcome::Complete) => DecodeDisposition::Decoded,
        Err(error) => DecodeDisposition::Failed {
            class: classify_error(&error),
            detail: error.to_string(),
        },
    };
    let metrics = DecodeMetrics {
        startup_micros,
        decode_micros: decode_started.elapsed().as_micros(),
        progress,
    };
    report_for_opened(arguments, &opened, metrics, disposition)
}

fn decode_packets(
    arguments: &Arguments,
    opened: &mut OpenedDecoder,
    progress: &mut DecodeProgress,
) -> Result<LoopOutcome, Error> {
    loop {
        if arguments
            .cancel_after_frames
            .is_some_and(|threshold| progress.frames >= threshold)
        {
            return Ok(LoopOutcome::Cancelled);
        }

        let Some(packet) = opened.format.next_packet()? else {
            return Ok(LoopOutcome::Complete);
        };
        if packet.track_id != opened.track_id {
            continue;
        }
        progress.packets = progress.packets.saturating_add(1);

        let decoded = opened.decoder.decode(&packet)?;
        let decoded_frames = u64::try_from(decoded.frames()).unwrap_or(u64::MAX);
        progress.frames = progress.frames.saturating_add(decoded_frames);
        progress.sample_rate_hz.get_or_insert(decoded.spec().rate());
        progress
            .channels
            .get_or_insert(decoded.spec().channels().count());
    }
}

fn report_for_opened(
    arguments: &Arguments,
    opened: &OpenedDecoder,
    metrics: DecodeMetrics,
    disposition: DecodeDisposition,
) -> DecodeReport {
    let (status, error_class, error_detail, cancellation_requested) = match disposition {
        DecodeDisposition::Decoded => ("decoded", None, None, false),
        DecodeDisposition::Cancelled => ("cancelled", None, None, true),
        DecodeDisposition::Failed { class, detail } => {
            ("error", Some(class), Some(detail), false)
        }
    };
    DecodeReport {
        path: arguments.path.clone(),
        status,
        error_class,
        error_detail,
        startup_micros: metrics.startup_micros,
        decode_micros: metrics.decode_micros,
        packets: metrics.progress.packets,
        frames: metrics.progress.frames,
        sample_rate_hz: metrics.progress.sample_rate_hz,
        channels: metrics.progress.channels,
        source_sample_format: opened.source_sample_format.clone(),
        codec: opened.codec.clone(),
        cancellation_requested,
    }
}

fn open_failure_report(
    arguments: &Arguments,
    startup_micros: u128,
    error: &Error,
) -> DecodeReport {
    DecodeReport {
        path: arguments.path.clone(),
        status: "error",
        error_class: Some(classify_error(error)),
        error_detail: Some(error.to_string()),
        startup_micros,
        decode_micros: 0,
        packets: 0,
        frames: 0,
        sample_rate_hz: None,
        channels: None,
        source_sample_format: "unknown".to_owned(),
        codec: "unknown".to_owned(),
        cancellation_requested: false,
    }
}

struct OpenedDecoder {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track_id: u32,
    source_sample_format: String,
    codec: String,
}

fn open_decoder(path: &Path) -> Result<OpenedDecoder, Error> {
    let source = File::open(path).map_err(Error::IoError)?;
    let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }

    let format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .cloned()
        .ok_or(Error::Unsupported("no audio track"))?;
    let Some(CodecParameters::Audio(audio_parameters)) = track.codec_params.as_ref() else {
        return Err(Error::Unsupported("audio codec parameters missing"));
    };
    let source_sample_format = format!("{:?}", audio_parameters.sample_format);
    let codec = format!("{:?}", audio_parameters.codec);
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_parameters, &AudioDecoderOptions::default())?;

    Ok(OpenedDecoder {
        format,
        decoder,
        track_id: track.id,
        source_sample_format,
        codec,
    })
}

fn classify_error(error: &Error) -> &'static str {
    match error {
        Error::IoError(_) => "io",
        Error::DecodeError(_) => "corrupt",
        Error::SeekError(_) => "seek",
        Error::Unsupported(_) => "unsupported",
        Error::LimitError(_) => "limit",
        Error::ResetRequired => "format_change",
        _ => "other",
    }
}

impl DecodeReport {
    fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"path\":{},\"status\":{},\"error_class\":{},\"error_detail\":{},",
                "\"startup_micros\":{},\"decode_micros\":{},\"packets\":{},\"frames\":{},",
                "\"sample_rate_hz\":{},\"channels\":{},\"source_sample_format\":{},",
                "\"codec\":{},\"cancellation_requested\":{}}}"
            ),
            json_string(&self.path.to_string_lossy()),
            json_string(self.status),
            json_optional_string(self.error_class),
            json_optional_string(self.error_detail.as_deref()),
            self.startup_micros,
            self.decode_micros,
            self.packets,
            self.frames,
            json_optional_number(self.sample_rate_hz),
            json_optional_number(self.channels),
            json_string(&self.source_sample_format),
            json_string(&self.codec),
            self.cancellation_requested,
        )
    }
}

fn json_optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn json_string(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control.is_control() => {
                let value = u32::from(control);
                escaped.push_str("\\u");
                for shift in [12_u32, 8, 4, 0] {
                    let index = usize::try_from((value >> shift) & 0x0f).unwrap_or(0);
                    escaped.push(char::from(HEX[index]));
                }
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::{json_optional_string, json_string};

    #[test]
    fn json_string_escapes_control_characters() {
        assert_eq!(json_string("a\n\"b\\c"), "\"a\\n\\\"b\\\\c\"");
    }

    #[test]
    fn optional_string_emits_null() {
        assert_eq!(json_optional_string(None), "null");
    }
}
