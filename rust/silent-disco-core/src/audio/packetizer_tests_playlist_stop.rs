//! Stop after actor reservation acknowledgment but before boundary enqueue.
use super::*;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn stop_after_reservation_ack_under_backpressure_discards_the_successor() {
    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    // Two full packets leave the last outgoing packet occupying the sole
    // channel slot when the producer tries to publish the next boundary.
    let first_path = write_bytes(&temp, "stop-first.wav", &pcm_wav_frames(48_000, 2, 480));
    let next_path = write_bytes(&temp, "stop-next.wav", &pcm_wav_frames(48_000, 2, 480));
    let decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("current decoder");
    let successor = StreamingPlaylistTrackFactory::new("stop-next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
    let ack_rx = Mutex::new(ack_rx);
    let reserver = Arc::new(move |boundary: &crate::protocol::TrackBoundary, _cancel| {
        entered_tx
            .send(boundary.first_sample_index.get())
            .expect("reservation entered");
        ack_rx
            .lock()
            .expect("ack mutex")
            .recv_timeout(TEST_TIMEOUT)
            .expect("ack reservation");
        Ok(())
    });
    let handle = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "stop-first",
        "First",
        vec![successor],
        SessionId::new("stop-after-ack-session").expect("session"),
        StreamId::new("stop-after-ack-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        reserver,
    )
    .expect("reserved packetizer");
    assert!(matches!(
        handle.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    assert_eq!(
        entered_rx.recv_timeout(TEST_TIMEOUT).expect("reservation"),
        480
    );
    assert_eq!(
        handle.statistics().0,
        1,
        "last outgoing packet occupies the channel"
    );
    let backpressure_before_ack = handle.statistics().2;
    ack_tx.send(()).expect("authorize next boundary");
    let start = Instant::now();
    while handle.statistics().2 == backpressure_before_ack && start.elapsed() < TEST_TIMEOUT {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        handle.statistics().2 > backpressure_before_ack,
        "the acknowledged boundary must be blocked behind outgoing audio"
    );

    // This models Stop after the actor accepted the reservation but before
    // the bounded producer could publish that boundary or successor PCM.
    handle.cancel();
    let mut tail = Vec::new();
    loop {
        match handle.recv_timeout(TEST_TIMEOUT) {
            Ok(frame) => tail.push(frame),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("cancelled producer did not exit"),
        }
    }
    let error = handle.join().expect_err("Stop cancels the producer");
    assert_eq!(error.kind, PacketizerWorkerErrorKind::Cancelled);
    assert!(
        matches!(
            tail.as_slice(),
            [ProtocolFrame::Audio(audio)] if audio.first_sample_index.get() == 240
        ),
        "only the already-buffered outgoing audio may remain after Stop"
    );
}

#[test]
fn stop_while_reservation_callback_is_pending_discards_stale_acknowledgment() {
    use std::sync::atomic::Ordering;

    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "pending-first.wav", &pcm_wav_frames(48_000, 2, 480));
    let next_path = write_bytes(&temp, "pending-next.wav", &pcm_wav_frames(48_000, 2, 480));
    let decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("current decoder");
    let successor = StreamingPlaylistTrackFactory::new("pending-next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
    let ack_rx = Mutex::new(ack_rx);
    let reserver = Arc::new(move |_boundary: &crate::protocol::TrackBoundary, cancel| {
        entered_tx.send(cancel).expect("reservation entered");
        ack_rx
            .lock()
            .expect("ack mutex")
            .recv_timeout(TEST_TIMEOUT)
            .expect("release reservation");
        // Simulate an actor acknowledgment that arrives after Stop.
        Ok(())
    });
    let handle = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "pending-first",
        "First",
        vec![successor],
        SessionId::new("stop-pending-session").expect("session"),
        StreamId::new("stop-pending-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        reserver,
    )
    .expect("reserved packetizer");

    assert!(matches!(
        handle.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    let cancellation = entered_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("producer reached actor reservation");
    assert!(!cancellation.load(Ordering::SeqCst));
    handle.cancel();
    assert!(cancellation.load(Ordering::SeqCst));
    ack_tx.send(()).expect("late reservation acknowledgment");

    let mut tail = Vec::new();
    loop {
        match handle.recv_timeout(TEST_TIMEOUT) {
            Ok(frame) => tail.push(frame),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("stopped producer did not exit"),
        }
    }
    let error = handle.join().expect_err("Stop cancels the producer");
    assert_eq!(error.kind, PacketizerWorkerErrorKind::Cancelled);
    assert!(
        matches!(
            tail.as_slice(),
            [ProtocolFrame::Audio(audio)] if audio.first_sample_index.get() == 240
        ),
        "a late reservation acknowledgment must not publish a boundary or successor PCM"
    );
}

// Replay can complete independently while the previous stream awaits a stale ack.
fn assert_replay_stream_is_contiguous(
    first_path: std::path::PathBuf,
    next_path: std::path::PathBuf,
) {
    let new_decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("new current decoder");
    let new_successor = StreamingPlaylistTrackFactory::new("next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("new successor");
    let new = StreamingPacketizeHandle::spawn_playlist_reserved(
        new_decoder,
        "current",
        "Current",
        vec![new_successor],
        SessionId::new("replay-session").expect("session"),
        StreamId::new("replay-stream").expect("replay stream"),
        MonotonicMillis::new(10_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        Arc::new(|_boundary: &crate::protocol::TrackBoundary, _cancel| Ok(())),
    )
    .expect("replay packetizer");
    let mut new_boundaries = Vec::new();
    let mut new_audio = Vec::new();
    loop {
        match new.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                new_boundaries.push((
                    boundary.stream_id.as_str().to_owned(),
                    boundary.entry_id,
                    boundary.first_sample_index.get(),
                ));
            }
            Ok(ProtocolFrame::Audio(audio)) => {
                new_audio.push((
                    audio.stream_id.as_str().to_owned(),
                    audio.sequence.get(),
                    audio.first_sample_index.get(),
                ));
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("replay producer stalled"),
        }
    }
    assert_eq!(
        new.join().expect("replay completes").state,
        PacketizerWorkerState::Completed
    );
    assert_eq!(
        new_boundaries,
        [("replay-stream".to_owned(), "next".to_owned(), 480)]
    );
    assert_eq!(
        new_audio,
        [
            ("replay-stream".to_owned(), 0, 0),
            ("replay-stream".to_owned(), 1, 240),
            ("replay-stream".to_owned(), 2, 480),
            ("replay-stream".to_owned(), 3, 720),
        ],
        "replay starts a new contiguous stream despite an outstanding old reservation"
    );
}

#[test]
fn stop_replay_with_the_same_queue_ids_rejects_the_retired_streams_late_ack() {
    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "replay-first.wav", &pcm_wav_frames(48_000, 2, 480));
    let next_path = write_bytes(&temp, "replay-next.wav", &pcm_wav_frames(48_000, 2, 480));
    let old_decoder =
        StreamingDecodeHandle::open(first_path.clone(), StreamingDecodeConfig::default())
            .expect("old current decoder");
    let old_successor_path = next_path.clone();
    let old_successor = StreamingPlaylistTrackFactory::new("next", "Next", move || {
        StreamingDecodeHandle::open(old_successor_path, StreamingDecodeConfig::default())
    })
    .expect("old successor");
    let (entered_tx, entered_rx) = mpsc::sync_channel::<()>(1);
    let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
    let ack_rx = Mutex::new(ack_rx);
    let old_reserver = Arc::new(move |_boundary: &crate::protocol::TrackBoundary, _cancel| {
        entered_tx.send(()).expect("old reservation entered");
        ack_rx
            .lock()
            .expect("old acknowledgment mutex")
            .recv_timeout(TEST_TIMEOUT)
            .expect("release old reservation");
        // A delayed success response from a retired stream must be harmless.
        Ok(())
    });
    let old = StreamingPacketizeHandle::spawn_playlist_reserved(
        old_decoder,
        "current",
        "Current",
        vec![old_successor],
        SessionId::new("replay-session").expect("session"),
        StreamId::new("retired-stream").expect("retired stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        old_reserver,
    )
    .expect("old packetizer");
    assert!(matches!(
        old.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    entered_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("old producer reached reservation");
    old.cancel();

    // The retired producer remains blocked while replay crosses its boundary.
    assert_replay_stream_is_contiguous(first_path, next_path);
    ack_tx.send(()).expect("late old-stream acknowledgment");
    let mut retired_tail = Vec::new();
    loop {
        match old.recv_timeout(TEST_TIMEOUT) {
            Ok(frame) => retired_tail.push(frame),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("retired producer did not stop"),
        }
    }
    assert_eq!(
        old.join().expect_err("retired stream is cancelled").kind,
        PacketizerWorkerErrorKind::Cancelled
    );
    assert!(
        matches!(
            retired_tail.as_slice(),
            [ProtocolFrame::Audio(audio)]
                if audio.stream_id.as_str() == "retired-stream"
                    && audio.first_sample_index.get() == 240
        ),
        "a late old-stream acknowledgment must not publish a boundary or successor PCM"
    );
}

#[test]
fn reserved_successor_becomes_final_when_later_track_is_removed_after_boundary_delivery() {
    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "remove-first.wav", &pcm_wav_frames(48_000, 2, 240));
    let next_path = write_bytes(&temp, "remove-next.wav", &pcm_wav_frames(48_000, 2, 4_800));
    let removed_path = write_bytes(&temp, "remove-third.wav", &pcm_wav_frames(48_000, 2, 480));
    let decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("first decoder");
    let next = StreamingPlaylistTrackFactory::new("next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let removed = StreamingPlaylistTrackFactory::new("removed", "Removed", move || {
        StreamingDecodeHandle::open(removed_path, StreamingDecodeConfig::default())
    })
    .expect("third factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "first",
        "First",
        vec![next, removed],
        SessionId::new("remove-later-session").expect("session"),
        StreamId::new("remove-later-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        Arc::new(|_boundary: &crate::protocol::TrackBoundary, _cancel| Ok(())),
    )
    .expect("reserved packetizer");
    let updater = packetizer.playlist_updater().expect("updater");

    // The reserved boundary is already visible to the consumer, but the
    // one-slot channel prevents the producer from reaching the third track.
    assert!(matches!(
        packetizer.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    let boundary = packetizer
        .recv_timeout(TEST_TIMEOUT)
        .expect("reserved boundary");
    assert!(matches!(
        boundary,
        ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))
            if boundary.entry_id == "next" && boundary.first_sample_index.get() == 240
    ));
    // The actor still projects the reserved current entry as the first
    // upcoming item. Its shadow must be stripped without re-opening it.
    let shadow = StreamingPlaylistTrackFactory::new("next", "Next", || {
        unreachable!("reserved successor must not be opened twice")
    })
    .expect("shadow");
    updater
        .replace_upcoming(vec![shadow])
        .expect("remove the later track without replacing the reserved one");

    let mut audio = vec![(0_u64, 0_u64)];
    let mut later_boundaries = Vec::new();
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Audio(frame)) => {
                audio.push((frame.sequence.get(), frame.first_sample_index.get()));
            }
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                later_boundaries.push(boundary.entry_id);
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("retargeted stream stalled"),
        }
    }
    let summary = packetizer
        .join()
        .expect("queue ends after reserved successor");
    assert_eq!(summary.state, PacketizerWorkerState::Completed);
    assert!(
        later_boundaries.is_empty(),
        "removed third track must not play"
    );
    assert_eq!(
        audio,
        (0_u64..21)
            .map(|packet| (packet, packet * 240))
            .collect::<Vec<_>>(),
        "the retained successor must finish with no stale third-track samples"
    );
    assert_eq!(summary.emitted_packets, 21);
}

#[test]
fn append_behind_reserved_successor_keeps_one_stream_and_both_boundaries() {
    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "append-first.wav", &pcm_wav_frames(48_000, 2, 240));
    let next_path = write_bytes(&temp, "append-next.wav", &pcm_wav_frames(48_000, 2, 4_800));
    let appended_path = write_bytes(&temp, "append-third.wav", &pcm_wav_frames(48_000, 2, 480));
    let decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("first decoder");
    let next = StreamingPlaylistTrackFactory::new("next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "first",
        "First",
        vec![next],
        SessionId::new("append-later-session").expect("session"),
        StreamId::new("append-later-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        Arc::new(|_boundary: &crate::protocol::TrackBoundary, _cancel| Ok(())),
    )
    .expect("reserved packetizer");
    let updater = packetizer.playlist_updater().expect("updater");

    assert!(matches!(
        packetizer.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    assert!(matches!(
        packetizer.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))
            ) if boundary.entry_id == "next" && boundary.first_sample_index.get() == 240
    ));

    // The actor still includes the reserved successor as the leading upcoming
    // entry. Its shadow must not cause a second decode or boundary.
    let shadow = StreamingPlaylistTrackFactory::new("next", "Next", || {
        unreachable!("reserved successor must not be opened twice")
    })
    .expect("shadow");
    let appended = StreamingPlaylistTrackFactory::new("appended", "Appended", move || {
        StreamingDecodeHandle::open(appended_path, StreamingDecodeConfig::default())
    })
    .expect("appended factory");
    updater
        .replace_upcoming(vec![shadow, appended])
        .expect("append after reserved successor");

    let mut audio = vec![(0_u64, 0_u64)];
    let mut boundaries = vec![("next".to_owned(), 240_u64)];
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Audio(frame)) => {
                audio.push((frame.sequence.get(), frame.first_sample_index.get()));
            }
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                boundaries.push((boundary.entry_id, boundary.first_sample_index.get()));
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("appended stream stalled"),
        }
    }
    let summary = packetizer.join().expect("appended playlist completes");
    assert_eq!(summary.state, PacketizerWorkerState::Completed);
    assert_eq!(
        boundaries,
        [("next".to_owned(), 240), ("appended".to_owned(), 5_040)]
    );
    assert_eq!(
        audio,
        (0_u64..23)
            .map(|packet| (packet, packet * 240))
            .collect::<Vec<_>>(),
        "append must preserve sequence and sample continuity without replaying next"
    );
    assert_eq!(summary.emitted_packets, 23);
}

