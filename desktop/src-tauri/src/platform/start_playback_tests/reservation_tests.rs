//! The boundary commit point on the desktop production path, reservation
//! first (producer boundary commit protocol, test 5; gapless TODO §4 race
//! permutations). The edit-first order is covered by
//! `desktop_live_queue_removal_cancels_the_prebuffer_and_targets_the_replacement`.
//! Here the real desktop producer reserves the successor with the actor
//! first; a later remove of that entry is refused, the queue keeps it, and
//! the listener receives the boundary for exactly that entry.

use super::fixtures::{replace_source, stage_long_source};
use super::harness::{
    TEST_TIMEOUT, join_and_approve_listener, real_private_lan_address, start_host_session, submit,
    wait_for_audio, wait_for_stream_start, wait_snapshot,
};
use crate::platform::start_playback;
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};
use silent_disco_core::runtime::{CoreCommand, CoreCommandRequest, CoreNotification, QueueEntry};
use silent_disco_core::transport::{TransportChannel, TransportEvent};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[test]
#[allow(clippy::too_many_lines)] // One acceptance scenario over the real producer.
fn a_successor_the_producer_already_reserved_cannot_be_removed_and_still_plays() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this host; skipping");
        return;
    };
    let temp = TempDir::new().expect("temp");
    let (current, registry) = stage_long_source(&temp);
    let reserved = replace_source(&temp, &registry, "desktop-reserved-next");
    let later = replace_source(&temp, &registry, "desktop-reserved-later");
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(current, interface_name, interface_index, address);
    let before = handle.current_snapshot().expect("queue before append");
    submit(
        &handle,
        before.revision.get(),
        CoreCommand::AppendHostQueue {
            expected_queue_revision: before.host_queue.revision(),
            entries: vec![
                QueueEntry::new("reserved-next", reserved).expect("next entry"),
                QueueEntry::new("reserved-later", later).expect("later entry"),
            ],
        },
    );
    wait_snapshot(&handle, |snapshot| snapshot.host_queue.entries().len() == 3);
    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );
    start_playback::start(&handle, &network, &registry).expect("start playlist");
    let _ = wait_for_stream_start(&mut *listener);
    let _ = wait_for_audio(&mut *listener);

    // The producer reserves the successor with the actor before committing
    // its boundary, while the current track is still playing.
    let reservation = wait_snapshot(&handle, |snapshot| {
        snapshot
            .host_queue_next_boundary
            .as_ref()
            .is_some_and(|boundary| boundary.entry_id == "reserved-next")
    });
    assert_eq!(
        reservation.host_queue.upcoming()[0].entry_id,
        "reserved-next",
        "reserved while the first track is still current"
    );

    let receipt = handle
        .submit_command(
            CoreCommandRequest::new(
                reservation.revision,
                CoreCommand::RemoveHostQueueEntry {
                    expected_queue_revision: reservation.host_queue.revision(),
                    entry_id: "reserved-next".to_owned(),
                },
            )
            .expect("command"),
        )
        .expect("submit remove");
    let deadline = Instant::now() + TEST_TIMEOUT;
    let refusal = loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the remove was neither applied nor refused in time");
        match receiver.recv_timeout(left) {
            Ok(CoreNotification::Error(error))
                if error.operation_id.as_ref() == Some(&receipt.operation_id) =>
            {
                break error.message;
            }
            Ok(_) => {}
            Err(error) => panic!("observer failed: {error}"),
        }
    };
    assert!(
        refusal.contains("already scheduled next track cannot be removed"),
        "refused because it is reserved, not for another reason: {refusal}"
    );
    assert!(
        handle
            .current_snapshot()
            .expect("snapshot")
            .host_queue
            .entries()
            .iter()
            .any(|entry| entry.entry_id == "reserved-next"),
        "the reserved entry stays queued"
    );

    // The committed successor is the one the listener hears next.
    let boundary_entry = loop {
        match listener.recv_event(Duration::from_secs(10)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)),
                ..
            }) => break boundary.entry_id,
            Ok(_) => {}
            Err(error) => panic!("listener did not receive the boundary: {error}"),
        }
    };
    assert_eq!(boundary_entry, "reserved-next");

    network.stop_playback().expect("stop playback");
    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("host shutdown");
    actor.shutdown().expect("actor shutdown");
}
