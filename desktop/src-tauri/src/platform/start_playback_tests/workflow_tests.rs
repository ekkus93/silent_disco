//! End-to-end desktop host workflow coverage through a real profile
//! (`workflow_harness`): transactional multi-file staging, revision-aware
//! queue commands, live additions, multi-track transitions, a late source
//! failure in the middle of a queue, and session/profile cleanup.

use super::fixtures::{corrupt_wav_bytes, long_pcm_wav, pcm_wav, truncated_body_full_header_wav};
use super::harness::{TEST_TIMEOUT, wait_for_audio, wait_for_stream_start};
use super::workflow_harness::HostWorkflow;
use crate::host_workflow_ops;
use crate::runtime_dto::{
    AppendHostQueueRequest, AudioSourceSummaryDto, HostQueueEntryCommandRequest, HostQueueEntryDto,
    ReorderHostQueueRequest, RevisionCommandRequest,
};
use silent_disco_core::domain::{HostLifecycle, PlaybackState};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};
use silent_disco_core::transport::{
    ListenerTransportNode, TransportChannel, TransportErrorKind, TransportEvent,
};
use std::net::{TcpListener, UdpSocket};
use std::time::{Duration, Instant};

/// 100 ms of 44.1 kHz source resampled onto the 48 kHz network timeline.
const SHORT_TRACK_SAMPLES: u64 = 4_800;
/// 3 s of 44.1 kHz source on the 48 kHz network timeline.
const LONG_TRACK_SAMPLES: u64 = 144_000;

macro_rules! open_or_skip {
    () => {
        match HostWorkflow::open() {
            Some(workflow) => workflow,
            None => {
                eprintln!("no private LAN interface on this host; skipping workflow test");
                return;
            }
        }
    };
}

#[test]
fn a_failed_item_in_a_multi_file_selection_leaves_queue_registry_and_session_untouched() {
    let workflow = open_or_skip!();
    let first = workflow.pick(&[("first.wav", pcm_wav())]);
    workflow.add_songs(&first).expect("stage the first song");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 1);
    let before = workflow.snapshot();

    let selection = workflow.pick(&[
        ("second.wav", pcm_wav()),
        ("broken.wav", corrupt_wav_bytes()),
        ("third.wav", pcm_wav()),
    ]);
    let error = workflow
        .add_songs(&selection)
        .expect_err("a selection containing an undecodable file must be rejected");
    assert!(
        error.message.contains("broken.wav"),
        "the rejection names the failing file: {}",
        error.message
    );

    let after = workflow.snapshot();
    assert_eq!(after.host_queue, before.host_queue, "no entry was appended");
    for source in &selection {
        assert!(
            workflow
                .registry
                .resolve(&source.descriptor().source_id)
                .expect("registry")
                .is_none(),
            "{} must not be registered after a rejected selection",
            source.descriptor().display_name
        );
    }
    assert_eq!(after.host_lifecycle, HostLifecycle::Idle);
    assert!(
        workflow
            .network
            .active_host_session()
            .expect("network")
            .is_none()
    );

    // The operator can retry with only the valid files.
    let retry = [selection[0].clone(), selection[2].clone()];
    workflow.add_songs(&retry).expect("retry the valid files");
    let retried = workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let names: Vec<_> = retried
        .host_queue
        .entries()
        .iter()
        .map(|entry| entry.source.display_name.as_str())
        .collect();
    assert_eq!(names, ["first.wav", "second.wav", "third.wav"]);
    workflow.close();
}

