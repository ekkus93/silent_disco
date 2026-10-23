//! Exercise the real actor authority and FFI producer when a queued edit wins
//! before a backpressured worker can reserve the next track.
use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::host_control::{
    FfiBridgeError, FfiCoreHandle, FfiCoreNotification, FfiCoreObserver, FfiPlatformCompletion,
    FfiPlatformEffect,
};
use silent_disco_core::domain::{AppRole, ApprovalMode, HostLifecycle, PlaybackState, StreamId};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorHandle, CoreCommand,
    CoreCommandRequest, CoreSnapshot, HostDraftPatch, InviteCodePatch, QueueEntry,
};
use std::fs;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Observer(mpsc::Sender<FfiCoreNotification>);

impl FfiCoreObserver for Observer {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.0.send(notification).expect("observer receiver");
        Ok(())
    }
}

fn snapshot_when(
    actor: &CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = actor.current_snapshot().expect("actor snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "actor did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn submit(actor: &CoreActorHandle, command: CoreCommand) {
    let revision = actor.current_snapshot().expect("actor revision").revision;
    actor
        .submit_command(CoreCommandRequest::new(revision, command).expect("command"))
        .expect("submit command");
}

fn wav(frames: u32, sample: i16) -> Vec<u8> {
    let bytes = frames * 4;
    let mut data = Vec::with_capacity(44 + bytes as usize);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16_u32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.extend_from_slice(&48_000_u32.to_le_bytes());
    data.extend_from_slice(&192_000_u32.to_le_bytes());
    data.extend_from_slice(&4_u16.to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for _ in 0..frames * 2 {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real actor/FFI backpressure and queue-edit interleaving"
)]
fn actor_edit_before_ffi_reservation_retargets_without_stale_successor_pcm() {
    let (tx, rx) = mpsc::channel();
    let ffi = FfiCoreHandle::open("ffi-edit-first-actor".into(), Arc::new(Observer(tx)))
        .expect("open actor");
    let actor = ffi.actor_handle().expect("live actor");
    let first = AudioSourceDescriptor::new("first-source", "First.wav", Some(38_400), None)
        .expect("first source");
    let obsolete = AudioSourceDescriptor::new("obsolete-source", "Obsolete.wav", Some(1_920), None)
        .expect("obsolete source");
    let replacement =
        AudioSourceDescriptor::new("replacement-source", "Replacement.wav", Some(1_920), None)
            .expect("replacement source");
    let tail = AudioSourceDescriptor::new("tail-source", "Tail.wav", Some(1_920), None)
        .expect("tail source");

    submit(
        &actor,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    snapshot_when(&actor, |s| s.selected_role == Some(AppRole::Host));
    submit(
        &actor,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Edit-first proof".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(first),
            remember_approved_devices: Some(false),
        }),
    );
    snapshot_when(&actor, |s| s.host_draft.audio_source.is_some());
    submit(&actor, CoreCommand::CreateHostSession);
    let operation_id = loop {
        if let FfiCoreNotification::PlatformEffect {
            effect: FfiPlatformEffect::StartAdvertising { operation_id, .. },
        } = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("advertise effect")
        {
            break operation_id;
        }
    };
    ffi.platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("advertising acknowledged");
    let ready = snapshot_when(&actor, |s| {
        matches!(
            s.host_lifecycle,
            HostLifecycle::Ready | HostLifecycle::WaitingForListeners
        )
    });
    let current_id = ready
        .host_queue
        .current()
        .expect("current")
        .entry_id
        .clone();
    submit(
        &actor,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: ready.host_queue.revision(),
            entries: vec![
                QueueEntry::new("obsolete-entry", obsolete).expect("obsolete entry"),
                QueueEntry::new("replacement-entry", replacement).expect("replacement entry"),
            ],
        },
    );
    snapshot_when(&actor, |s| s.host_queue.entries().len() == 3);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
        .expect("buffer");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Buffering);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("play");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Playing);
    let stream_id = StreamId::new("ffi-edit-first-stream").expect("stream");
    actor
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: stream_id.clone(),
            position_ms: 0,
        })
        .expect("bind stream");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Playing);

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-edit-first-{unique}"));
    fs::create_dir_all(&root).expect("fixtures");
    let paths = [
        root.join("first.wav"),
        root.join("obsolete.wav"),
        root.join("replacement.wav"),
        root.join("tail.wav"),
    ];
    // Forty packets of current audio exceed the producer's 32-frame output
    // queue. Without a consumer, the worker cannot reach its first reservation.
    for (path, frames, sample) in [
        (&paths[0], 9_600, 3_000),
        (&paths[1], 480, 6_000),
        (&paths[2], 480, 12_000),
        (&paths[3], 480, 15_000),
    ] {
        fs::write(path, wav(frames, sample)).expect("PCM fixture");
    }
    let source = |id: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: id.to_owned(),
        display_name: id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };
    let stream = FfiHostPlaylistStreamer::open_authoritative(
        "ffi-edit-first-session".into(),
        stream_id.to_string(),
        5_000,
        vec![
            source(&current_id, &paths[0]),
            source("obsolete-entry", &paths[1]),
            source("replacement-entry", &paths[2]),
        ],
        Arc::clone(&ffi),
    )
    .expect("open authoritative producer");

    let before = actor.current_snapshot().expect("snapshot before edit");
    assert!(
        before.host_queue_next_boundary.is_none(),
        "the backpressured producer must not reserve before the edit"
    );
    submit(
        &actor,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: before.host_queue.revision(),
            entry_id: "obsolete-entry".into(),
        },
    );
    snapshot_when(&actor, |s| {
        s.host_queue.entries().len() == 2
            && s.host_queue.upcoming()[0].entry_id == "replacement-entry"
    });
    stream
        .replace_upcoming(vec![source("replacement-entry", &paths[2])])
        .expect("retarget producer from authoritative queue");
    // A second accepted edit while the 32-frame producer queue is still
    // backpressured appends behind the replacement. The latest authoritative
    // snapshot must win, without resurrecting the removed source.
    let after_removal = actor.current_snapshot().expect("queue after removal");
    submit(
        &actor,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: after_removal.host_queue.revision(),
            entries: vec![QueueEntry::new("tail-entry", tail).expect("tail entry")],
        },
    );
    let appended = snapshot_when(&actor, |s| {
        s.host_queue.entries().len() == 3 && s.host_queue.upcoming()[1].entry_id == "tail-entry"
    });
    assert!(appended.host_queue_next_boundary.is_none());
    stream
        .replace_upcoming(vec![
            source("replacement-entry", &paths[2]),
            source("tail-entry", &paths[3]),
        ])
        .expect("retarget producer from latest authoritative queue");

    let mut next_sample = 0_u64;
    let mut last_sequence = None;
    let mut boundaries = Vec::new();
    loop {
        match stream.next_frame(5_000).expect("stream frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence,
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            }) => {
                assert_eq!(first_sample_index, next_sample, "sample gap or overlap");
                if let Some(previous) = last_sequence {
                    assert_eq!(sequence, previous + 1, "packet sequence gap");
                }
                last_sequence = Some(sequence);
                let sample = i16::from_le_bytes([payload[0], payload[1]]);
                assert_eq!(
                    sample,
                    if next_sample < 9_600 {
                        3_000
                    } else if next_sample < 10_080 {
                        12_000
                    } else {
                        15_000
                    },
                    "removed track PCM must not escape"
                );
                next_sample += u64::from(samples_per_packet);
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                assert_eq!(first_sample_index, next_sample);
                if entry_id == "tail-entry" {
                    assert_eq!(first_sample_index, 10_080);
                    // The actor keeps ordered reservations even when the host
                    // scheduler has not yet applied the earlier boundary.
                    let snapshot = actor.current_snapshot().expect("ordered reservations");
                    assert_eq!(snapshot.host_queue.upcoming()[1].entry_id, "tail-entry");
                    boundaries.push((entry_id, first_sample_index));
                    continue;
                }
                assert_eq!(entry_id, "replacement-entry");
                let snapshot = snapshot_when(&actor, |s| s.host_queue_next_boundary.is_some());
                assert_eq!(
                    snapshot.host_queue_next_boundary.as_ref().unwrap().entry_id,
                    "replacement-entry"
                );
                // Reservation has already won the actor's serialized race.
                // A later host edit must not remove PCM that is committed to
                // the producer's bounded output queue.
                let queue_revision = snapshot.host_queue.revision();
                submit(
                    &actor,
                    CoreCommand::RemoveHostQueueEntry {
                        expected_queue_revision: queue_revision,
                        entry_id: "replacement-entry".into(),
                    },
                );
                let rejected = loop {
                    if let FfiCoreNotification::Error { error } = rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("reserved successor removal must be rejected")
                    {
                        break error;
                    }
                };
                assert!(!rejected.message.is_empty());
                let protected = actor.current_snapshot().expect("protected queue");
                assert_eq!(protected.host_queue.revision(), queue_revision);
                assert_eq!(
                    protected.host_queue.upcoming()[0].entry_id,
                    "replacement-entry"
                );
                assert_eq!(
                    protected
                        .host_queue_next_boundary
                        .as_ref()
                        .unwrap()
                        .entry_id,
                    "replacement-entry"
                );
                // Pausing with a reserved successor must retain the actor's
                // committed boundary. Resume must not unlock the successor
                // or change which PCM the already-buffered producer emits.
                actor
                    .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
                    .expect("pause at reserved boundary");
                let paused = snapshot_when(&actor, |s| s.playback_state == PlaybackState::Paused);
                assert_eq!(
                    paused.host_queue_next_boundary.as_ref().unwrap().entry_id,
                    "replacement-entry"
                );
                actor
                    .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
                    .expect("resume at reserved boundary");
                let resumed = snapshot_when(&actor, |s| s.playback_state == PlaybackState::Playing);
                assert_eq!(resumed.host_queue.revision(), queue_revision);
                assert_eq!(
                    resumed.host_queue_next_boundary.as_ref().unwrap().entry_id,
                    "replacement-entry"
                );
                boundaries.push((entry_id, first_sample_index));
            }
            Some(FfiPlaylistFrame::End) => break,
            None => panic!("producer stalled after edit-first retarget"),
        }
    }
    assert_eq!(next_sample, 10_560);
    assert_eq!(last_sequence, Some(43));
    assert_eq!(
        boundaries,
        [
            ("replacement-entry".to_owned(), 9_600),
            ("tail-entry".to_owned(), 10_080),
        ]
    );
    // Stop invalidates any reservation made for the old stream. A fresh Play
    // can reuse the same queue entry IDs, but a delayed producer belonging to
    // the retired stream must fail closed before emitting its successor.
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
        .expect("stop first stream");
    let stopped = snapshot_when(&actor, |s| s.playback_state == PlaybackState::Stopped);
    assert!(stopped.host_queue_next_boundary.is_none());
    assert_eq!(
        stopped.host_queue.upcoming()[0].entry_id,
        "replacement-entry"
    );
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
        .expect("buffer replay");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Buffering);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("play new stream");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Playing);
    let replay_stream = StreamId::new("ffi-edit-first-new-stream").expect("replay stream");
    actor
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: replay_stream,
            position_ms: 7,
        })
        .expect("bind new stream");
    // A snapshot request follows the stream-binding message in actor order.
    let replay = actor.current_snapshot().expect("replay snapshot");
    assert_eq!(replay.playback_state, PlaybackState::Playing);
    assert!(replay.host_queue_next_boundary.is_none());

    let retired = FfiHostPlaylistStreamer::open_authoritative(
        "ffi-edit-first-session".into(),
        stream_id.to_string(),
        5_000,
        vec![
            source(&current_id, &paths[1]),
            source("replacement-entry", &paths[2]),
        ],
        Arc::clone(&ffi),
    )
    .expect("old stream opens, but cannot reserve a boundary");
    let mut retired_frames = 0;
    let failure = loop {
        match retired.next_frame(5_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                first_sample_index,
                payload,
                ..
            })) => {
                assert!(first_sample_index < 480, "retired successor audio escaped");
                assert_eq!(i16::from_le_bytes([payload[0], payload[1]]), 6_000);
                retired_frames += 1;
            }
            Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                panic!("retired producer published a boundary into replay")
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                panic!("retired reservation cannot report a clean queue end")
            }
            Ok(None) => panic!("retired producer stalled instead of failing closed"),
            Err(error) => break error,
        }
    };
    assert_eq!(
        retired_frames, 2,
        "old current PCM should precede rejection"
    );
    assert!(
        failure.to_string().contains("reservation"),
        "stale stream must be rejected by authoritative reservation: {failure}"
    );
    let untouched = actor
        .current_snapshot()
        .expect("new stream remains unmodified");
    assert!(untouched.host_queue_next_boundary.is_none());
    assert_eq!(
        untouched.host_queue.upcoming()[0].entry_id,
        "replacement-entry"
    );
    ffi.shutdown().expect("shutdown actor");
    fs::remove_dir_all(root).expect("remove fixtures");
}
