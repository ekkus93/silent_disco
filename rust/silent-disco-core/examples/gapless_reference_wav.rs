//! Generate one canonical PCM16 WAV per source using Silent Disco's own
//! decoder, so listeners can be compared against the same normalization path.
//! Usage:
//! cargo run --manifest-path rust/silent-disco-core/Cargo.toml --example gapless_reference_wav --
//!   /tmp/references source-one.mp3 source-two.wav
use silent_disco_core::audio::{DebugPcmRecorder, StreamingDecodeConfig, StreamingDecodeHandle};
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let output_dir = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| invalid("expected output directory followed by source files"))?;
    let sources: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if sources.is_empty() {
        return Err(invalid("at least one source file is required").into());
    }
    fs::create_dir_all(&output_dir)?;
    for (index, path) in sources.iter().enumerate() {
        let decoder = StreamingDecodeHandle::open(path, StreamingDecodeConfig::default())?;
        let format = decoder.stream_info().format;
        let output = output_dir.join(format!("{index:03}.wav"));
        let mut recorder =
            DebugPcmRecorder::create(&output, format.sample_rate_hz, format.channels)?;
        let mut next_sample_index = 0_u64;
        while let Ok(chunk) = decoder.recv() {
            if chunk.format != format {
                return Err(invalid("decoder changed the canonical format").into());
            }
            if chunk.first_sample_index.get() != next_sample_index {
                return Err(invalid("decoded reference has a sample gap or overlap").into());
            }
            next_sample_index = next_sample_index
                .checked_add(u64::try_from(chunk.frame_count())?)
                .ok_or_else(|| invalid("decoded reference frame index overflow"))?;
            recorder.append(&chunk.frames)?;
        }
        decoder.join()?;
        recorder.finish(format.sample_rate_hz)?;
        println!(
            "{}: {} canonical frames at {} Hz/{} channels -> {}",
            path.display(),
            next_sample_index,
            format.sample_rate_hz,
            format.channels,
            output.display()
        );
    }
    Ok(())
}