#[test]
fn queue_commands_apply_current_revisions_and_reject_stale_ones_visibly() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("a.wav", pcm_wav()),
        ("b.wav", pcm_wav()),
        ("c.wav", pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("add three songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let ids = workflow.queue_ids();
    let (stale_revision, stale_queue_revision) = workflow.revisions();

    let reordered = vec![ids[0].clone(), ids[2].clone(), ids[1].clone()];
    host_workflow_ops::reorder_host_queue(
        &workflow.state,
        &workflow.registry,
        ReorderHostQueueRequest {
            expected_revision: stale_revision.clone(),
            expected_queue_revision: stale_queue_revision.clone(),
            ordered_entry_ids: reordered.clone(),
        },
    )
    .expect("reorder with current revisions");
    workflow.wait(|snapshot| {
        snapshot
            .host_queue
            .entries()
            .iter()
            .map(|entry| &entry.entry_id)
            .eq(reordered.iter())
    });

    // Both revisions are now stale: the actor rejects the removal and the
    // frontend channel receives the rejection; the queue is unchanged.
    host_workflow_ops::remove_host_queue_entry(
        &workflow.state,
        &workflow.registry,
        HostQueueEntryCommandRequest {
            expected_revision: stale_revision,
            expected_queue_revision: stale_queue_revision.clone(),
            entry_id: ids[1].clone(),
        },
    )
    .expect("the command is accepted for authoritative validation");
    workflow
        .notifications
        .wait_for_error("expected snapshot revision");
    assert_eq!(workflow.queue_ids(), reordered);

    // A fresh snapshot revision with a stale queue revision is still rejected.
    let (fresh_revision, _) = workflow.revisions();
    host_workflow_ops::remove_host_queue_entry(
        &workflow.state,
        &workflow.registry,
        HostQueueEntryCommandRequest {
            expected_revision: fresh_revision,
            expected_queue_revision: stale_queue_revision,
            entry_id: ids[1].clone(),
        },
    )
    .expect("the command is accepted for authoritative validation");
    workflow.notifications.wait_for_error("queue revision");
    assert_eq!(workflow.queue_ids(), reordered);

    // Appending a source this profile never staged fails before the actor.
    let (current_revision, current_queue_revision) = workflow.revisions();
    let unstaged = AppendHostQueueRequest {
        expected_revision: current_revision,
        expected_queue_revision: current_queue_revision,
        entries: vec![HostQueueEntryDto {
            entry_id: "unstaged-entry".to_owned(),
            source: AudioSourceSummaryDto {
                source_id: "never-staged-source".to_owned(),
                display_name: "ghost.wav".to_owned(),
                byte_length: Some("128".to_owned()),
                duration_ms: None,
            },
        }],
    };
    let error = host_workflow_ops::append_host_queue(&workflow.state, &workflow.registry, unstaged)
        .expect_err("an unstaged source cannot be queued");
    assert_eq!(error.code, "desktop.host_queue.invalid_request");
    assert_eq!(workflow.queue_ids(), reordered);

    // Current revisions remove the entry.
    let (current_revision, current_queue_revision) = workflow.revisions();
    host_workflow_ops::remove_host_queue_entry(
        &workflow.state,
        &workflow.registry,
        HostQueueEntryCommandRequest {
            expected_revision: current_revision,
            expected_queue_revision: current_queue_revision,
            entry_id: ids[1].clone(),
        },
    )
    .expect("remove with current revisions");
    let removed = workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 2);
    assert!(
        removed
            .host_queue
            .entries()
            .iter()
            .all(|entry| entry.entry_id != ids[1])
    );
    workflow.close();
}

/// Regression: "Add songs" during live playback used the snapshot revision
/// read before the file dialog opened (position/sync updates make it stale
/// within milliseconds) and never retargeted the active packetizer, so a song
/// added to the last playing track was neither accepted nor prebuffered.
#[test]
fn songs_added_during_live_playback_are_queued_and_play_at_the_exact_boundary() {
    let workflow = open_or_skip!();
    let current = workflow.pick(&[("current.wav", long_pcm_wav())]);
    workflow
        .add_songs(&current)
        .expect("queue the current song");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 1);
    let endpoint = workflow.create_session();
    let mut listener = workflow.join_listener(endpoint);
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start playback");
    let _ = wait_for_stream_start(&mut *listener);
    let _ = wait_for_audio(&mut *listener);
    let playing_revision = workflow.snapshot().revision;
    workflow.wait(|snapshot| snapshot.revision > playing_revision);

    let added = workflow.pick(&[("added-live.wav", pcm_wav())]);
    workflow
        .add_songs(&added)
        .expect("add a song while the snapshot revision advances");
    let queued = workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 2);
    let added_entry = queued.host_queue.entries()[1].entry_id.clone();

    let stream = receive_until_stop(&mut *listener);
    assert_eq!(
        stream.boundaries,
        vec![(added_entry, LONG_TRACK_SAMPLES)],
        "the live addition starts exactly where the current song ends"
    );
    stream.assert_contiguous();
    assert!(stream.stop_time_ms >= stream.last_audio_end_ms);
    listener.shutdown().expect("listener shutdown");
    workflow.close();
}

