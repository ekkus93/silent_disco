//! Continuous playlist packetizer boundary, retargeting, and lookahead tests.

use super::super::DecodeWorkerState;
use super::*;

#[test]
fn playlist_packetizer_emits_exact_boundary_on_one_contiguous_packet_timeline() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "first.wav", &pcm_wav_frames(48_000, 2, 241));
    let second_path = write_bytes(&temp, "second.wav", &pcm_wav_frames(48_000, 2, 497));
    let first_decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("open first track");
    let second_factory =
        StreamingPlaylistTrackFactory::new("entry-second", "Second track", move || {
            StreamingDecodeHandle::open(second_path, StreamingDecodeConfig::default())
        })
        .expect("second track factory");
    let handle = StreamingPacketizeHandle::spawn_playlist(
        first_decoder,
        "entry-first",
        "First track",
        vec![second_factory],
        SessionId::new("session-playlist").expect("session id"),
        StreamId::new("stream-playlist").expect("stream id"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig::default(),
    )
    .expect("start playlist packetizer");

    let mut frames = Vec::new();
    loop {
        match handle.recv_timeout(TEST_TIMEOUT) {
            Ok(frame) => frames.push(frame),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("playlist packetizer did not complete in time")
            }
        }
    }
    let summary = handle.join().expect("playlist completed");
    let audio = frames
        .iter()
        .filter_map(|frame| match frame {
            ProtocolFrame::Audio(datagram) => Some(datagram),
            _ => None,
        })
        .collect::<Vec<_>>();
    let boundary = frames
        .iter()
        .position(|frame| {
            matches!(
                frame,
                ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))
                    if boundary.entry_id == "entry-second"
                        && boundary.first_sample_index.get() == 241
            )
        })
        .expect("sample-indexed second-track boundary");

    assert_eq!(summary.state, PacketizerWorkerState::Completed);
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.sequence.get())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(
        audio
            .iter()
            .map(|packet| packet.first_sample_index.get())
            .collect::<Vec<_>>(),
        [0, 240, 480, 720]
    );
    assert!(boundary < frames.iter().position(|frame| matches!(frame, ProtocolFrame::Audio(packet) if packet.first_sample_index.get() == 240)).expect("boundary-spanning packet"));
    assert_eq!(audio[0].samples_per_packet, 240);
    assert_eq!(summary.emitted_packets, 5); // one boundary control and four audio packets
}

#[test]
fn playlist_update_cancels_the_staged_successor_and_emits_only_the_replacement_boundary() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let current_path = write_bytes(
        &temp,
        "update-current.wav",
        &pcm_wav_frames(48_000, 2, 48_000),
    );
    let stale_path = write_bytes(
        &temp,
        "update-stale.wav",
        &pcm_wav_frames(48_000, 2, 48_000),
    );
    let replacement_path = write_bytes(
        &temp,
        "update-replacement.wav",
        &pcm_wav_frames(48_000, 2, 480),
    );
    let current = StreamingDecodeHandle::open(current_path, StreamingDecodeConfig::default())
        .expect("open current track");
    let stale = StreamingPlaylistTrackFactory::new("entry-stale", "Stale track", move || {
        StreamingDecodeHandle::open(stale_path, StreamingDecodeConfig::default())
    })
    .expect("stale factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist(
        current,
        "entry-current",
        "Current track",
        vec![stale],
        SessionId::new("session-playlist-update").expect("session ID"),
        StreamId::new("stream-playlist-update").expect("stream ID"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
    )
    .expect("start playlist");
    let updater = packetizer.playlist_updater().expect("playlist updater");
    let replacement =
        StreamingPlaylistTrackFactory::new("entry-replacement", "Replacement track", move || {
            StreamingDecodeHandle::open(replacement_path, StreamingDecodeConfig::default())
        })
        .expect("replacement factory");
    updater
        .replace_upcoming(vec![replacement])
        .expect("replace upcoming playlist");

    let mut frames = Vec::new();
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(frame) => frames.push(frame),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("updated playlist stalled"),
        }
    }
    packetizer.join().expect("updated playlist completes");
    let boundaries = frames
        .iter()
        .filter_map(|frame| match frame {
            ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)) => Some(boundary),
            _ => None,
        })
        .map(|boundary| boundary.entry_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(boundaries, ["entry-replacement"]);
}

#[test]
fn playlist_decoder_prebuffers_the_next_source_before_current_source_finishes() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let current_path = write_bytes(&temp, "current.wav", &pcm_wav(48_000, 2, 10));
    let next_path = write_bytes(&temp, "next.wav", &pcm_wav(48_000, 2, 10));
    let current = StreamingDecodeHandle::open(current_path, StreamingDecodeConfig::default())
        .expect("open current track");
    let current_statistics = current.statistics_reader();
    let (next_statistics_sender, next_statistics_receiver) = std::sync::mpsc::sync_channel(1);
    let next_factory = StreamingPlaylistTrackFactory::new("entry-next", "Next track", move || {
        let decoder = StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())?;
        next_statistics_sender
            .send(decoder.statistics_reader())
            .expect("test still waiting for upcoming decoder");
        Ok(decoder)
    })
    .expect("next track factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist(
        current,
        "entry-current",
        "Current track",
        vec![next_factory],
        SessionId::new("session-lookahead").expect("session ID"),
        StreamId::new("stream-lookahead").expect("stream ID"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
    )
    .expect("start playlist packetizer");

    let next_statistics = next_statistics_receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("next decoder opened before the current track finished");
    let deadline = std::time::Instant::now() + TEST_TIMEOUT;
    loop {
        let next = next_statistics.snapshot();
        let current = current_statistics.snapshot();
        if next.queued_chunks > 0 {
            assert!(next.queued_chunks <= next.queue_capacity_chunks);
            assert_eq!(current.state, DecodeWorkerState::Running);
            assert!(current.emitted_frames < 480_000);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "next source did not prebuffer"
        );
        std::thread::yield_now();
    }

    let cancelled = packetizer
        .cancel_and_join()
        .expect_err("the deliberately stopped lookahead worker is cancelled");
    assert_eq!(cancelled.kind, PacketizerWorkerErrorKind::Cancelled);
}

