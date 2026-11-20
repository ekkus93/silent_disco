//! Queue edits made while a stream plays (device matrix, 2026-11-20).
//!
//! Position updates advance the snapshot revision several times a second
//! during playback, so a host UI's on-screen snapshot revision is almost
//! always stale by the time an edit reaches the actor. On the A54 the desktop's
//! move and remove buttons silently did nothing. A queue edit carries its own
//! exact precondition, the queue revision, which only queue changes advance;
//! that is what the actor checks for it. Other commands keep the snapshot
//! revision check.

use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, PlaybackState, RequestId, StreamId, TransportState, TrustState,
};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle,
    CoreActorRuntime, CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot,
    HostDraftPatch, InviteCodePatch, ListenerSummary, PlatformEffect, PlatformEvent,
    PlatformOperationCompletion, QueueEntry, SnapshotRevision, TransportEvent,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn queue_edits_from_a_snapshot_that_position_updates_outdated_still_apply() {
    let (runtime, handle, receiver) = start_actor();
    let playing = start_playing_with_queue(&handle, &receiver, &["b", "c", "d"]);
    assert_eq!(entry_ids(&playing), ["source-1", "b", "c", "d"]);
    // What the host UI shows when the user presses a button.
    let shown = playing.clone();

    let advanced = advance_position(&handle, &receiver, &playing, 4);
    assert!(advanced.revision.get() > shown.revision.get());
    assert_eq!(advanced.host_queue.revision(), shown.host_queue.revision());

    let moved = command(
        &handle,
        &receiver,
        shown.revision,
        CoreCommand::ReorderHostQueue {
            expected_queue_revision: shown.host_queue.revision(),
            ordered_entry_ids: ids(&["source-1", "d", "b", "c"]),
        },
    )
    .expect("a reorder from an outdated snapshot applies");
    assert_eq!(entry_ids(&moved), ["source-1", "d", "b", "c"]);

    // The UI refreshed to the reorder, then position moved on again.
    let shown = moved.clone();
    advance_position(&handle, &receiver, &moved, 3);
    let removed = command(
        &handle,
        &receiver,
        shown.revision,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: shown.host_queue.revision(),
            entry_id: "b".to_owned(),
        },
    )
    .expect("a removal from an outdated snapshot applies");
    assert_eq!(entry_ids(&removed), ["source-1", "d", "c"]);

    let appended = command(
        &handle,
        &receiver,
        shown.revision,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: removed.host_queue.revision(),
            entries: vec![entry("e")],
        },
    )
    .expect("an append from an outdated snapshot applies");
    assert_eq!(entry_ids(&appended), ["source-1", "d", "c", "e"]);

    runtime.shutdown().expect("shutdown actor");
}

#[test]
fn a_queue_edit_from_an_outdated_queue_is_still_refused() {
    let (runtime, handle, receiver) = start_actor();
    let playing = start_playing_with_queue(&handle, &receiver, &["b", "c"]);
    let removed = command(
        &handle,
        &receiver,
        playing.revision,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: playing.host_queue.revision(),
            entry_id: "c".to_owned(),
        },
    )
    .expect("first removal");

    // A second edit planned against the queue before that removal.
    let error = command(
        &handle,
        &receiver,
        removed.revision,
        CoreCommand::ReorderHostQueue {
            expected_queue_revision: playing.host_queue.revision(),
            ordered_entry_ids: ids(&["source-1", "c", "b"]),
        },
    )
    .expect_err("an edit planned against an older queue is refused");
    assert!(
        error.message.contains("queue revision changed"),
        "{}",
        error.message
    );
    let after = advance_position(&handle, &receiver, &removed, 1);
    assert_eq!(entry_ids(&after), ["source-1", "b"]);

    runtime.shutdown().expect("shutdown actor");
}

#[test]
fn a_track_change_outdates_a_queue_edit_planned_before_it() {
    let (runtime, handle, receiver) = start_actor();
    let playing = start_playing_with_queue(&handle, &receiver, &["b", "c"]);
    handle
        .submit_audio_event(AudioEvent::TrackBoundaryScheduled {
            entry_id: "b".to_owned(),
            first_sample_index: 48_000,
        })
        .expect("schedule boundary");
    let scheduled = next_snapshot(&receiver, playing.revision.get() + 1);
    handle
        .submit_audio_event(AudioEvent::TrackBoundaryApplied {
            entry_id: "b".to_owned(),
            first_sample_index: 48_000,
        })
        .expect("apply boundary");
    let changed = next_snapshot(&receiver, scheduled.revision.get() + 1);
    assert_eq!(entry_ids(&changed), ["b", "c"]);

    // Planned while source-1 was current: removing "b" would now remove the
    // playing track's entry, so the outdated queue revision must stop it.
    let error = command(
        &handle,
        &receiver,
        changed.revision,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: playing.host_queue.revision(),
            entry_id: "b".to_owned(),
        },
    )
    .expect_err("an edit planned before the track change is refused");
    assert!(
        error.message.contains("queue revision changed"),
        "{}",
        error.message
    );

    runtime.shutdown().expect("shutdown actor");
}