#[test]
fn playback_crosses_every_queued_boundary_and_ends_with_the_session_still_open() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("one.wav", pcm_wav()),
        ("two.wav", pcm_wav()),
        ("three.wav", pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("queue three songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let ids = workflow.queue_ids();
    let endpoint = workflow.create_session();
    let mut listener = workflow.join_listener(endpoint);

    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start the queue");
    let stream = receive_until_stop(&mut *listener);
    assert_eq!(stream.start_entry.as_deref(), Some(ids[0].as_str()));
    assert_eq!(stream.audio.first().map(|packet| packet.0), Some(0));
    assert_eq!(
        stream.boundaries,
        vec![
            (ids[1].clone(), SHORT_TRACK_SAMPLES),
            (ids[2].clone(), 2 * SHORT_TRACK_SAMPLES),
        ],
    );
    stream.assert_contiguous();
    assert_eq!(stream.total_samples(), 3 * SHORT_TRACK_SAMPLES);
    assert!(stream.stop_time_ms >= stream.last_audio_end_ms);

    let ended = workflow.wait(|snapshot| {
        snapshot.playback_state == PlaybackState::Stopped && snapshot.stream_ended_naturally
    });
    assert!(
        ended.host_queue.entries().is_empty(),
        "queue end clears the played queue"
    );
    assert!(ended.last_error.is_none());
    assert!(
        workflow
            .network
            .active_host_session()
            .expect("network")
            .is_some(),
        "the host session stays open for newly added songs"
    );
    assert!(!workflow.network.playback_is_active().expect("playback"));
    listener.shutdown().expect("listener shutdown");
    workflow.close();
}

#[test]
fn a_late_source_failure_mid_queue_retains_the_failed_item_after_its_valid_audio() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("opener.wav", pcm_wav()),
        ("damaged.wav", truncated_body_full_header_wav()),
        ("closer.wav", pcm_wav()),
    ]);
    // The damaged file decodes until its truncation point, so preparation
    // validation rejects it; stage the valid neighbours, then register the
    // damaged copy directly to model a source that was readable when queued
    // and fails later (for example, on removable storage).
    let error = workflow
        .add_songs(&picked)
        .expect_err("queue-time validation catches the truncated body");
    assert!(error.message.contains("damaged.wav"), "{}", error.message);
    assert!(workflow.snapshot().host_queue.entries().is_empty());
    workflow
        .add_songs(&[picked[0].clone()])
        .expect("queue opener");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 1);
    append_without_validation(&workflow, &picked[1]);
    workflow
        .add_songs(&[picked[2].clone()])
        .expect("queue closer");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let ids = workflow.queue_ids();

    let endpoint = workflow.create_session();
    let mut listener = workflow.join_listener(endpoint);
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start queue");
    let stream = receive_until_stop(&mut *listener);
    assert_eq!(stream.start_entry.as_deref(), Some(ids[0].as_str()));
    assert_eq!(
        stream.boundaries.first().map(|b| b.0.as_str()),
        Some(ids[1].as_str())
    );
    assert!(
        stream.boundaries.iter().all(|(entry, _)| entry != &ids[2]),
        "playback never silently skips past the failed song"
    );
    stream.assert_contiguous();
    assert!(
        stream.stop_time_ms >= stream.last_audio_end_ms,
        "Stop ({}) must not cut off valid audio ending at {}",
        stream.stop_time_ms,
        stream.last_audio_end_ms
    );

    let failed = workflow.wait(|snapshot| snapshot.playback_state == PlaybackState::Error);
    assert!(!failed.stream_ended_naturally);
    let current = failed
        .host_queue
        .current()
        .expect("failed item stays current");
    assert_eq!(current.entry_id, ids[1]);
    assert!(
        failed
            .host_queue
            .entries()
            .iter()
            .any(|entry| entry.entry_id == ids[2]),
        "songs after the failure stay queued"
    );
    assert!(
        failed
            .last_error
            .as_ref()
            .is_some_and(|error| error.message.contains("damaged.wav")),
        "the visible error names the failed song: {:?}",
        failed.last_error
    );
    assert!(
        workflow.state.stop_host_playback().is_err(),
        "the pump failure is reported, not swallowed as a clean stop"
    );
    assert!(
        workflow
            .network
            .active_host_session()
            .expect("network")
            .is_some()
    );

    // The failed current song can be removed so the queue can continue.
    let (current_revision, current_queue_revision) = workflow.revisions();
    host_workflow_ops::remove_host_queue_entry(
        &workflow.state,
        &workflow.registry,
        HostQueueEntryCommandRequest {
            expected_revision: current_revision,
            expected_queue_revision: current_queue_revision,
            entry_id: ids[1].clone(),
        },
    )
    .expect("remove failed song");
    let recovered = workflow.wait(|snapshot| {
        snapshot
            .host_queue
            .current()
            .is_some_and(|entry| entry.entry_id == ids[2])
    });
    assert_eq!(recovered.host_queue.entries().len(), 1);
    listener.shutdown().expect("listener shutdown");
    workflow.close();
}