#[test]
fn an_empty_update_while_current_audio_is_backpressured_does_not_leak_a_boundary() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let current_path = write_bytes(&temp, "clear-current.wav", &pcm_wav_frames(48_000, 2, 48_000));
    let next_path = write_bytes(&temp, "clear-next.wav", &pcm_wav_frames(48_000, 2, 480));
    let decoder = StreamingDecodeHandle::open(current_path, StreamingDecodeConfig::default())
        .expect("current decoder");
    let next = StreamingPlaylistTrackFactory::new("entry-removed", "Removed", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist(
        decoder,
        "entry-current",
        "Current",
        vec![next],
        SessionId::new("session-clear-backpressure").expect("session"),
        StreamId::new("stream-clear-backpressure").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
    )
    .expect("playlist");
    packetizer
        .playlist_updater()
        .expect("updater")
        .replace_upcoming(Vec::new())
        .expect("remove upcoming before consumer drains");
    let mut saw_removed = false;
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                saw_removed |= boundary.entry_id == "entry-removed";
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("clear test stalled"),
        }
    }
    // The update is consumed while the current track is still packetizing;
    // no stale successor boundary is ever allowed to escape.
    packetizer.join().expect("playlist completed after clearing next track");
    assert!(!saw_removed);
}

#[test]
fn empty_final_contiguous_chunk_flushes_only_the_retained_partial_packet() {
    let mut packetizer = Packetizer::new(
        SessionId::new("session-final-removal").expect("session"),
        StreamId::new("stream-final-removal").expect("stream"),
        AudioFormat::CANONICAL,
        5,
        MonotonicMillis::new(5_000),
    )
    .expect("packetizer");
    let first = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format: AudioFormat::CANONICAL,
            first_sample_index: SampleIndex::new(0),
            frames: vec![42; 241 * 2],
            end_of_stream: false,
        })
        .expect("non-final outgoing source");
    assert_eq!(first.frames.len(), 1);
    assert!(!first.end_of_stream);
    let final_outcome = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format: AudioFormat::CANONICAL,
            first_sample_index: SampleIndex::new(0),
            frames: Vec::new(),
            end_of_stream: true,
        })
        .expect("finish without another source");
    assert!(final_outcome.end_of_stream);
    assert_eq!(final_outcome.frames.len(), 1);
    let ProtocolFrame::Audio(tail) = &final_outcome.frames[0] else {
        panic!("expected final padded audio packet");
    };
    assert_eq!(tail.first_sample_index.get(), 240);
    assert_eq!(tail.sequence.get(), 1);
    assert_eq!(packetizer.next_contiguous_sample_index(), 241);
}

#[test]
fn playlist_live_updater_rejects_current_entry_before_and_after_promotion() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(
        &temp,
        "identity-first.wav",
        &pcm_wav_frames(48_000, 2, 241),
    );
    let second_path = write_bytes(
        &temp,
        "identity-second.wav",
        &pcm_wav_frames(48_000, 2, 480),
    );
    let first = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("first decoder");
    let second = StreamingPlaylistTrackFactory::new("entry-second", "Second", move || {
        StreamingDecodeHandle::open(second_path, StreamingDecodeConfig::default())
    })
    .expect("second factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist(
        first,
        "entry-first",
        "First",
        vec![second],
        SessionId::new("session-current-id").expect("session"),
        StreamId::new("stream-current-id").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig::default(),
    )
    .expect("playlist");
    let updater = packetizer.playlist_updater().expect("updater");
    let duplicate = |id: &str| {
        StreamingPlaylistTrackFactory::new(id, "Duplicate", || {
            unreachable!("duplicate entry must be rejected before opening")
        })
        .expect("factory")
    };
    let error = updater
        .replace_upcoming(vec![duplicate("entry-first")])
        .expect_err("the initially current entry cannot be requeued");
    assert_eq!(error.kind, PacketizerWorkerErrorKind::Packetize);
    assert!(error.message.contains("duplicate entry IDs"));

    let mut saw_second = false;
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                assert_eq!(boundary.entry_id, "entry-second");
                saw_second = true;
                let error = updater
                    .replace_upcoming(vec![duplicate("entry-second")])
                    .expect_err("the promoted current entry cannot be requeued");
                assert_eq!(error.kind, PacketizerWorkerErrorKind::Packetize);
                assert!(error.message.contains("duplicate entry IDs"));
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("identity test stalled"),
        }
    }
    assert!(saw_second);
    packetizer.join().expect("rejected edits do not interrupt playback");
}
