//! Queue-edit races the commit protocol names (test 4: "repeat under
//! pause/resume, natural queue end, decoder failure, and a disconnected
//! consumer") on the production path both hosts use: the FFI producer
//! reserving each successor with a real core actor (`open_authoritative`).
//! The 2026-11-18 coverage survey found these combinations untested at
//! every layer.

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
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Current-track frames: more packets than the producer's 32-frame output
/// queue, so without a consumer it cannot reach its first reservation.
const CURRENT_FRAMES: u32 = 9_600;
const SUCCESSOR_FRAMES: u32 = 480;

struct Observer(mpsc::Sender<FfiCoreNotification>);

impl FfiCoreObserver for Observer {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        let _ = self.0.send(notification);
        Ok(())
    }
}

/// A host actor playing its first queue entry, with `upcoming` appended, and
/// a fixture directory whose WAV for entry `id` is `<id>.wav`.
struct PlayingHost {
    ffi: Arc<FfiCoreHandle>,
    actor: CoreActorHandle,
    notifications: mpsc::Receiver<FfiCoreNotification>,
    current_id: String,
    stream_id: StreamId,
    root: PathBuf,
}

impl Drop for PlayingHost {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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

fn playback(actor: &CoreActorHandle, state: PlaybackState) {
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(state))
        .expect("playback state");
    snapshot_when(actor, |s| s.playback_state == state);
}

fn descriptor(id: &str, frames: u32) -> AudioSourceDescriptor {
    AudioSourceDescriptor::new(
        format!("{id}-source"),
        format!("{id}.wav"),
        Some(u64::from(frames) * 4),
        None,
    )
    .expect("source descriptor")
}

/// Writes `<id>.wav` for the current entry and each upcoming one that has a
/// sample value; returns the fixture directory.
fn write_fixtures(name: &str, current_id: &str, upcoming: &[(&str, Option<i16>)]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-{name}-{unique}"));
    fs::create_dir_all(&root).expect("fixtures");
    fs::write(
        root.join(format!("{current_id}.wav")),
        wav(CURRENT_FRAMES, 3_000),
    )
    .expect("current PCM");
    for (id, sample) in upcoming {
        if let Some(sample) = sample {
            fs::write(
                root.join(format!("{id}.wav")),
                wav(SUCCESSOR_FRAMES, *sample),
            )
            .expect("PCM");
        }
    }
    root
}

impl PlayingHost {
    /// `upcoming` is `(entry id, PCM sample value)`; `None` leaves the file
    /// missing, so that entry fails when the producer opens it.
    fn start(name: &str, upcoming: &[(&str, Option<i16>)]) -> Self {
        let (tx, notifications) = mpsc::channel();
        let ffi =
            FfiCoreHandle::open(format!("ffi-{name}"), Arc::new(Observer(tx))).expect("open actor");
        let actor = ffi.actor_handle().expect("live actor");
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
                session_name: Some(name.to_owned()),
                approval_mode: Some(ApprovalMode::Manual),
                invite_code: InviteCodePatch::Unchanged,
                audio_source: AudioSourcePatch::Set(descriptor("current", CURRENT_FRAMES)),
                remember_approved_devices: Some(false),
            }),
        );
        snapshot_when(&actor, |s| s.host_draft.audio_source.is_some());
        submit(&actor, CoreCommand::CreateHostSession);
        let operation_id = loop {
            if let FfiCoreNotification::PlatformEffect {
                effect: FfiPlatformEffect::StartAdvertising { operation_id, .. },
            } = notifications
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
        if !upcoming.is_empty() {
            submit(
                &actor,
                CoreCommand::AppendHostQueue {
                    expected_queue_revision: ready.host_queue.revision(),
                    entries: upcoming
                        .iter()
                        .map(|(id, _)| {
                            QueueEntry::new(*id, descriptor(id, SUCCESSOR_FRAMES)).expect("entry")
                        })
                        .collect(),
                },
            );
            snapshot_when(&actor, |s| {
                s.host_queue.entries().len() == 1 + upcoming.len()
            });
        }
        playback(&actor, PlaybackState::Buffering);
        playback(&actor, PlaybackState::Playing);
        let stream_id = StreamId::new(format!("{name}-stream")).expect("stream");
        actor
            .submit_audio_event(AudioEvent::PositionAdvanced {
                stream_id: stream_id.clone(),
                position_ms: 0,
            })
            .expect("bind stream");

        let root = write_fixtures(name, &current_id, upcoming);
        Self {
            ffi,
            actor,
            notifications,
            current_id,
            stream_id,
            root,
        }
    }

    fn source(&self, id: &str) -> FfiPlaylistSource {
        let path: &Path = &self.root.join(format!("{id}.wav"));
        FfiPlaylistSource {
            entry_id: id.to_owned(),
            display_name: format!("{id} title"),
            staged_path: path.to_string_lossy().into_owned(),
        }
    }

    /// The production producer, opened before any consumer reads.
    fn producer(&self, upcoming: &[&str]) -> Arc<FfiHostPlaylistStreamer> {
        let mut sources = vec![self.source(&self.current_id)];
        sources.extend(upcoming.iter().map(|id| self.source(id)));
        let producer = FfiHostPlaylistStreamer::open_authoritative(
            "ffi-race-session".into(),
            self.stream_id.to_string(),
            5_000,
            sources,
            Arc::clone(&self.ffi),
        )
        .expect("open authoritative producer");
        assert!(
            self.actor
                .current_snapshot()
                .expect("snapshot")
                .host_queue_next_boundary
                .is_none(),
            "a backpressured producer has not reserved yet"
        );
        producer
    }

    fn remove(&self, entry_id: &str) {
        let before = self.actor.current_snapshot().expect("snapshot");
        let len = before.host_queue.entries().len();
        submit(
            &self.actor,
            CoreCommand::RemoveHostQueueEntry {
                expected_queue_revision: before.host_queue.revision(),
                entry_id: entry_id.to_owned(),
            },
        );
        snapshot_when(&self.actor, |s| s.host_queue.entries().len() == len - 1);
    }
}

