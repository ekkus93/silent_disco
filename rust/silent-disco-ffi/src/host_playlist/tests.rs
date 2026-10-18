use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::host_control::{FfiBridgeError, FfiCoreHandle, FfiCoreNotification, FfiCoreObserver};
use silent_disco_core::audio::PacketizerWorkerErrorKind;
use silent_disco_core::domain::{SampleIndex, SessionId, StreamId};
use silent_disco_core::protocol::TrackBoundary;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

struct QuietObserver;

impl FfiCoreObserver for QuietObserver {
    fn on_notification(&self, _: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        Ok(())
    }
}

#[test]
fn authoritative_ffi_playlist_rejects_closed_actor_before_opening_source() {
    let actor = FfiCoreHandle::open("closed-playlist-actor".to_owned(), Arc::new(QuietObserver))
        .expect("actor opens");
    actor.shutdown().expect("actor shuts down");
    let error = FfiHostPlaylistStreamer::open_authoritative(
        "closed-session".to_owned(),
        "closed-stream".to_owned(),
        0,
        vec![],
        actor,
    )
    .expect_err("closed actor must be rejected before opening sources");
    assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
}

#[test]
fn reservation_reply_failure_preserves_cancellation_classification() {
    use std::sync::atomic::Ordering;

    let signal = AtomicBool::new(false);
    let rejected = super::playlist_reservation_error("actor rejected edit", &signal);
    assert_eq!(rejected.kind, PacketizerWorkerErrorKind::Packetize);
    signal.store(true, Ordering::Release);
    let interrupted = super::playlist_reservation_error("actor reply interrupted", &signal);
    assert_eq!(interrupted.kind, PacketizerWorkerErrorKind::Cancelled);
    assert!(interrupted.message.contains("cancelled"));
}

#[test]
fn cancelled_ffi_reservation_never_waits_for_playing() {
    let authority = FfiCoreHandle::open(
        "cancelled-playlist-actor".to_owned(),
        Arc::new(QuietObserver),
    )
    .expect("actor opens");
    let actor = authority.actor_handle().expect("live actor");
    let boundary = TrackBoundary {
        session_id: SessionId::new("cancelled-session").expect("session ID"),
        stream_id: StreamId::new("cancelled-stream").expect("stream ID"),
        first_sample_index: SampleIndex::new(960),
        entry_id: "next-track".to_owned(),
        display_name: "Next".to_owned(),
    };
    let cancellation = Arc::new(AtomicBool::new(true));
    let error = super::reserve_authoritative_boundary(&actor, &boundary, cancellation)
        .expect_err("cancelled producer must not wait for playback");
    assert_eq!(error.kind, PacketizerWorkerErrorKind::Cancelled);
    authority.shutdown().expect("actor shuts down");
}