#[test]
fn other_commands_keep_the_snapshot_revision_check() {
    let (runtime, handle, receiver) = start_actor();
    let playing = start_playing_with_queue(&handle, &receiver, &["b"]);
    advance_position(&handle, &receiver, &playing, 1);
    let error = command(
        &handle,
        &receiver,
        playing.revision,
        CoreCommand::RejectJoin {
            request_id: RequestId::new("request-1").expect("request id"),
        },
    )
    .expect_err("a join decision from an outdated snapshot is refused");
    assert!(
        error.message.contains("expected snapshot revision"),
        "{}",
        error.message
    );

    runtime.shutdown().expect("shutdown actor");
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn entry_ids(snapshot: &CoreSnapshot) -> Vec<&str> {
    snapshot
        .host_queue
        .entries()
        .iter()
        .map(|entry| entry.entry_id.as_str())
        .collect()
}

fn entry(id: &str) -> QueueEntry {
    QueueEntry::new(
        id,
        AudioSourceDescriptor::new(
            format!("source-{id}"),
            format!("{id}.wav"),
            Some(4096),
            Some(2000),
        )
        .expect("source"),
    )
    .expect("queue entry")
}

fn advance_position(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    from: &CoreSnapshot,
    updates: u64,
) -> CoreSnapshot {
    let stream_id = StreamId::new("stream-edit").expect("stream id");
    let mut latest = from.clone();
    for step in 1..=updates {
        handle
            .submit_audio_event(AudioEvent::PositionAdvanced {
                stream_id: stream_id.clone(),
                position_ms: from.playback_position_ms + step * 250,
            })
            .expect("submit position");
        latest = next_snapshot(receiver, latest.revision.get() + 1);
    }
    latest
}

fn start_playing_with_queue(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    upcoming: &[&str],
) -> CoreSnapshot {
    let selected = command(
        handle,
        receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    )
    .expect("select host");
    let source =
        AudioSourceDescriptor::new("source-1", "a.wav", Some(4096), Some(2000)).expect("source");
    let drafted = command(
        handle,
        receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Edit host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    )
    .expect("draft");
    let queued = command(
        handle,
        receiver,
        drafted.revision,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: drafted.host_queue.revision(),
            entries: upcoming.iter().map(|id| entry(id)).collect(),
        },
    )
    .expect("queue");
    let creating = command(
        handle,
        receiver,
        queued.revision,
        CoreCommand::CreateHostSession,
    )
    .expect("create");
    let start_advertising = next_platform_effect(receiver);
    handle
        .submit_transport_event(TransportEvent::StateChanged(TransportState::Advertising))
        .expect("advertising");
    let advertising = next_snapshot(receiver, creating.revision.get() + 1);
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: start_advertising.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising started");
    let waiting = next_snapshot(receiver, advertising.revision.get() + 1);
    handle
        .submit_transport_event(TransportEvent::ListenerConnected(
            ListenerSummary::new(
                DeviceId::new("listener-1").expect("listener id"),
                "Listener One",
                TrustState::SessionOnly,
                TransportState::Connected,
            )
            .expect("listener"),
        ))
        .expect("listener connected");
    let ready = next_snapshot(receiver, waiting.revision.get() + 1);
    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("playing");
    next_snapshot(receiver, ready.revision.get() + 1)
}

fn start_actor() -> (
    CoreActorRuntime,
    CoreActorHandle,
    Receiver<CoreNotification>,
) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("host-core").expect("host id")),
        move |notification| {
            sender
                .send(notification)
                .expect("test receiver remains connected");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = runtime.handle();
    assert_eq!(next_snapshot(&receiver, 0).revision.get(), 0);
    (runtime, handle, receiver)
}

/// Submits one command and returns the snapshot it produced, or the error
/// the actor refused it with.
fn command(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    revision: SnapshotRevision,
    command: CoreCommand,
) -> Result<CoreSnapshot, CoreError> {
    handle
        .submit_command(CoreCommandRequest::new(revision, command).expect("valid request"))
        .expect("queue command");
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() > revision.get() =>
            {
                return Ok(snapshot);
            }
            Ok(CoreNotification::Error(error)) => return Err(error),
            Ok(_) => {}
            Err(error) => panic!("no outcome for the command: {error}"),
        }
    }
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum_revision: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() >= minimum_revision =>
            {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out waiting for snapshot revision {minimum_revision}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("notification receiver disconnected"),
        }
    }
}

fn next_platform_effect(receiver: &Receiver<CoreNotification>) -> PlatformEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for platform effect: {error}"),
        }
    }
}