#[test]
fn ending_the_session_mid_playback_releases_ports_workers_and_staged_sources() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[("long.wav", long_pcm_wav()), ("next.wav", pcm_wav())]);
    workflow.add_songs(&picked).expect("queue songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 2);
    let endpoint = workflow.create_session();
    let mut listener = workflow.join_listener(endpoint);
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start playback");
    let _ = wait_for_stream_start(&mut *listener);
    let _ = wait_for_audio(&mut *listener);

    let (current_revision, _) = workflow.revisions();
    host_workflow_ops::end_host_session(
        &workflow.state,
        &workflow.registry,
        &RevisionCommandRequest {
            expected_revision: current_revision,
        },
    )
    .expect("end session");
    let ended = workflow.wait(|snapshot| snapshot.host_lifecycle == HostLifecycle::Idle);
    assert!(
        ended.host_queue.entries().is_empty(),
        "ending clears the queue"
    );
    assert!(
        workflow
            .network
            .active_host_session()
            .expect("network")
            .is_none()
    );
    assert!(!workflow.network.playback_is_active().expect("playback"));
    for source in &picked {
        assert!(
            workflow
                .registry
                .resolve(&source.descriptor().source_id)
                .expect("registry")
                .is_none(),
            "{} is released with the session",
            source.descriptor().display_name
        );
    }
    let host_address = endpoint.address;
    TcpListener::bind((host_address, endpoint.control_port)).expect("control port released");
    UdpSocket::bind((host_address, endpoint.sync_port)).expect("sync port released");
    UdpSocket::bind((host_address, endpoint.audio_port)).expect("audio port released");
    assert!(
        workflow
            .state
            .start_host_playback(&workflow.registry)
            .is_err(),
        "no stale queue or binding can restart playback after End"
    );

    // A new session in the same profile binds again and accepts new songs.
    let fresh = workflow.pick(&[("after-end.wav", pcm_wav())]);
    workflow.add_songs(&fresh).expect("queue after End");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 1);
    let _ = workflow.create_session();
    listener.shutdown().expect("listener shutdown");
    let network = std::sync::Arc::clone(&workflow.network);
    workflow.close();
    assert!(
        network.active_host_session().expect("network").is_none(),
        "profile close stops the active host transport"
    );
}

