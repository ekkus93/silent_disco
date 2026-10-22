//! Gated playlist regression: an upcoming edit beats producer reservation.
use super::*;

#[test]
fn authoritative_playlist_edit_during_backpressure_never_reserves_stale_successor() {
    let _guard = super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let current_path = write_bytes(
        &temp,
        "edit-current.wav",
        &pcm_wav_frames(48_000, 2, 4_800),
    );
    let stale_path = write_bytes(&temp, "edit-stale.wav", &pcm_wav_frames(48_000, 2, 480));
    let replacement_path = write_bytes(
        &temp,
        "edit-replacement.wav",
        &pcm_wav_frames(48_000, 2, 480),
    );
    let decoder = StreamingDecodeHandle::open(current_path, StreamingDecodeConfig::default())
        .expect("current decoder");
    let (opened_tx, opened_rx) = std::sync::mpsc::sync_channel(1);
    let stale = StreamingPlaylistTrackFactory::new("stale-next", "Stale", move || {
        let opened = StreamingDecodeHandle::open(
            stale_path.clone(),
            StreamingDecodeConfig::default(),
        )?;
        opened_tx.send(()).expect("notify staged decoder");
        Ok(opened)
    })
    .expect("stale factory");
    let reserved = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let reserved_log = std::sync::Arc::clone(&reserved);
    let reserve = std::sync::Arc::new(
        move |boundary: &crate::protocol::TrackBoundary, _cancel| {
            reserved_log
                .lock()
                .expect("reservation log")
                .push((boundary.entry_id.clone(), boundary.first_sample_index.get()));
            Ok(())
        },
    );
    let packetizer = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "current",
        "Current",
        vec![stale],
        SessionId::new("reserved-edit-session").expect("session"),
        StreamId::new("reserved-edit-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        reserve,
    )
    .expect("reserved playlist");

    // The consumer is deliberately not draining the single frame slot.
    // The outgoing track needs 20 audio packets, so the producer cannot
    // reach its successor reservation while this edit is submitted.
    opened_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("the stale decoder was actually prebuffered");
    let replacement =
        StreamingPlaylistTrackFactory::new("replacement", "Replacement", move || {
            StreamingDecodeHandle::open(replacement_path, StreamingDecodeConfig::default())
        })
        .expect("replacement factory");
    packetizer
        .playlist_updater()
        .expect("live updater")
        .replace_upcoming(vec![replacement])
        .expect("edit wins while outgoing audio is backpressured");

    let mut boundaries = Vec::new();
    let mut audio_indices = Vec::new();
    let mut sequences = Vec::new();
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                boundaries.push((boundary.entry_id, boundary.first_sample_index.get()));
            }
            Ok(ProtocolFrame::Audio(audio)) => {
                audio_indices.push(audio.first_sample_index.get());
                sequences.push(audio.sequence.get());
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("edited, reserved playlist stalled")
            }
        }
    }
    let summary = packetizer.join().expect("edited playlist completes");
    assert_eq!(summary.state, PacketizerWorkerState::Completed);
    assert_eq!(boundaries, [("replacement".to_owned(), 4_800)]);
    assert_eq!(
        *reserved.lock().expect("reservation log"),
        [("replacement".to_owned(), 4_800)],
        "the stale entry must not reach the authority callback"
    );
    assert_eq!(audio_indices.len(), 22);
    assert_eq!(audio_indices.first(), Some(&0));
    assert_eq!(audio_indices.last(), Some(&5_040));
    for window in audio_indices.windows(2) {
        assert_eq!(window[1], window[0] + 240, "no sample gap or overlap");
    }
    for (sequence, observed) in sequences.iter().enumerate() {
        assert_eq!(*observed, sequence as u64, "sequence remains monotonic");
    }
}
