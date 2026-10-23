//! Test the production playlist producer using actual encoded source fixtures.
//! Expected samples come from independent runs of the canonical Rust decoder;
//! protocol boundary and packet sequence continuity must survive codec changes.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
    silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::CallbackCaptureTap;
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::playlist_pcm_capture_tests::authorized_pcm_socket_pair;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use silent_disco_core::audio::{StreamingDecodeConfig, StreamingDecodeHandle};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{TransportChannel, TransportEvent};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one combined source decode, playlist packet sequence and boundary regression"
)]
fn encoded_flac_mp3_and_wav_playlist_matches_independent_canonical_pcm() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "silent-disco-compressed-playlist-{}-{unique}",
        std::process::id(),
    ));
    fs::create_dir_all(&root).expect("create fixtures directory");
    let encoded_sources = [
        (
            "wav",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.wav.b64"),
        ),
        (
            "flac",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.flac.b64"),
        ),
        (
            "mp3",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.mp3.b64"),
        ),
    ];
    let mut canonical = Vec::<i16>::new();
    let mut expected_boundaries = Vec::new();
    let mut sources = Vec::new();

    for (index, (extension, contents)) in encoded_sources.iter().enumerate() {
        let path = root.join(format!("song-{index}.{extension}"));
        let bytes = STANDARD
            .decode(contents.trim())
            .expect("known decoder fixture");
        fs::write(&path, bytes).expect("stage encoded source");
        let decoder = StreamingDecodeHandle::open(&path, StreamingDecodeConfig::default())
            .expect("decode canonical PCM reference");
        loop {
            let chunk = decoder
                .recv_timeout(Duration::from_secs(5))
                .expect("reference decoder produces PCM");
            let at_end = chunk.end_of_stream;
            canonical.extend(chunk.frames);
            if at_end {
                break;
            }
        }
        decoder.join().expect("reference decoder completed");
        assert_eq!(canonical.len() % 2, 0, "interleaved canonical stereo");
        if index + 1 < encoded_sources.len() {
            expected_boundaries.push(u64::try_from(canonical.len() / 2).expect("bounded fixture"));
        }
        sources.push(FfiPlaylistSource {
            entry_id: format!("encoded-entry-{index}"),
            display_name: format!("Encoded {extension}"),
            staged_path: path.to_string_lossy().into_owned(),
        });
    }
    assert_eq!(expected_boundaries.len(), 2);
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let playlist = FfiHostPlaylistStreamer::open(
        "end-to-end-session".to_owned(),
        "end-to-end-stream".to_owned(),
        1_000,
        sources,
    )
    .expect("start production playlist");
    let start = playlist.stream_start();
    assert_eq!(start.sample_rate, 48_000);
    assert_eq!(start.channels, 2);
    let sent_start = host
        .broadcast_control(&ControlMessage::StreamStart(StreamStart {
            session_id: SessionId::new("end-to-end-session").expect("session"),
            stream_id: StreamId::new("end-to-end-stream").expect("stream"),
            host_start_time_ms: MonotonicMillis::new(start.host_start_time_ms),
            sample_rate: start.sample_rate,
            channels: start.channels,
            samples_per_packet: start.samples_per_packet,
            current_entry_id: start.current_entry_id.clone(),
            current_display_name: start.current_display_name.clone(),
        }))
        .expect("announce encoded stream over TCP");
    assert_eq!(sent_start.report.successful_peers, 1);
    let observed_start = loop {
        if let TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::StreamStart(start)),
            ..
        } = listener
            .recv_event(Duration::from_secs(5))
            .expect("receive encoded stream start")
        {
            break start;
        }
    };
    assert_eq!(observed_start.sample_rate, start.sample_rate);
    assert_eq!(observed_start.channels, start.channels);

    // Exercise the same native C ABI used by Android Oboe with the actual
    // encoded playlist output, not just the packetizer's decoded bytes.
    // The tap is drained after every callback, as a background writer would.
    // Keep every encoded packet in one native ring until callbacks can span
    // both codec boundaries instead of conveniently reading packet-by-packet.
    let ring_frames = u32::try_from(canonical.len() / 2 + 480)
        .expect("bounded encoded fixtures")
        .max(4_800);
    let output = FfiAudioOutputHandle::open(ring_frames, 1).expect("native output ring");
    let token = output.engine_token().expect("output token");
    let tap = Arc::new(
        CallbackCaptureTap::new(usize::try_from(ring_frames).expect("capture capacity"))
            .expect("bounded capture tap"),
    );
    attach_callback_capture(u64::try_from(token).expect("token"), Arc::clone(&tap))
        .expect("attach native capture");
    let mut callback_pcm = Vec::<f32>::new();
    let mut sequence = 0_u64;
    let mut next_sample = 0_u64;
    let mut actual = Vec::<i16>::new();
    let mut boundaries = Vec::new();
    let mut ended = false;
    // These bundled fixtures are intentionally small; an ordinary playlist
    // must not stall or silently drop an encoded successor.
    for _ in 0..1_024 {
        match playlist
            .next_frame(5_000)
            .expect("production playlist frame")
        {
            Some(FfiPlaylistFrame::Audio {
                sequence: observed_sequence,
                first_sample_index,
                sample_rate,
                channels,
                samples_per_packet,
                host_presentation_time_ms,
                payload,
                ..
            }) => {
                assert_eq!(observed_sequence, sequence, "packet sequence continuity");
                assert_eq!(first_sample_index, next_sample, "PCM index gap/overlap");
                assert_eq!(sample_rate, 48_000);
                assert_eq!(channels, 2);
                assert_eq!(
                    payload.len(),
                    usize::try_from(samples_per_packet).expect("frames") * 4
                );
                let sent = host
                    .broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
                        session_id: SessionId::new("end-to-end-session").expect("session"),
                        stream_id: StreamId::new("end-to-end-stream").expect("stream"),
                        sequence: PacketSequence::new(sequence),
                        codec: AudioCodec::PcmS16Le,
                        sample_rate,
                        channels,
                        samples_per_packet,
                        first_sample_index: SampleIndex::new(first_sample_index),
                        host_presentation_time_ms: MonotonicMillis::new(
                            host_presentation_time_ms,
                        ),
                        payload,
                    }))
                    .expect("send encoded-source PCM over production UDP");
                assert_eq!(sent.report.successful_peers, 1);
                let received = loop {
                    if let TransportEvent::FrameReceived {
                        channel: TransportChannel::Audio,
                        frame: ProtocolFrame::Audio(datagram),
                        ..
                    } = listener
                        .recv_event(Duration::from_secs(5))
                        .expect("receive encoded-source PCM over UDP")
                    {
                        break datagram;
                    }
                };
                assert_eq!(received.sequence.get(), sequence);
                assert_eq!(received.first_sample_index.get(), first_sample_index);
                let packet_samples: Vec<i16> = received
                    .payload
                    .chunks_exact(2)
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                actual.extend_from_slice(&packet_samples);
                let packet_pcm: Vec<f32> = packet_samples
                    .iter()
                    .map(|sample| f32::from(*sample) / 32768.0)
                    .collect();
                assert_eq!(
                    output
                        .push_frames(packet_pcm.clone())
                        .expect("native ring write"),
                    samples_per_packet
                );
                next_sample += u64::from(samples_per_packet);
                sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                display_name,
                first_sample_index,
                ..
            }) => {
                assert_eq!(first_sample_index, next_sample, "boundary ordering");
                assert_eq!(entry_id, format!("encoded-entry-{}", boundaries.len() + 1));
                let sent = host
                    .broadcast_control(&ControlMessage::TrackBoundary(TrackBoundary {
                        session_id: SessionId::new("end-to-end-session").expect("session"),
                        stream_id: StreamId::new("end-to-end-stream").expect("stream"),
                        first_sample_index: SampleIndex::new(first_sample_index),
                        entry_id: entry_id.clone(),
                        display_name,
                    }))
                    .expect("send encoded-source boundary over TCP");
                assert_eq!(sent.report.successful_peers, 1);
                let received = loop {
                    if let TransportEvent::FrameReceived {
                        frame: ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)),
                        ..
                    } = listener
                        .recv_event(Duration::from_secs(5))
                        .expect("receive encoded-source boundary over TCP")
                    {
                        break boundary;
                    }
                };
                assert_eq!(received.first_sample_index.get(), first_sample_index);
                assert_eq!(received.entry_id, entry_id);
                boundaries.push(first_sample_index);
            }
            Some(FfiPlaylistFrame::End) => {
                ended = true;
                break;
            }
            None => {}
        }
    }
    assert!(ended, "production playlist must reach End");
    assert_eq!(
        boundaries, expected_boundaries,
        "exact decoded sample boundaries"
    );
    assert!(
        actual.len() >= canonical.len(),
        "must not drop decoded PCM tail"
    );
    assert_eq!(
        &actual[..canonical.len()],
        canonical,
        "codec switch altered PCM"
    );
    // The only permitted excess is the producer's final-packet padding, not
    // silence inserted at either track boundary.
    assert!(actual.len() - canonical.len() < 480);
    assert!(
        actual[canonical.len()..].iter().all(|sample| *sample == 0),
        "only final packet padding may add silence"
    );
    // Read the continuous native output in callback sizes that deliberately
    // cross the WAV->FLAC and FLAC->MP3 boundaries by one frame. A callback
    // is not synchronized with source or packet boundaries in real playback.
    let first_boundary = u32::try_from(expected_boundaries[0]).expect("first boundary");
    let second_boundary = u32::try_from(expected_boundaries[1]).expect("second boundary");
    let total_frames = u32::try_from(actual.len() / 2).expect("decoded frame count");
    assert!(first_boundary > 0 && second_boundary > first_boundary + 1);
    assert!(total_frames > second_boundary + 1);
    let callback_sizes = [
        first_boundary + 1,
        second_boundary - first_boundary,
        total_frames - second_boundary - 1,
    ];
    for callback_frames in callback_sizes {
        let mut returned = vec![f32::NAN; usize::try_from(callback_frames).expect("frames") * 2];
        let mut supplied = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            usize::try_from(token).expect("token") as *mut core::ffi::c_void,
            returned.as_mut_ptr(),
            callback_frames,
            2,
            &raw mut supplied,
        );
        assert_eq!(status, 0, "codec boundary must not underrun native output");
        assert_eq!(supplied, callback_frames, "no inserted callback silence");
        assert_eq!(
            tap.drain(usize::try_from(callback_frames).expect("frames")),
            returned,
            "native tap must capture callback-returned PCM"
        );
        callback_pcm.extend(returned);
    }
    assert_eq!(
        callback_pcm.len(),
        actual.len(),
        "native callback frame count"
    );
    let canonical_pcm: Vec<f32> = canonical
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    // Save sample-position evidence before the waveform assertion so hosted
    // CI can diagnose a callback regression without uploading audio data.
    // Explicitly model final packet padding rather than aligning each codec
    // boundary or silently accepting a shifted callback timeline.
    let mut expected_callback_pcm = canonical_pcm.clone();
    expected_callback_pcm.resize(actual.len(), 0.0);
    let first_mismatch = callback_pcm
        .iter()
        .zip(&expected_callback_pcm)
        .position(|(observed, reference)| observed.to_bits() != reference.to_bits())
        .or_else(|| {
            (callback_pcm.len() != expected_callback_pcm.len())
                .then_some(callback_pcm.len().min(expected_callback_pcm.len()))
        });
    let mismatch_count = callback_pcm
        .iter()
        .zip(&expected_callback_pcm)
        .filter(|(observed, reference)| observed.to_bits() != reference.to_bits())
        .count()
        + callback_pcm.len().abs_diff(expected_callback_pcm.len());
    let engine = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    let native_callbacks = silent_disco_audio_callback_count(engine);
    let native_underruns = silent_disco_audio_underrun_count(engine);
    let native_silence = silent_disco_audio_silence_filled_frames(engine);
    let (tap_frames, tap_loss, tap_callbacks) = tap.counters();
    let passed = mismatch_count == 0
        && tap_loss == 0
        && tap_frames == next_sample
        && tap_callbacks == 3
        && native_callbacks == 3
        && native_underruns == 0
        && native_silence == 0
        && boundaries == expected_boundaries;
    if let Some(directory) = std::env::var_os("SILENT_DISCO_PCM_EVIDENCE_DIR") {
        let commit = std::env::var("GITHUB_SHA")
            .ok()
            .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .unwrap_or_else(|| "local".to_owned());
        let path = std::path::PathBuf::from(directory);
        fs::create_dir_all(&path).expect("create encoded comparison evidence directory");
        let status = if passed { "PASS" } else { "FAIL" };
        let first_frame =
            first_mismatch.map_or("null".to_owned(), |sample| (sample / 2).to_string());
        let first_channel =
            first_mismatch.map_or("null".to_owned(), |sample| (sample % 2).to_string());
        let report = format!(
            "{{\n  \"schema\": 1,\n  \"status\": \"{status}\",\n  \"commit\": \"{commit}\",\n  \"capture\": \"native-rust-callback-after-real-tcp-udp-sockets\",\n  \"source_formats\": [\"wav\", \"flac\", \"mp3\"],\n  \"sample_rate\": 48000,\n  \"channels\": 2,\n  \"source_boundaries\": {expected_boundaries:?},\n  \"observed_boundaries\": {boundaries:?},\n  \"expected_frames_with_padding\": {},\n  \"captured_frames\": {},\n  \"callback_frame_chunks\": {callback_sizes:?},\n  \"callback_count\": {native_callbacks},\n  \"native_underrun_callbacks\": {native_underruns},\n  \"native_silence_filled_frames\": {native_silence},\n  \"tap_captured_frames\": {tap_frames},\n  \"tap_lost_frames\": {tap_loss},\n  \"tap_callback_count\": {tap_callbacks},\n  \"mismatched_samples\": {mismatch_count},\n  \"first_mismatch_frame\": {first_frame},\n  \"first_mismatch_channel\": {first_channel},\n  \"listener_scheduler_exercised\": false,\n  \"physical_dac_verified\": false\n}}\n",
            expected_callback_pcm.len() / 2,
            callback_pcm.len() / 2
        );
        fs::write(path.join("encoded-socket-callback.json"), report)
            .expect("write encoded comparison evidence");
    }
    assert!(
        passed,
        "encoded native callback comparison mismatch at frame {first_mismatch:?}"
    );
    assert_eq!(
        &callback_pcm[..canonical_pcm.len()],
        canonical_pcm,
        "native callback must match the independent WAV/FLAC/MP3 decode"
    );
    assert!(
        callback_pcm[canonical.len()..]
            .iter()
            .all(|sample| sample.to_bits() == 0),
        "native callback may contain only final-packet padding"
    );
    assert_eq!(silent_disco_audio_callback_count(engine), 3);
    assert_eq!(silent_disco_audio_underrun_count(engine), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    assert_eq!(tap.counters(), (next_sample, 0, 3));
    detach_callback_capture(u64::try_from(token).expect("token"));
    output.release();
    playlist.cancel().expect("close encoded playlist");
    listener.shutdown().expect("close socket listener");
    host.shutdown().expect("close socket host");
    fs::remove_dir_all(root).expect("remove generated sources");
}
