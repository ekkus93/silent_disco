//! Melody fixture generation for the manual device tests: a recognizable
//! ascending/descending C major scale a human listener can judge for
//! smoothness by ear, plus the real `ffmpeg` encode step needed to produce
//! FLAC/MP3 variants (the desktop app only ever decodes audio).

use crate::platform::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// The eight notes of an ascending C major scale ("do re mi fa so la ti
/// do"), a sequence a listener can recognize and judge for smoothness by
/// ear far more easily than one sustained tone -- a dropped, repeated, or
/// glitched note is unmistakable, where a gap in a continuous tone can
/// blend into the tone's own texture.
/// Linear fade-in/fade-out applied at each note boundary in
/// [`melody_pcm_wav`], long enough to remove the phase-reset amplitude
/// discontinuity without being long enough to noticeably shorten the note.
const NOTE_FADE_SECONDS: f64 = 0.005;

pub(super) const C_MAJOR_SCALE_HZ: [f64; 8] = [
    261.63, // C4
    293.66, // D4
    329.63, // E4
    349.23, // F4
    392.00, // G4
    440.00, // A4
    493.88, // B4
    523.25, // C5
];

pub(super) fn stage_melody_source(
    temp: &TempDir,
    registry: &SelectedSourceRegistry,
    source_id: &str,
    notes_hz: &[f64],
    note_seconds: f64,
    total_seconds: u32,
    container: AudioContainer,
) -> AudioSourceDescriptor {
    let wav_bytes = melody_pcm_wav(notes_hz, note_seconds, total_seconds);
    let (extension, bytes) = match container {
        AudioContainer::Wav => ("wav", wav_bytes),
        AudioContainer::Flac => ("flac", encode_with_ffmpeg(&wav_bytes, "flac")),
        AudioContainer::Mp3 => ("mp3", encode_with_ffmpeg(&wav_bytes, "mp3")),
    };
    let source_path = temp.path().join(format!("{source_id}.{extension}"));
    fs::write(&source_path, &bytes).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        format!("desktop-block-playback-manual-{source_id}"),
        format!("{source_id}.{extension}"),
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            container,
        ))
        .expect("register staged source");
    descriptor
}

/// Encodes PCM WAV bytes to the given container via a real `ffmpeg`
/// subprocess. The desktop app only ever *decodes* audio (`symphonia` has
/// no encoder -- see `docs/DESKTOP_BLOCK18_DECODER_DECISION.md`), so
/// producing a FLAC/MP3 fixture long enough for a human to judge by ear
/// needs a real external encoder. Manual-test-only dependency: never used
/// by the automated suite, and this panics with a clear message rather than
/// silently skipping if `ffmpeg` is not on `PATH`.
fn encode_with_ffmpeg(wav_bytes: &[u8], extension: &str) -> Vec<u8> {
    let codec = match extension {
        "flac" => "flac",
        "mp3" => "libmp3lame",
        other => panic!("no ffmpeg codec mapped for extension: {other}"),
    };
    let temp = TempDir::new().expect("ffmpeg temp dir");
    let input_path = temp.path().join("input.wav");
    fs::write(&input_path, wav_bytes).expect("write ffmpeg input");
    let output_path = temp.path().join(format!("output.{extension}"));

    let result = Command::new("ffmpeg")
        .arg("-y")
        .args(["-loglevel", "error"])
        .arg("-i")
        .arg(&input_path)
        .args(["-c:a", codec])
        .arg(&output_path)
        .output();
    let output = match result {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => panic!(
            "ffmpeg is not on PATH -- install it to run the {extension} variant of this manual \
             device test (the desktop app only decodes audio, so a real external encoder is \
             needed to produce a fixture long enough to judge by ear)"
        ),
        Err(error) => panic!("failed to run ffmpeg: {error}"),
    };
    assert!(
        output.status.success(),
        "ffmpeg failed to encode {extension}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(&output_path).expect("read ffmpeg output")
}

/// Cycles through `notes_hz`, holding each for `note_seconds`, until
/// `total_seconds` of audio have been generated. Each note restarts its sine
/// wave at phase zero rather than gliding from the previous note, so a
/// listener hears a clean, deliberate transition, not a bend.
fn melody_pcm_wav(notes_hz: &[f64], note_seconds: f64, total_seconds: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = sample_rate * total_seconds;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames_per_note = (f64::from(sample_rate) * note_seconds) as u32;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    // Each note restarts its sine phase at 0, and the previous note almost
    // never ends on a zero-crossing -- without a fade, that's a real,
    // audible click at every note boundary baked into this fixture's own
    // audio, indistinguishable from a genuine playback-pipeline defect.
    // A short linear fade-in/fade-out at each note's edges removes that
    // amplitude discontinuity so any clicks heard on a real device are
    // attributable to the playback pipeline, not this synthetic source.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fade_frames = (f64::from(sample_rate) * NOTE_FADE_SECONDS) as u32;
    for index in 0..frame_count {
        let note_index =
            usize::try_from(index / frames_per_note).expect("note index") % notes_hz.len();
        let frequency_hz = notes_hz[note_index];
        let index_within_note = index % frames_per_note;
        let time_within_note = f64::from(index_within_note) / f64::from(sample_rate);
        let envelope = if index_within_note < fade_frames {
            f64::from(index_within_note) / f64::from(fade_frames)
        } else if index_within_note >= frames_per_note - fade_frames {
            f64::from(frames_per_note - index_within_note) / f64::from(fade_frames)
        } else {
            1.0
        };
        let sample =
            (time_within_note * frequency_hz * std::f64::consts::TAU).sin() * 12_000.0 * envelope;
        #[allow(clippy::cast_possible_truncation)]
        let sample = sample as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}
