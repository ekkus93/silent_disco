//! The actor-authorized queue edit must drive both real TCP/UDP listeners,
//! the production scheduler, and native callback PCM, not only the FFI queue.
use crate::audio_abi::registry_test_guard;
use crate::host_control::{
    FfiBridgeError, FfiCoreHandle, FfiCoreNotification, FfiCoreObserver, FfiPlatformCompletion,
    FfiPlatformEffect,
};
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, playback_generation, source};
use crate::socket_pause_resume_pcm_tests::authorized_late_joiner;
use silent_disco_core::domain::{
    AppRole, ApprovalMode, HostLifecycle, PlaybackState, SessionId, StreamId,
};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorHandle, CoreCommand,
    CoreCommandRequest, CoreSnapshot, HostDraftPatch, InviteCodePatch, QueueEntry,
};
use std::fs;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Observer(mpsc::Sender<FfiCoreNotification>);
impl FfiCoreObserver for Observer {
    fn on_notification(&self, event: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.0.send(event).expect("observer receiver");
        Ok(())
    }
}

fn snapshot_when(actor: &CoreActorHandle, matches: impl Fn(&CoreSnapshot) -> bool) -> CoreSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = actor.current_snapshot().expect("actor snapshot");
        if matches(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "actor state deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn submit(actor: &CoreActorHandle, command: CoreCommand) {
    let revision = actor.current_snapshot().expect("revision").revision;
    actor
        .submit_command(CoreCommandRequest::new(revision, command).expect("command"))
        .expect("submit");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "actor mutation, reserved producer, two socket listeners and callback PCM"
)]
fn actor_approved_remove_append_reorder_reaches_both_native_outputs_without_stale_track() {
    let _guard = registry_test_guard();
    let (tx, rx) = mpsc::channel();
    let ffi = FfiCoreHandle::open("actor-socket-edit".to_owned(), Arc::new(Observer(tx)))
        .expect("open actor authority");
    let actor = ffi.actor_handle().expect("actor");
    let descriptor = |id: &str, bytes| {
        AudioSourceDescriptor::new(id, id, Some(bytes), None).expect("source descriptor")
    };
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
            session_name: Some("Socket actor edit proof".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor("current-source", 38_400)),
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
            .expect("advertising effect")
        {
            break operation_id;
        }
    };
    ffi.platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("advertising confirmed");
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
                QueueEntry::new("stale-entry", descriptor("stale-source", 1_920))
                    .expect("stale queue entry"),
                QueueEntry::new("replacement-entry", descriptor("replacement-source", 1_920))
                    .expect("replacement queue entry"),
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
    let stream = StreamId::new("actor-socket-edit-stream").expect("stream");
    actor
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: stream.clone(),
            position_ms: 0,
        })
        .expect("associate stream");
    snapshot_when(&actor, |s| s.playback_state == PlaybackState::Playing);

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-actor-socket-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let current = root.join("current.wav");
    let stale = root.join("stale.wav");
    let replacement = root.join("replacement.wav");
    let tail = root.join("tail.wav");
    fs::write(&current, source_wav_format(3_000, 48_000, 2, 9_600)).expect("40-packet current");
    fs::write(&stale, source_wav_format(11_000, 48_000, 2, 480)).expect("stale source");
    fs::write(&replacement, source_wav_format(7_000, 44_100, 1, 441)).expect("mixed-rate source");
    fs::write(&tail, source_wav_format(9_000, 48_000, 2, 480)).expect("tail source");

    let session = SessionId::new("end-to-end-session").expect("session");
    let (mut host, mut primary) = authorized_pcm_socket_pair();
    let mut secondary = authorized_late_joiner(host.as_mut(), "actor-edited-second-listener");
    // With 40 packets of current PCM and a 32-frame producer queue, no
    // reservation can be reached before the following typed actor edit.
    let producer = FfiHostPlaylistStreamer::open_authoritative(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![
            source(&current_id, &current),
            source("stale-entry", &stale),
            source("replacement-entry", &replacement),
        ],
        Arc::clone(&ffi),
    )
    .expect("production reserved streamer");
    let before = actor.current_snapshot().expect("pre-edit state");
    assert!(before.host_queue_next_boundary.is_none());
    submit(
        &actor,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: before.host_queue.revision(),
            entry_id: "stale-entry".to_owned(),
        },
    );
    let removed = snapshot_when(&actor, |s| {
        s.host_queue.entries().len() == 2
            && s.host_queue.upcoming()[0].entry_id == "replacement-entry"
    });
    producer
        .replace_upcoming(vec![source("replacement-entry", &replacement)])
        .expect("apply approved removal to producer");
    submit(
        &actor,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: removed.host_queue.revision(),
            entries: vec![
                QueueEntry::new("tail-entry", descriptor("tail-source", 1_920))
                    .expect("tail queue entry"),
            ],
        },
    );
    let appended = snapshot_when(&actor, |s| {
        s.host_queue.entries().len() == 3 && s.host_queue.upcoming()[1].entry_id == "tail-entry"
    });
    assert!(appended.host_queue_next_boundary.is_none());
    // A second typed edit reorders both upcoming sources while the producer
    // is still backpressured by the 40-packet current track. The two native
    // listeners must render the actor's final order, not the old prefetch.
    submit(
        &actor,
        CoreCommand::ReorderHostQueue {
            expected_queue_revision: appended.host_queue.revision(),
            ordered_entry_ids: vec![
                current_id.clone(),
                "tail-entry".to_owned(),
                "replacement-entry".to_owned(),
            ],
        },
    );
    let reordered = snapshot_when(&actor, |s| {
        s.host_queue.upcoming()[0].entry_id == "tail-entry"
            && s.host_queue.upcoming()[1].entry_id == "replacement-entry"
    });
    assert!(reordered.host_queue_next_boundary.is_none());
    producer
        .replace_upcoming(vec![
            source("tail-entry", &tail),
            source("replacement-entry", &replacement),
        ])
        .expect("apply approved reorder to producer");

    // The same two native playback outputs span edited boundaries and a
    // Pause/Resume; the deleted source must appear in neither PCM stream.
    let rendered = playback_generation(
        host.as_mut(),
        primary.as_mut(),
        Some(secondary.as_mut()),
        &producer,
        &session,
        &stream,
        1_000,
        &[241, 337, 382, 4_800, 4_800],
        true,
        false,
    );
    assert!(rendered.failure.is_none());
    assert_eq!(rendered.packets, 44);
    assert_eq!(rendered.boundaries, [9_600, 10_080]);
    let mut levels = vec![3_000_i16; 20];
    levels.extend([9_000, 7_000]);
    assert_eq!(
        rendered.pcm,
        expected_pcm(&levels, rendered.startup_ramp_frames),
        "TCP/UDP-to-callback PCM must reflect actor queue edits exactly"
    );

    // The producer has already reserved the reordered successor inside
    // Rust's serialized actor. Later removal or reorder cannot replace it.
    let reserved = snapshot_when(&actor, |s| s.host_queue_next_boundary.is_some());
    assert_eq!(
        reserved.host_queue_next_boundary.as_ref().unwrap().entry_id,
        "tail-entry"
    );
    assert_eq!(
        reserved.host_queue.upcoming()[1].entry_id,
        "replacement-entry"
    );
    submit(
        &actor,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: reserved.host_queue.revision(),
            entry_id: "tail-entry".to_owned(),
        },
    );
    let rejected = loop {
        if let FfiCoreNotification::Error { error } = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reservation rejection")
        {
            break error;
        }
    };
    assert!(!rejected.message.is_empty());
    let protected = actor.current_snapshot().expect("reservation preserved");
    assert_eq!(
        protected.host_queue.revision(),
        reserved.host_queue.revision()
    );
    assert_eq!(protected.host_queue.upcoming()[0].entry_id, "tail-entry");
    submit(
        &actor,
        CoreCommand::ReorderHostQueue {
            expected_queue_revision: protected.host_queue.revision(),
            ordered_entry_ids: vec![
                current_id,
                "replacement-entry".to_owned(),
                "tail-entry".to_owned(),
            ],
        },
    );
    let reorder_rejected = loop {
        if let FfiCoreNotification::Error { error } = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reserved reorder rejection")
        {
            break error;
        }
    };
    assert!(!reorder_rejected.message.is_empty());
    let still_reserved = actor.current_snapshot().expect("reserved order retained");
    assert_eq!(
        still_reserved.host_queue.revision(),
        protected.host_queue.revision()
    );
    assert_eq!(
        still_reserved.host_queue.upcoming()[0].entry_id,
        "tail-entry"
    );

    producer.cancel().expect("release streamer");
    secondary.shutdown().expect("secondary shutdown");
    primary.shutdown().expect("primary shutdown");
    host.shutdown().expect("host shutdown");
    ffi.shutdown().expect("actor shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