/// What the consumer read: per-packet first sample value and the boundaries.
#[derive(Debug, Default)]
struct Drained {
    samples: u64,
    values: Vec<(u64, i16)>,
    boundaries: Vec<(String, u64)>,
    ended: bool,
    error: Option<String>,
}

/// Reads until End or an error, checking contiguity and sequence on the way.
fn drain(producer: &FfiHostPlaylistStreamer) -> Drained {
    let mut drained = Drained::default();
    let mut last_sequence = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "the producer never finished");
        match producer.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                sequence,
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            })) => {
                assert_eq!(first_sample_index, drained.samples, "sample gap or overlap");
                if let Some(previous) = last_sequence {
                    assert_eq!(sequence, previous + 1, "packet sequence gap");
                }
                last_sequence = Some(sequence);
                drained.values.push((
                    first_sample_index,
                    i16::from_le_bytes([payload[0], payload[1]]),
                ));
                drained.samples += u64::from(samples_per_packet);
            }
            Ok(Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            })) => {
                assert_eq!(
                    first_sample_index, drained.samples,
                    "boundary off the sample timeline"
                );
                drained.boundaries.push((entry_id, first_sample_index));
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                drained.ended = true;
                return drained;
            }
            Ok(None) => {}
            Err(error) => {
                drained.error = Some(error.to_string());
                return drained;
            }
        }
    }
}

/// A Pause before the producer reserves its successor, an edit during the
/// pause, then Resume: the producer retargets while paused, and after resume
/// only the replacement is reserved, announced and heard.
#[test]
fn a_paused_edit_before_reservation_retargets_and_only_the_replacement_plays_after_resume() {
    let host = PlayingHost::start(
        "paused-edit",
        &[("obsolete", Some(6_000)), ("replacement", Some(12_000))],
    );
    let producer = host.producer(&["obsolete", "replacement"]);

    playback(&host.actor, PlaybackState::Paused);
    host.remove("obsolete");
    let paused = host.actor.current_snapshot().expect("paused queue");
    assert_eq!(paused.playback_state, PlaybackState::Paused);
    assert_eq!(paused.host_queue.upcoming()[0].entry_id, "replacement");
    producer
        .replace_upcoming(vec![host.source("replacement")])
        .expect("retarget while paused");
    playback(&host.actor, PlaybackState::Playing);

    let drained = drain(&producer);
    assert!(drained.ended, "{:?}", drained.error);
    let boundary = u64::from(CURRENT_FRAMES);
    assert_eq!(drained.boundaries, [("replacement".to_owned(), boundary)]);
    assert_eq!(drained.samples, boundary + u64::from(SUCCESSOR_FRAMES));
    assert!(
        drained
            .values
            .iter()
            .all(|&(at, value)| value == if at < boundary { 3_000 } else { 12_000 }),
        "removed track PCM escaped: {:?}",
        drained
            .values
            .iter()
            .filter(|(_, v)| *v == 6_000)
            .collect::<Vec<_>>()
    );
    let reserved = host.actor.current_snapshot().expect("reserved");
    assert_eq!(
        reserved
            .host_queue_next_boundary
            .map(|b| b.entry_id)
            .as_deref(),
        Some("replacement")
    );
    producer.cancel().expect("cancel");
}