#[test]
fn direct_ffi_playlist_cannot_bypass_authoritative_queue_entry_limits() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-queue-bounds-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let source = root.join("source.wav");
    fs::write(&source, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("valid source");
    let staged_path = source.to_string_lossy().into_owned();
    let make_source = |id: String| FfiPlaylistSource {
        entry_id: id,
        display_name: "Track".to_owned(),
        staged_path: staged_path.clone(),
    };

    let oversized = (0..=100)
        .map(|index| make_source(format!("entry-{index}")))
        .collect();
    let error = FfiHostPlaylistStreamer::open(
        "session-ffi-overflow".to_owned(),
        "stream-ffi-overflow".to_owned(),
        5_000,
        oversized,
    )
    .expect_err("direct FFI open must enforce the 100-entry queue bound");
    assert!(error.to_string().contains("100-entry"));

    let duplicate = vec![
        make_source("same-entry".to_owned()),
        make_source("same-entry".to_owned()),
    ];
    let error = FfiHostPlaylistStreamer::open(
        "session-ffi-duplicate".to_owned(),
        "stream-ffi-duplicate".to_owned(),
        5_000,
        duplicate,
    )
    .expect_err("direct FFI open must reject duplicate entry IDs");
    assert!(error.to_string().contains("duplicate entry IDs"));

    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-live-bounds".to_owned(),
        "stream-ffi-live-bounds".to_owned(),
        5_000,
        vec![make_source("current".to_owned())],
    )
    .expect("valid single-track stream");
    let too_many_upcoming = (0..100)
        .map(|index| make_source(format!("upcoming-{index}")))
        .collect();
    let error = stream
        .replace_upcoming(too_many_upcoming)
        .expect_err("live update must reserve one current-track slot");
    assert!(error.to_string().contains("100-entry"));
    let error = stream
        .replace_upcoming(vec![
            make_source("duplicate".to_owned()),
            make_source("duplicate".to_owned()),
        ])
        .expect_err("live update must reject duplicate upcoming IDs");
    assert!(error.to_string().contains("duplicate entry IDs"));
    stream.cancel().expect("cancel stream");
    let error = stream
        .replace_upcoming(Vec::new())
        .expect_err("closed stream must not silently accept queue edits");
    assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn ffi_update_reports_closed_when_producer_finished_but_end_not_consumed() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-unobserved-eof-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let source = root.join("short.wav");
    fs::write(&source, wav_pcm16(48_000, 2, 241, 3_000)).expect("source");
    let stream = FfiHostPlaylistStreamer::open(
        "session-unobserved-eof".to_owned(),
        "stream-unobserved-eof".to_owned(),
        5_000,
        vec![FfiPlaylistSource {
            entry_id: "current".to_owned(),
            display_name: "Short".to_owned(),
            staged_path: source.to_string_lossy().into_owned(),
        }],
    )
    .expect("open short stream");
    // Join the short producer without calling next_frame. This leaves
    // the UniFFI object's consumer-observed closed flag false.
    let worker = stream.inner.lock().expect("lock").take().expect("worker");
    worker.join().expect("producer completed");
    assert!(!stream.closed.load(std::sync::atomic::Ordering::Acquire));
    let error = stream
        .replace_upcoming(Vec::new())
        .expect_err("completed producer must not accept a phantom update");
    assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn ffi_playlist_stream_keeps_audio_sequence_and_sample_clock_across_boundary() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-playlist-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    fs::write(&first, wav_pcm16(48_000, 2, 240, 3_000)).expect("first source");
    fs::write(&second, wav_pcm16(44_100, 1, 441, 6_000)).expect("second source");

    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-playlist".to_owned(),
        "stream-ffi-playlist".to_owned(),
        5_000,
        vec![
            FfiPlaylistSource {
                entry_id: "entry-first".to_owned(),
                display_name: "First".to_owned(),

                staged_path: first.to_string_lossy().into_owned(),
            },
            FfiPlaylistSource {
                entry_id: "entry-second".to_owned(),
                display_name: "Second".to_owned(),
                staged_path: second.to_string_lossy().into_owned(),
            },
        ],
    )
    .expect("open FFI playlist");
    let start = stream.stream_start();
    assert_eq!(start.current_entry_id.as_deref(), Some("entry-first"));
    assert_eq!(start.current_display_name.as_deref(), Some("First"));

    let mut sequence = Vec::new();
    let mut audio_samples = Vec::new();
    let mut audio_frame_counts = Vec::new();
    let mut saw_boundary = false;
    let mut boundary_position = None;
    let mut next_track_audio_position = None;
    let mut frame_position = 0;
    loop {
        match stream.next_frame(2_000).expect("next playlist frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence: packet_sequence,
                first_sample_index,
                payload,
                ..
            }) => {
                if saw_boundary && next_track_audio_position.is_none() {
                    next_track_audio_position = Some(frame_position);
                }
                sequence.push(packet_sequence);
                audio_samples.push(first_sample_index);
                audio_frame_counts.push(payload.len() as u64 / 4);
                frame_position += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                assert_eq!(entry_id, "entry-second");
                assert_eq!(first_sample_index, 240);
                saw_boundary = true;
                boundary_position = Some(frame_position);
                frame_position += 1;
            }
            Some(FfiPlaylistFrame::End) => break,
            None => panic!("playlist timed out"),
        }
    }
    assert!(saw_boundary);
    assert_eq!(audio_samples.first(), Some(&0));
    for index in 1..audio_samples.len() {
        assert_eq!(
            audio_samples[index],
            audio_samples[index - 1] + audio_frame_counts[index - 1],
            "playlist output must have no inserted or overlapped samples",
        );
    }
    assert_eq!(
        audio_samples
            .iter()
            .zip(&audio_frame_counts)
            .map(|(first, count)| first + count)
            .next_back(),
        Some(720),
    );
    assert!(boundary_position.expect("boundary") < next_track_audio_position.expect("next audio"));
    assert!(sequence.windows(2).all(|pair| pair[1] == pair[0] + 1));
    let error = stream
        .replace_upcoming(Vec::new())
        .expect_err("completed stream must not accept queue edits");
    assert!(matches!(error, super::FfiHostPlaylistError::Closed(_)));
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn ffi_playlist_can_replace_the_upcoming_source_before_its_boundary() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-playlist-update-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let current = root.join("current.wav");
    let stale = root.join("stale.wav");
    let replacement = root.join("replacement.wav");
    fs::write(&current, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("current source");
    fs::write(&stale, wav_pcm16(48_000, 2, 480, 4_000)).expect("stale source");
    fs::write(&replacement, wav_pcm16(48_000, 2, 480, 6_000)).expect("replacement source");

    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-update".to_owned(),
        "stream-ffi-update".to_owned(),
        5_000,
        vec![
            FfiPlaylistSource {
                entry_id: "entry-current".to_owned(),
                display_name: "Current".to_owned(),
                staged_path: current.to_string_lossy().into_owned(),
            },
            FfiPlaylistSource {
                entry_id: "entry-stale".to_owned(),
                display_name: "Stale".to_owned(),
                staged_path: stale.to_string_lossy().into_owned(),
            },
        ],
    )
    .expect("open FFI playlist");
    let duplicate_error = stream
        .replace_upcoming(vec![FfiPlaylistSource {
            entry_id: "entry-current".to_owned(),
            display_name: "Duplicate current".to_owned(),
            staged_path: current.to_string_lossy().into_owned(),
        }])
        .expect_err("UniFFI must reject a live update that duplicates the active track");
    assert!(duplicate_error.to_string().contains("duplicate entry IDs"));
    stream
        .replace_upcoming(vec![FfiPlaylistSource {
            entry_id: "entry-replacement".to_owned(),
            display_name: "Replacement".to_owned(),
            staged_path: replacement.to_string_lossy().into_owned(),
        }])
        .expect("retarget upcoming queue");

    let mut boundaries = Vec::new();
    loop {
        match stream.next_frame(2_000).expect("read playlist") {
            Some(FfiPlaylistFrame::TrackBoundary { entry_id, .. }) => boundaries.push(entry_id),
            Some(FfiPlaylistFrame::End) => break,
            Some(FfiPlaylistFrame::Audio { .. }) => {}
            None => panic!("updated FFI playlist stalled"),
        }
    }
    assert_eq!(boundaries, ["entry-replacement"]);
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn failed_upcoming_source_finishes_current_track_and_names_failed_item() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-playlist-failure-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let current = root.join("current.wav");
    fs::write(&current, wav_pcm16(48_000, 2, 241, 3_000)).expect("current source");

    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-failure".to_owned(),
        "stream-ffi-failure".to_owned(),
        5_000,
        vec![
            FfiPlaylistSource {
                entry_id: "entry-current".to_owned(),
                display_name: "Current song".to_owned(),
                staged_path: current.to_string_lossy().into_owned(),
            },
            FfiPlaylistSource {
                entry_id: "entry-unavailable".to_owned(),
                display_name: "Unavailable song".to_owned(),
                staged_path: root.join("unavailable.wav").to_string_lossy().into_owned(),
            },
        ],
    )
    .expect("start playlist with lazy upcoming source");

    let mut audio_frames = 0;
    let mut saw_boundary = false;
    let error = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio { .. })) => audio_frames += 1,
            Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => saw_boundary = true,
            Ok(Some(FfiPlaylistFrame::End)) => panic!("failed playlist ended cleanly"),
            Ok(None) => {}
            Err(error) => break error,
        }
    };

    assert!(audio_frames > 0, "the valid current track must be emitted");
    assert!(
        !saw_boundary,
        "a failed source must not be announced as current"
    );
    assert!(error.to_string().contains("Unavailable song"));
    let _ = stream.cancel();
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn unsupported_upcoming_source_is_an_item_specific_failure() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-unsupported-playlist-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let current = root.join("current.wav");
    let unsupported = root.join("unsupported.ogg");
    fs::write(&current, wav_pcm16(48_000, 2, 241, 3_000)).expect("current source");
    fs::write(&unsupported, b"OggSunsupported playlist fixture").expect("unsupported source");

    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-unsupported".to_owned(),
        "stream-ffi-unsupported".to_owned(),
        5_000,
        vec![
            FfiPlaylistSource {
                entry_id: "entry-current".to_owned(),
                display_name: "Current song".to_owned(),
                staged_path: current.to_string_lossy().into_owned(),
            },
            FfiPlaylistSource {
                entry_id: "entry-unsupported".to_owned(),
                display_name: "Unsupported song".to_owned(),
                staged_path: unsupported.to_string_lossy().into_owned(),
            },
        ],
    )
    .expect("start playlist with lazy upcoming source");

    let mut audio_frames = 0;
    let error = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio { .. })) => audio_frames += 1,
            Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                panic!("unsupported source must not be announced as current")
            }
            Ok(Some(FfiPlaylistFrame::End)) => panic!("failed playlist ended cleanly"),
            Ok(None) => {}
            Err(error) => break error,
        }
    };

    assert!(audio_frames > 0, "the valid current track must be emitted");
    assert!(error.to_string().contains("Unsupported song"));
    let _ = stream.cancel();
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn decoder_read_failure_after_open_is_reported_instead_of_clean_queue_end() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-late-decode-failure-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let current = root.join("current.wav");
    // Keep enough audio beyond the decoder's bounded read-ahead window
    // that truncation after the first delivered frame forces a later read.
    fs::write(&current, wav_pcm16(48_000, 2, 480_000, 3_000)).expect("current source");
    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-late-failure".to_owned(),
        "stream-ffi-late-failure".to_owned(),
        5_000,
        vec![FfiPlaylistSource {
            entry_id: "entry-current".to_owned(),
            display_name: "Current song".to_owned(),
            staged_path: current.to_string_lossy().into_owned(),
        }],
    )
    .expect("source opens before the read failure");

    let first = stream.next_frame(2_000).expect("first frame result");
    assert!(matches!(first, Some(FfiPlaylistFrame::Audio { .. })));
    fs::OpenOptions::new()
        .write(true)
        .open(&current)
        .expect("open active source for truncation")
        .set_len(44)
        .expect("truncate active source after playback begins");

    let failure = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio { .. }) | None) => {}
            Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                panic!("failed active source must not announce a later track")
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                panic!("decoder read failure must not look like clean queue completion")
            }
            Err(error) => break error,
        }
    };
    assert!(failure.to_string().contains("Current song"));
    let _ = stream.cancel();
    fs::remove_dir_all(root).expect("remove temp directory");
}