/// Registers an inspected source after staging it without decode
/// validation, then appends it through the production append command.
fn append_without_validation(
    workflow: &HostWorkflow,
    source: &crate::platform::file_picker::InspectedAudioSource,
) {
    let control = crate::platform::source_staging_control::SourceStagingControl::new();
    let operation = control.begin().expect("staging operation");
    let directory = workflow.state.source_staging_directory().expect("sources");
    let staged = crate::platform::source_staging::stage_audio_source(
        source,
        &directory,
        &operation,
        &super::workflow_harness::DiscardProgress,
    )
    .expect("stage damaged source");
    drop(operation);
    let descriptor = staged.source.descriptor().clone();
    workflow.registry.register(staged.source).expect("register");
    let (current_revision, current_queue_revision) = workflow.revisions();
    let request = AppendHostQueueRequest {
        expected_revision: current_revision,
        expected_queue_revision: current_queue_revision,
        entries: vec![HostQueueEntryDto {
            entry_id: "damaged-entry".to_owned(),
            source: AudioSourceSummaryDto {
                source_id: descriptor.source_id,
                display_name: descriptor.display_name,
                byte_length: descriptor.byte_length.map(|length| length.to_string()),
                duration_ms: None,
            },
        }],
    };
    let before = workflow.snapshot().host_queue.entries().len();
    host_workflow_ops::append_host_queue(&workflow.state, &workflow.registry, request)
        .expect("append damaged source");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == before + 1);
}

/// What one listener heard of a stream, from its first audio to `Stop`.
struct ReceivedStream {
    /// `(first_sample_index, samples_per_packet, sequence)` per datagram.
    audio: Vec<(u64, u64, u64)>,
    boundaries: Vec<(String, u64)>,
    start_entry: Option<String>,
    last_audio_end_ms: u64,
    stop_time_ms: u64,
}

impl ReceivedStream {
    fn assert_contiguous(&self) {
        for pair in self.audio.windows(2) {
            let ((first, length, sequence), (next, _, next_sequence)) = (pair[0], pair[1]);
            assert_eq!(next, first + length, "no gap or overlap between packets");
            assert_eq!(next_sequence, sequence + 1, "packet sequence is continuous");
        }
    }

    fn total_samples(&self) -> u64 {
        self.audio.iter().map(|packet| packet.1).sum()
    }
}

/// Collects audio and track boundaries until `Stop`, then drains briefly so
/// UDP audio racing the TCP `Stop` is still counted.
fn receive_until_stop(listener: &mut dyn ListenerTransportNode) -> ReceivedStream {
    let mut stream = ReceivedStream {
        audio: Vec::new(),
        boundaries: Vec::new(),
        start_entry: None,
        last_audio_end_ms: 0,
        stop_time_ms: 0,
    };
    let deadline = Instant::now() + TEST_TIMEOUT;
    let mut stopped_at = None;
    loop {
        let wait = if stopped_at.is_some() {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(200)
        };
        match listener.recv_event(wait) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(audio),
                ..
            }) => {
                let samples = u64::from(audio.samples_per_packet);
                stream.audio.push((
                    audio.first_sample_index.get(),
                    samples,
                    audio.sequence.get(),
                ));
                let end = audio.host_presentation_time_ms.get()
                    + samples * 1_000 / u64::from(audio.sample_rate);
                stream.last_audio_end_ms = stream.last_audio_end_ms.max(end);
            }
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) => match message {
                ControlMessage::TrackBoundary(boundary) => stream
                    .boundaries
                    .push((boundary.entry_id, boundary.first_sample_index.get())),
                ControlMessage::StreamStart(start) => {
                    stream.start_entry = start.current_entry_id;
                }
                ControlMessage::Stop(stop) => {
                    stream.stop_time_ms = stop.host_stop_time_ms.get();
                    stopped_at = Some(Instant::now());
                }
                _ => {}
            },
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {
                if stopped_at.is_some() {
                    break;
                }
            }
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(Instant::now() < deadline, "stream never stopped");
    }
    stream.audio.sort_unstable();
    stream
}
