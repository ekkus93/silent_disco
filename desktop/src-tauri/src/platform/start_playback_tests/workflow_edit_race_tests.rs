//! Queue edits racing the producer's successor reservation on the desktop
//! production path (gapless TODO §5): a real profile (`HostWorkflow`), the
//! edit commands the frontend calls (`host_workflow_ops`, which retarget the
//! running producer from the actor's snapshot), pause and resume through
//! `DesktopAppState`, and a real listener on the LAN. Before these, the
//! desktop path covered only removal (edit-first in `streaming_tests`,
//! reservation-first in `reservation_tests`).
//!
//! The desktop sends audio up to 1 s ahead, so a successor is reserved about
//! 1 s before its boundary is applied: an edit made as soon as the
//! reservation shows lands inside that window. The listener is read on its
//! own thread from the start, as a real one would be, so its bounded receive
//! queue never overflows while the test edits.

use super::fixtures::{long_pcm_wav, pcm_wav};
use super::workflow_harness::{HostWorkflow, ReceivedStream, receive_until_stop};
use crate::host_workflow_ops;
use crate::runtime_dto::{HostQueueEntryCommandRequest, ReorderHostQueueRequest};
use silent_disco_core::domain::PlaybackState;
use silent_disco_core::runtime::CoreSnapshot;
use silent_disco_core::transport::ListenerTransportNode;
use std::thread::JoinHandle;

/// 100 ms of 44.1 kHz source on the 48 kHz network timeline.
const SHORT: u64 = 4_800;
/// 3 s of 44.1 kHz source on the 48 kHz network timeline.
const LONG: u64 = 144_000;

macro_rules! open_or_skip {
    () => {
        match HostWorkflow::open() {
            Some(workflow) => workflow,
            None => {
                eprintln!("no private LAN interface on this host; skipping edit race test");
                return;
            }
        }
    };
}

fn reserved(snapshot: &CoreSnapshot, entry_id: &str) -> bool {
    snapshot
        .host_queue_next_boundary
        .as_ref()
        .is_some_and(|boundary| boundary.entry_id == entry_id)
}

fn reorder(workflow: &HostWorkflow, order: &[String]) {
    let (revision, queue_revision) = workflow.revisions();
    host_workflow_ops::reorder_host_queue(
        &workflow.state,
        &workflow.registry,
        ReorderHostQueueRequest {
            expected_revision: revision,
            expected_queue_revision: queue_revision,
            ordered_entry_ids: order.to_vec(),
        },
    )
    .expect("the command reaches the actor");
}

type Listening = JoinHandle<(ReceivedStream, Box<dyn ListenerTransportNode>)>;

/// Reads the listener until Stop on its own thread.
fn listen(mut listener: Box<dyn ListenerTransportNode>) -> Listening {
    std::thread::spawn(move || (receive_until_stop(&mut *listener), listener))
}

fn finish(listening: Listening) -> ReceivedStream {
    let (stream, mut listener) = listening.join().expect("listener thread");
    listener.shutdown().expect("listener shutdown");
    stream
}

/// The upcoming entries in order, whether or not the current one has
/// already moved on.
fn upcoming_after(workflow: &HostWorkflow, entry_id: &str) -> Vec<String> {
    let ids = workflow.queue_ids();
    ids.iter()
        .position(|id| id == entry_id)
        .map(|at| ids[at + 1..].to_vec())
        .unwrap_or_default()
}