#[test]
fn reorder_behind_reserved_successor_changes_only_later_boundaries() {
    let _guard = super::super::super::tests::audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let first_path = write_bytes(&temp, "reorder-first.wav", &pcm_wav_frames(48_000, 2, 240));
    let next_path = write_bytes(&temp, "reorder-next.wav", &pcm_wav_frames(48_000, 2, 4_800));
    let alpha_path = write_bytes(&temp, "reorder-alpha.wav", &pcm_wav_frames(48_000, 2, 480));
    let beta_path = write_bytes(&temp, "reorder-beta.wav", &pcm_wav_frames(48_000, 2, 720));
    let decoder = StreamingDecodeHandle::open(first_path, StreamingDecodeConfig::default())
        .expect("first decoder");
    let next = StreamingPlaylistTrackFactory::new("next", "Next", move || {
        StreamingDecodeHandle::open(next_path, StreamingDecodeConfig::default())
    })
    .expect("next factory");
    let initial_alpha_path = alpha_path.clone();
    let alpha = StreamingPlaylistTrackFactory::new("alpha", "Alpha", move || {
        StreamingDecodeHandle::open(initial_alpha_path, StreamingDecodeConfig::default())
    })
    .expect("alpha factory");
    let initial_beta_path = beta_path.clone();
    let beta = StreamingPlaylistTrackFactory::new("beta", "Beta", move || {
        StreamingDecodeHandle::open(initial_beta_path, StreamingDecodeConfig::default())
    })
    .expect("beta factory");
    let packetizer = StreamingPacketizeHandle::spawn_playlist_reserved(
        decoder,
        "first",
        "First",
        vec![next, alpha, beta],
        SessionId::new("reorder-later-session").expect("session"),
        StreamId::new("reorder-later-stream").expect("stream"),
        MonotonicMillis::new(5_000),
        StreamingPacketizeConfig {
            queue_capacity: 1,
            ..StreamingPacketizeConfig::default()
        },
        Arc::new(|_boundary: &crate::protocol::TrackBoundary, _cancel| Ok(())),
    )
    .expect("reserved packetizer");
    let updater = packetizer.playlist_updater().expect("updater");
    assert!(matches!(
        packetizer.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Audio(audio)) if audio.first_sample_index.get() == 0
    ));
    assert!(matches!(
        packetizer.recv_timeout(TEST_TIMEOUT),
        Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))
            ) if boundary.entry_id == "next" && boundary.first_sample_index.get() == 240
    ));

    // The first successor has already been reserved by the actor. Reorder
    // only the later entries while the bounded sender is backpressured.
    let shadow = StreamingPlaylistTrackFactory::new("next", "Next", || {
        unreachable!("the reserved successor must not be reopened")
    })
    .expect("shadow");
    let beta = StreamingPlaylistTrackFactory::new("beta", "Beta", move || {
        StreamingDecodeHandle::open(beta_path, StreamingDecodeConfig::default())
    })
    .expect("reordered beta");
    let alpha = StreamingPlaylistTrackFactory::new("alpha", "Alpha", move || {
        StreamingDecodeHandle::open(alpha_path, StreamingDecodeConfig::default())
    })
    .expect("reordered alpha");
    updater
        .replace_upcoming(vec![shadow, beta, alpha])
        .expect("reorder only entries behind the reserved successor");

    let mut audio = vec![(0_u64, 0_u64)];
    let mut boundaries = vec![("next".to_owned(), 240_u64)];
    loop {
        match packetizer.recv_timeout(TEST_TIMEOUT) {
            Ok(ProtocolFrame::Audio(frame)) => {
                audio.push((frame.sequence.get(), frame.first_sample_index.get()));
            }
            Ok(ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary))) => {
                boundaries.push((boundary.entry_id, boundary.first_sample_index.get()));
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("reordered stream stalled"),
        }
    }
    let summary = packetizer.join().expect("reordered playlist completes");
    assert_eq!(summary.state, PacketizerWorkerState::Completed);
    assert_eq!(
        boundaries,
        [
            ("next".to_owned(), 240),
            ("beta".to_owned(), 5_040),
            ("alpha".to_owned(), 5_760),
        ]
    );
    assert_eq!(
        audio,
        (0_u64..26)
            .map(|packet| (packet, packet * 240))
            .collect::<Vec<_>>(),
        "reordering later tracks must preserve one contiguous packet timeline"
    );
    assert_eq!(summary.emitted_packets, 26);
}