#[test]
fn ffi_cancel_signals_producer_before_waiting_for_frame_consumer_lock() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-cancel-lock-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let source = root.join("current.wav");
    fs::write(&source, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("source");
    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-cancel-lock".to_owned(),
        "stream-ffi-cancel-lock".to_owned(),
        5_000,
        vec![FfiPlaylistSource {
            entry_id: "current".to_owned(),
            display_name: "Current".to_owned(),
            staged_path: source.to_string_lossy().into_owned(),
        }],
    )
    .expect("open playlist");

    // Simulate next_frame holding the consumer lock. Stop must signal the
    // producer without waiting for its receive timeout or queued frames.
    let held_consumer = stream.inner.lock().expect("consumer lock");
    let cancellation = Arc::clone(&stream.cancellation);
    let (started_tx, started_rx) = sync_channel(1);
    let stop_stream = Arc::clone(&stream);
    let stopper = std::thread::spawn(move || {
        started_tx.send(()).expect("signal stopper");
        stop_stream.cancel()
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stopper started");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !cancellation.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let signalled_before_unlock = cancellation.load(Ordering::Acquire);
    drop(held_consumer);
    stopper
        .join()
        .expect("stopper thread")
        .expect("cancel playlist");
    assert!(
        signalled_before_unlock,
        "cancellation must not wait for the frame-consumer mutex"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn ffi_stop_does_not_deliver_buffered_audio_to_a_waiting_consumer() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-stop-frame-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let source = root.join("current.wav");
    fs::write(&source, wav_pcm16(48_000, 2, 48_000, 3_000)).expect("source");
    let stream = FfiHostPlaylistStreamer::open(
        "session-ffi-stop-frame".to_owned(),
        "stream-ffi-stop-frame".to_owned(),
        5_000,
        vec![FfiPlaylistSource {
            entry_id: "current".to_owned(),
            display_name: "Current".to_owned(),
            staged_path: source.to_string_lossy().into_owned(),
        }],
    )
    .expect("open playlist");

    let consumer_lock = stream.inner.lock().expect("consumer mutex");
    let (started_tx, started_rx) = sync_channel(1);
    let consumer_stream = Arc::clone(&stream);
    let consumer = std::thread::spawn(move || {
        started_tx.send(()).expect("consumer started");
        consumer_stream.next_frame(50)
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("consumer started");
    // Simulate a concurrent host Stop before releasing the frame mutex.
    stream.closed.store(true, Ordering::Release);
    stream.cancellation.store(true, Ordering::Release);
    drop(consumer_lock);
    let frame = consumer
        .join()
        .expect("consumer thread")
        .expect("frame poll");
    assert!(
        frame.is_none(),
        "stopped stream must not expose queued audio"
    );
    stream.cancel().expect("finalize Stop");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn ffi_stop_interrupts_long_consumer_timeout_at_reserved_boundary() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-cancel-poll-{unique}"));
    fs::create_dir_all(&root).expect("temp directory");
    let wav = root.join("source.wav");
    fs::write(&wav, wav_pcm16(48_000, 2, 4_800, 3_000)).expect("source");
    let path = wav.to_string_lossy().into_owned();
    let sources = ["one", "two"]
        .into_iter()
        .map(|id| FfiPlaylistSource {
            entry_id: id.to_owned(),
            display_name: id.to_owned(),
            staged_path: path.clone(),
        })
        .collect();

    let (entered_tx, entered_rx) = sync_channel(1);
    let reservation = Arc::new(
        move |_: &silent_disco_core::protocol::TrackBoundary,
              cancelled: Arc<std::sync::atomic::AtomicBool>| {
            entered_tx.send(()).expect("reservation reached");
            while !cancelled.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(silent_disco_core::audio::PacketizerWorkerError {
                kind: PacketizerWorkerErrorKind::Cancelled,
                message: "test reservation cancelled".to_owned(),
            })
        },
    );
    let stream = FfiHostPlaylistStreamer::open_with_reserver(
        "session-ffi-cancel-poll".to_owned(),
        "stream-ffi-cancel-poll".to_owned(),
        5_000,
        sources,
        Some(reservation),
    )
    .expect("start gated packetizer");
    entered_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("producer reached boundary reservation");

    // Drain the outgoing packets. The producer cannot publish a boundary or
    // successor PCM until the gate is explicitly cancelled.
    let mut drained = 0;
    for _ in 0..64 {
        match stream.next_frame(30).expect("drain current track") {
            Some(FfiPlaylistFrame::End) => panic!("gated producer ended early"),
            Some(_) => drained += 1,
            None => break,
        }
    }
    assert!(drained > 0, "current track must produce frames");

    // Repeated short internal polls must honor the caller's *total* timeout.
    // A zero timeout remains a nonblocking read when no frame is queued.
    let immediate = Instant::now();
    assert!(stream.next_frame(0).expect("nonblocking poll").is_none());
    assert!(immediate.elapsed() < Duration::from_secs(1));
    let timed = Instant::now();
    assert!(stream.next_frame(120).expect("timed poll").is_none());
    assert!(
        timed.elapsed() >= Duration::from_millis(90),
        "the internal 20 ms poll must not shorten the caller's timeout"
    );

    let (started_tx, started_rx) = sync_channel(1);
    let reader = Arc::clone(&stream);
    let waiting_consumer = std::thread::spawn(move || {
        started_tx.send(()).expect("consumer started");
        reader.next_frame(10_000)
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("consumer started");
    // Ensure next_frame actually holds the mutex (rather than racing
    // cancellation at the initial closed check).
    let acquire_deadline = Instant::now() + Duration::from_secs(2);
    let mut locked = false;
    while Instant::now() < acquire_deadline {
        if stream.inner.try_lock().is_err() {
            locked = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if !locked {
        stream.cancellation.store(true, Ordering::Release);
        stream.closed.store(true, Ordering::Release);
        waiting_consumer
            .join()
            .expect("reader thread")
            .expect("reader result");
        panic!("consumer did not wait on the packet channel");
    }

    let stop_started = Instant::now();
    stream
        .cancel()
        .expect("cancel must not wait for long timeout");
    let stop_elapsed = stop_started.elapsed();
    let received = waiting_consumer
        .join()
        .expect("consumer thread")
        .expect("frame read");
    assert!(received.is_none(), "Stop must suppress queued audio");
    assert!(
        stop_elapsed < Duration::from_secs(2),
        "Stop blocked behind a long receive timeout: {stop_elapsed:?}"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

fn wav_pcm16(sample_rate: u32, channels: u16, frames: u32, sample: i16) -> Vec<u8> {
    let data_bytes = frames * u32::from(channels) * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for _ in 0..frames * u32::from(channels) {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}