#[test]
fn after_the_reservation_a_reorder_behind_it_plays_in_the_new_order_and_one_moving_it_is_refused() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("current.wav", long_pcm_wav()),
        ("reserved.wav", long_pcm_wav()),
        ("b.wav", pcm_wav()),
        ("c.wav", pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("queue four songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 4);
    let ids = workflow.queue_ids();
    let endpoint = workflow.create_session();
    let listening = listen(workflow.join_listener(endpoint));
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start the queue");
    workflow.wait(|snapshot| reserved(snapshot, &ids[1]));

    // Moving the reserved track is refused by the actor and changes nothing,
    // and the frontend command returns as soon as the refusal arrives
    // instead of waiting 2 s for a queue change that never comes (UI finding
    // 2026-11-18).
    let refused_at = std::time::Instant::now();
    reorder(
        &workflow,
        &[
            ids[0].clone(),
            ids[2].clone(),
            ids[1].clone(),
            ids[3].clone(),
        ],
    );
    let refused_after = refused_at.elapsed();
    assert!(
        refused_after < std::time::Duration::from_millis(500),
        "a refused reorder held the command for {refused_after:?}"
    );
    workflow
        .notifications
        .wait_for_error("cannot change already reserved upcoming tracks");
    assert_eq!(
        upcoming_after(&workflow, &ids[1]),
        [ids[2].clone(), ids[3].clone()]
    );

    // Reordering what follows it, through the frontend command, is accepted
    // and retargets the producer.
    let reordered = vec![
        ids[0].clone(),
        ids[1].clone(),
        ids[3].clone(),
        ids[2].clone(),
    ];
    reorder(&workflow, &reordered);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while upcoming_after(&workflow, &ids[1]) != [ids[3].clone(), ids[2].clone()] {
        assert!(
            std::time::Instant::now() < deadline,
            "the reorder was not applied"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let stream = finish(listening);
    assert_eq!(
        stream.boundaries,
        vec![
            (ids[1].clone(), LONG),
            (ids[3].clone(), 2 * LONG),
            (ids[2].clone(), 2 * LONG + SHORT),
        ],
    );
    stream.assert_contiguous();
    assert_eq!(stream.total_samples(), 2 * LONG + 2 * SHORT);
    workflow.close();
}

#[test]
fn a_song_added_after_the_reservation_plays_after_the_reserved_track() {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("current.wav", long_pcm_wav()),
        ("reserved.wav", long_pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("queue two songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 2);
    let ids = workflow.queue_ids();
    // Staged now, so the append itself lands inside the reservation window.
    let added = workflow
        .stage_songs(&workflow.pick(&[("added.wav", pcm_wav())]))
        .expect("stage the added song");
    let endpoint = workflow.create_session();
    let listening = listen(workflow.join_listener(endpoint));
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start the queue");
    workflow.wait(|snapshot| reserved(snapshot, &ids[1]));

    host_workflow_ops::append_staged_sources(&workflow.state, &workflow.registry, added)
        .expect("add a song behind the reserved track");
    let queued = workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let added_id = queued.host_queue.entries()[2].entry_id.clone();
    assert!(
        reserved(&queued, &ids[1]),
        "appended while the reserved track was still upcoming"
    );

    let stream = finish(listening);
    assert_eq!(
        stream.boundaries,
        vec![(ids[1].clone(), LONG), (added_id, 2 * LONG)]
    );
    stream.assert_contiguous();
    assert_eq!(stream.total_samples(), 2 * LONG + SHORT);
    workflow.close();
}

#[test]
fn a_removal_during_a_pause_before_the_reservation_retargets_and_the_replacement_plays_after_resume()
 {
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("current.wav", long_pcm_wav()),
        ("obsolete.wav", pcm_wav()),
        ("replacement.wav", pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("queue three songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let ids = workflow.queue_ids();
    let endpoint = workflow.create_session();
    let listening = listen(workflow.join_listener(endpoint));
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start the queue");
    workflow.wait(|snapshot| snapshot.playback_state == PlaybackState::Playing);

    workflow.state.pause_host_playback().expect("pause");
    workflow.wait(|snapshot| snapshot.playback_state == PlaybackState::Paused);
    assert!(
        workflow.snapshot().host_queue_next_boundary.is_none(),
        "paused near the start of a 3 s track: nothing reserved yet"
    );
    let (revision, queue_revision) = workflow.revisions();
    host_workflow_ops::remove_host_queue_entry(
        &workflow.state,
        &workflow.registry,
        HostQueueEntryCommandRequest {
            expected_revision: revision,
            expected_queue_revision: queue_revision,
            entry_id: ids[1].clone(),
        },
    )
    .expect("remove while paused");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 2);
    workflow.state.resume_host_playback().expect("resume");

    let stream = finish(listening);
    assert_eq!(stream.boundaries, vec![(ids[2].clone(), LONG)]);
    stream.assert_contiguous();
    assert_eq!(stream.total_samples(), LONG + SHORT);
    workflow.close();
}

/// Where in the reservation race the listener drops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DropPoint {
    /// The producer reserved the next track; its boundary is not sent yet.
    Reserved,
    /// The listener received the boundary; the host has not applied it.
    BoundarySent,
}

/// Reads `listener` on its own thread until `stop` is set, then hands it
/// back. Records whether a boundary for `watched` arrived.
fn read_until(
    listener: Box<dyn ListenerTransportNode>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    watched: String,
    saw_boundary: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> JoinHandle<Box<dyn ListenerTransportNode>> {
    use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};
    use silent_disco_core::transport::TransportEvent;
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if let Ok(TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)),
                ..
            }) = listener.recv_event(std::time::Duration::from_millis(20))
                && boundary.entry_id == watched
            {
                saw_boundary.store(true, Ordering::Release);
            }
        }
        listener
    })
}