/// Removing the only successor before it is reserved makes the current track
/// the last: no boundary, a clean End, nothing reserved, and the actor's
/// end-of-stream is a natural queue end.
#[test]
fn removing_the_only_successor_before_reservation_ends_the_queue_after_the_current_track() {
    let host = PlayingHost::start("edit-to-end", &[("removed", Some(6_000))]);
    let producer = host.producer(&["removed"]);

    host.remove("removed");
    producer.replace_upcoming(Vec::new()).expect("no successor");

    let drained = drain(&producer);
    assert!(drained.ended, "{:?}", drained.error);
    assert!(drained.boundaries.is_empty(), "{:?}", drained.boundaries);
    assert_eq!(drained.samples, u64::from(CURRENT_FRAMES));
    assert!(
        drained.values.iter().all(|&(_, value)| value == 3_000),
        "removed PCM escaped"
    );
    assert!(
        host.actor
            .current_snapshot()
            .expect("snapshot")
            .host_queue_next_boundary
            .is_none()
    );

    host.actor
        .submit_audio_event(AudioEvent::EndOfStream {
            stream_id: host.stream_id.clone(),
        })
        .expect("end of stream");
    let ended = snapshot_when(&host.actor, |s| s.playback_state == PlaybackState::Stopped);
    assert!(
        ended.stream_ended_naturally,
        "the queue ran out; it did not fail or stop"
    );
    assert!(ended.last_error.is_none(), "{:?}", ended.last_error);
    producer.cancel().expect("cancel");
}

/// An edit that queues a source the producer cannot open, before the
/// reservation: the failure names that song, nothing is reserved or
/// announced for it, every current-track sample was delivered, and the
/// actor keeps the failed entry for retry.
#[test]
fn an_edit_that_queues_an_unreadable_successor_fails_before_any_boundary_or_reservation() {
    let host = PlayingHost::start("edit-to-failure", &[("good", Some(6_000))]);
    let producer = host.producer(&["good"]);

    let before = host.actor.current_snapshot().expect("snapshot");
    submit(
        &host.actor,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: before.host_queue.revision(),
            entries: vec![
                QueueEntry::new("missing", descriptor("missing", SUCCESSOR_FRAMES)).expect("entry"),
            ],
        },
    );
    snapshot_when(&host.actor, |s| s.host_queue.entries().len() == 3);
    host.remove("good");
    producer
        .replace_upcoming(vec![host.source("missing")])
        .expect("retarget to the unreadable source");

    let drained = drain(&producer);
    let error = drained
        .error
        .expect("an unreadable successor fails the stream");
    assert!(
        error.contains("missing title"),
        "names the failed song: {error}"
    );
    assert!(
        drained.boundaries.is_empty(),
        "a failed source was announced: {:?}",
        drained.boundaries
    );
    assert_eq!(
        drained.samples,
        u64::from(CURRENT_FRAMES),
        "the current track played out"
    );
    let snapshot = host.actor.current_snapshot().expect("snapshot");
    assert!(
        snapshot.host_queue_next_boundary.is_none(),
        "nothing reserved for the failed song"
    );
    assert!(
        snapshot
            .host_queue
            .entries()
            .iter()
            .any(|entry| entry.entry_id == "missing")
    );
    let _ = producer.cancel();
}

/// The consumer goes away after the producer reserved its successor but
/// before reading the boundary: dropping the producer joins its worker
/// without hanging, and a later Stop discards the orphaned reservation and
/// keeps the queue, so a replay can reserve again.
#[test]
fn a_consumer_that_disappears_after_a_reservation_releases_the_producer_and_stop_discards_it() {
    let host = PlayingHost::start("consumer-gone", &[("next", Some(12_000))]);
    let producer = host.producer(&["next"]);

    // Read just enough to let the worker reach its reservation. The actor
    // publishes its snapshot asynchronously, so give it time after each read
    // instead of racing ahead to the boundary.
    let mut read = 0;
    let reserved = |actor: &CoreActorHandle| {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if actor
                .current_snapshot()
                .expect("snapshot")
                .host_queue_next_boundary
                .is_some()
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    while !reserved(&host.actor) {
        assert!(read < 40, "the producer never reserved");
        if let Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) = producer.next_frame(200) {
            panic!("the boundary was read; the consumer must vanish before it");
        }
        read += 1;
    }
    assert!(
        read < 40,
        "the current track's 40 packets were not all read: the boundary is still queued"
    );
    let dropped_at = Instant::now();
    drop(producer);
    assert!(
        dropped_at.elapsed() < Duration::from_secs(3),
        "dropping the producer hung"
    );

    playback(&host.actor, PlaybackState::Stopped);
    let stopped = host.actor.current_snapshot().expect("stopped");
    assert!(
        stopped.host_queue_next_boundary.is_none(),
        "Stop discards the orphaned reservation"
    );
    assert!(
        stopped
            .host_queue
            .entries()
            .iter()
            .any(|entry| entry.entry_id == "next")
    );
    while host.notifications.try_recv().is_ok() {}
}
