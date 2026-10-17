//! Deterministic queue-edit admission checks used by the producer boundary
//! reservation design. These tests cover queue revision linearization only;
//! they do not qualify the still-unwired producer/actor reservation.

use silent_disco_core::runtime::{AudioSourceDescriptor, HostQueue, HostQueueError, QueueEntry};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

fn entry(id: &str) -> QueueEntry {
    QueueEntry::new(
        id,
        AudioSourceDescriptor::new(format!("source-{id}"), id, Some(1024), None)
            .expect("source metadata"),
    )
    .expect("queue entry")
}

fn queue() -> HostQueue {
    let mut queue = HostQueue::default();
    queue
        .append(0, vec![entry("current"), entry("next"), entry("later")])
        .expect("initial queue");
    queue
}

#[test]
fn edit_winning_before_boundary_commit_changes_the_authoritative_successor() {
    let mut queue = queue();
    queue
        .remove(queue.revision(), "next", true)
        .expect("remove unlocked upcoming track");
    assert_eq!(queue.current().expect("current").entry_id, "current");
    assert_eq!(queue.upcoming()[0].entry_id, "later");
}

#[test]
fn competing_queue_edits_cannot_both_commit_from_one_revision() {
    for _ in 0..32 {
        let queue = Arc::new(Mutex::new(queue()));
        let revision = queue.lock().expect("queue").revision();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for target in ["next", "later"] {
            let queue = Arc::clone(&queue);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                queue.lock().expect("queue").remove(revision, target, true)
            }));
        }
        barrier.wait();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect();
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|result| matches!(result, Err(HostQueueError::StaleRevision { .. })))
                .count(),
            1
        );
        let final_queue = queue.lock().expect("queue");
        assert_eq!(final_queue.entries().len(), 2);
        assert_eq!(final_queue.current().expect("current").entry_id, "current");
    }
}