/// A listener drops inside the reservation race and the same listener
/// rejoins while it is still open. The host neither fails nor loses the
/// reservation, and the rejoined listener is told the right current track
/// and every boundary from its join on, with no gap in its audio. At
/// `BoundarySent` it can only learn the pending boundary from the host's
/// replay of cached controls.
#[allow(clippy::too_many_lines)] // One end-to-end drop/rejoin scenario.
fn listener_drops_inside_the_race_and_rejoins(drop_point: DropPoint) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let workflow = open_or_skip!();
    let picked = workflow.pick(&[
        ("current.wav", long_pcm_wav()),
        ("reserved.wav", long_pcm_wav()),
        ("tail.wav", pcm_wav()),
    ]);
    workflow.add_songs(&picked).expect("queue three songs");
    workflow.wait(|snapshot| snapshot.host_queue.entries().len() == 3);
    let ids = workflow.queue_ids();
    let endpoint = workflow.create_session();
    let stop_reading = Arc::new(AtomicBool::new(false));
    let saw_boundary = Arc::new(AtomicBool::new(false));
    let first = read_until(
        workflow.join_listener(endpoint),
        Arc::clone(&stop_reading),
        ids[1].clone(),
        Arc::clone(&saw_boundary),
    );
    workflow
        .state
        .start_host_playback(&workflow.registry)
        .expect("start the queue");
    workflow.wait(|snapshot| reserved(snapshot, &ids[1]));
    if drop_point == DropPoint::BoundarySent {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !saw_boundary.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "the boundary was never sent"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    // Drop the listener inside the race.
    stop_reading.store(true, Ordering::Release);
    let mut dropped = first.join().expect("first listener thread");
    assert_eq!(
        saw_boundary.load(Ordering::Acquire),
        drop_point == DropPoint::BoundarySent,
        "dropped at {drop_point:?}"
    );
    dropped.shutdown().expect("listener drops");
    let alone = workflow.wait(|snapshot| snapshot.listeners.is_empty());
    assert!(
        reserved(&alone, &ids[1])
            || alone
                .host_queue
                .current()
                .is_some_and(|entry| entry.entry_id == ids[1]),
        "the reservation survives the listener leaving"
    );
    assert_eq!(alone.playback_state, PlaybackState::Playing);
    assert!(alone.last_error.is_none(), "{:?}", alone.last_error);

    // The same listener rejoins.
    let stream = finish(listen(workflow.join_listener(endpoint)));
    let start_entry = stream
        .start_entry
        .clone()
        .expect("a stream start on rejoin");
    eprintln!(
        "{drop_point:?}: rejoined with current track {start_entry} (before the reserved \
         boundary applied: {}), boundaries {:?}",
        start_entry == ids[0],
        stream.boundaries
    );
    assert_eq!(
        start_entry, ids[0],
        "rejoined while the race was still open (the reserved track not yet current)"
    );
    // A late joiner may also get earlier boundaries replayed; it must get
    // every one from its join on, in order, at the right samples.
    let expected = [(ids[1].clone(), LONG), (ids[2].clone(), 2 * LONG)];
    assert!(
        stream.boundaries.ends_with(&expected),
        "boundaries after rejoin: {:?}",
        stream.boundaries
    );
    stream.assert_contiguous();
    let joined_at = stream.audio.first().expect("audio after rejoin").0;
    // The host sends up to 1 s ahead, so a listener rejoining after the
    // boundary was sent gets its first audio already inside the reserved
    // track: only the replayed boundary tells it which track that audio is.
    assert!(
        joined_at < 2 * LONG,
        "rejoined before the last track: {joined_at}"
    );
    if drop_point == DropPoint::BoundarySent {
        assert!(
            joined_at >= LONG,
            "this run did not need the replay (first audio {joined_at})"
        );
    }
    assert_eq!(
        stream.audio.last().map(|(first, length, _)| first + length),
        Some(2 * LONG + SHORT),
        "the audio runs to the end of the queue"
    );

    let ended = workflow.wait(|snapshot| {
        snapshot.playback_state == PlaybackState::Stopped && snapshot.stream_ended_naturally
    });
    assert!(ended.last_error.is_none(), "{:?}", ended.last_error);
    workflow.close();
}

#[test]
fn a_listener_that_drops_after_the_reservation_and_rejoins_hears_the_right_tracks() {
    listener_drops_inside_the_race_and_rejoins(DropPoint::Reserved);
}

#[test]
fn a_listener_that_drops_after_the_boundary_is_sent_learns_it_again_on_rejoin() {
    listener_drops_inside_the_race_and_rejoins(DropPoint::BoundarySent);
}
