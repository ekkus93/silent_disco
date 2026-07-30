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
use symphonia::core::io::MediaSourceStream;
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

fn main() -> ExitCode {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    let report = decode_file(&arguments);
    println!("{}", report.to_json());
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
        Ok(mut opened) => {
            let startup_micros = started.elapsed().as_micros();
            let decode_started = Instant::now();
            let mut packets = 0_u64;
            let mut frames = 0_u64;
            let mut sample_rate_hz = None;
            let mut channels = None;

            loop {
                if arguments
                    .cancel_after_frames
                    .is_some_and(|threshold| frames >= threshold)
                {
                    return DecodeReport {
                        path: arguments.path.clone(),
                        status: "cancelled",
                        error_class: None,
                        error_detail: None,
                        startup_micros,
                        decode_micros: decode_started.elapsed().as_micros(),
                        packets,
                        frames,
                        sample_rate_hz,
                        channels,
                        source_sample_format: opened.source_sample_format,
                        codec: opened.codec,
                        cancellation_requested: true,
                    };
                }

                let packet = match opened.format.next_packet() {
                    Ok(Some(packet)) => packet,
                    Ok(None) => break,
                    Err(error) => {
                        return failure_report(
                            arguments,
                            startup_micros,
                            decode_started.elapsed().as_micros(),
                            packets,
                            frames,
                            sample_rate_hz,
                            channels,
                            opened.source_sample_format,
                            opened.codec,
                            error,
                        );
                    }
                };

                if packet.track_id != opened.track_id {
                    continue;
                }
                packets = packets.saturating_add(1);

                match opened.decoder.decode(&packet) {
                    Ok(decoded) => {
                        let decoded_frames = u64::try_from(decoded.frames()).unwrap_or(u64::MAX);
                        frames = frames.saturating_add(decoded_frames);
                        sample_rate_hz.get_or_insert(decoded.spec().rate());
                        channels.get_or_insert(decoded.spec().channels().count());
                    }
                    Err(error) => {
                        return failure_report(
                            arguments,
                            startup_micros,
                            decode_started.elapsed().as_micros(),
                            packets,
                            frames,
                            sample_rate_hz,
                            channels,
                            opened.source_sample_format,
                            opened.codec,
                            error,
                        );
                    }
                }
            }

            let (status, error_class, error_detail) = if frames == 0 {
                (
                    "error",
                    Some("empty_stream"),
                    Some("decoder reached end-of-stream without PCM frames".to_owned()),
                )
            } else {
                ("decoded", None, None)
            };

            DecodeReport {
                path: arguments.path.clone(),
                status,
                error_class,
                error_detail,
                startup_micros,
                decode_micros: decode_started.elapsed().as_micros(),
                packets,
                frames,
                sample_rate_hz,
                channels,
                source_sample_format: opened.source_sample_format,
                codec: opened.codec,
                cancellation_requested: false,
            }
        }
        Err(error) => DecodeReport {
            path: arguments.path.clone(),
            status: "error",
            error_class: Some(classify_error(&error)),
            error_detail: Some(error.to_string()),
            startup_micros: started.elapsed().as_micros(),
            decode_micros: 0,
            packets: 0,
            frames: 0,
            sample_rate_hz: None,
            channels: None,
            source_sample_format: "unknown".to_owned(),
            codec: "unknown".to_owned(),
            cancellation_requested: false,
        },
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
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
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
    let audio_parameters = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(parameters)) => parameters,
        _ => return Err(Error::Unsupported("audio codec parameters missing")),
    };
    let source_sample_format = format!("{:?}", audio_parameters.sample_format);
    let codec = format!("{:?}", audio_parameters.codec);
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_parameters, &AudioDecoderOptions::default())?;

    Ok(OpenedDecoder {
        format,
        decoder,
        track_id: track.id,
        source_sample_format,
        codec,
    })
}

#[allow(clippy::too_many_arguments)]
fn failure_report(
    arguments: &Arguments,
    startup_micros: u128,
    decode_micros: u128,
    packets: u64,
    frames: u64,
    sample_rate_hz: Option<u32>,
    channels: Option<usize>,
    source_sample_format: String,
    codec: String,
    error: Error,
) -> DecodeReport {
    DecodeReport {
        path: arguments.path.clone(),
        status: "error",
        error_class: Some(classify_error(&error)),
        error_detail: Some(error.to_string()),
        startup_micros,
        decode_micros,
        packets,
        frames,
        sample_rate_hz,
        channels,
        source_sample_format,
        codec,
        cancellation_requested: false,
    }
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
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", u32::from(control));
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
