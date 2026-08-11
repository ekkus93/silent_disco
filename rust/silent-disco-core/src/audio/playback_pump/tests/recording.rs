//! Debug capture: exactly what was released toward the ring, including
//! concealed frames but not the stream-start alignment silence, and
//! first-failure-disables-capture behavior.

use crate::audio::{DEFAULT_CONCEALMENT_RAMP_MS, DebugPcmRecorder};

use super::{HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, datagram, pump_with};

#[test]
fn the_debug_capture_records_exactly_what_was_released_toward_the_ring() {
    let directory =
        std::env::temp_dir().join(format!("silent-disco-pump-capture-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("capture.wav");

    let (mut pump, _consumer) = pump_with(48_000);
    pump.set_recorder(DebugPcmRecorder::create(&path, 48_000, 2).expect("recorder created"));
    // Sequence 1 is lost, so the capture must contain the concealed frame
    // too: it records what playback believed it was playing.
    for sequence in [0, 2, 3] {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    pump.tick(HOST_START_MS);
    pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS));
    pump.finish();

    assert!(pump.recorder_error().is_none());
    let bytes = std::fs::read(&path).expect("capture readable");
    // Four 960-frame stereo PCM16 frames: 0, concealed 1, then 2 and 3
    // drained at stop.
    let expected_data_bytes = 4 * 960 * 2 * 2;
    assert_eq!(bytes.len(), 44 + expected_data_bytes);
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[36..40], b"data");
    // The header is patched with the real length when the stream ends.
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
        u32::try_from(expected_data_bytes).expect("fits"),
    );
    // Sample rate and channel count round-trip.
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        48_000
    );
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);

    std::fs::remove_file(&path).ok();
}

/// Burst-loss simulation rendered through the real pump and measured off
/// the captured WAV, which is how this defect was found on a device and
/// the only way to see the artefact it produced.
///
/// Four consecutive losses used to emit four separately-ramped 20ms
/// envelopes, each returning to zero — amplitude modulation at the 50Hz
/// packet rate. The capture must instead show one continuous decay: no
/// interior silence, and no sample-to-sample step anywhere in the stream.
#[test]
fn a_burst_of_lost_packets_renders_as_one_continuous_decay() {
    let directory =
        std::env::temp_dir().join(format!("silent-disco-pump-burst-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("burst.wav");

    let (mut pump, _consumer) = pump_with(48_000);
    pump.set_recorder(DebugPcmRecorder::create(&path, 48_000, 2).expect("recorder created"));
    // Sequences 1 through 4 are lost: a four-packet burst, well inside the
    // skip threshold so every slot is concealed rather than abandoned.
    for sequence in [0, 5, 6, 7] {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }
    for slot in 0..6 {
        pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS) * slot);
    }
    pump.finish();
    assert!(pump.recorder_error().is_none());

    let bytes = std::fs::read(&path).expect("capture readable");
    let samples: Vec<i16> = bytes[44..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    // Frame 0 real, frames 1-4 concealed, frames 5-7 real.
    assert_eq!(samples.len(), 8 * 960 * 2);

    // The burst spans frames 1 through 4; the run must decay without ever
    // touching silence before real audio resumes.
    let burst = &samples[960 * 2..5 * 960 * 2];
    assert!(
        burst.iter().all(|&sample| sample != 0),
        "the concealed burst returned to silence between packets"
    );

    // No step anywhere: adjacent samples within a channel never jump by
    // more than one blend increment. A hard splice would jump by the full
    // sample amplitude at once; a ramped one climbs it over the ramp, so
    // the bound is derived from the ramp rather than fixed -- shortening
    // the ramp legitimately steepens every seam.
    let ramp_frames = SAMPLES_PER_PACKET * DEFAULT_CONCEALMENT_RAMP_MS / PACKET_DURATION_MS;
    let largest_ramped_step = i32::try_from(16_384 / ramp_frames).expect("fits") + 4;
    let mut largest_step = 0;
    for channel in 0..2 {
        let channel_samples: Vec<i16> = samples.iter().skip(channel).step_by(2).copied().collect();
        for pair in channel_samples.windows(2) {
            largest_step = largest_step.max((i32::from(pair[1]) - i32::from(pair[0])).abs());
        }
    }
    assert!(
        largest_step <= largest_ramped_step,
        "waveform stepped by {largest_step} within the burst or at its seams, \
         above the {largest_ramped_step} a single ramp increment allows"
    );

    std::fs::remove_file(&path).ok();
}

#[test]
fn a_pump_without_a_recorder_configured_captures_nothing() {
    let (mut pump, _consumer) = pump_with(48_000);
    pump.scheduler_mut()
        .submit_packet(datagram(0, 16_384))
        .expect("accepted");

    pump.tick(HOST_START_MS);
    pump.finish();

    assert!(pump.recorder_error().is_none());
}
