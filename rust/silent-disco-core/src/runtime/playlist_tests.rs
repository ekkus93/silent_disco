use super::{
    HostQueue, HostQueueError, MAX_HOST_QUEUE_ENTRIES, MAX_HOST_QUEUE_SOURCE_BYTES, QueueEntry,
};
use crate::runtime::AudioSourceDescriptor;

fn entry(entry_id: &str, source_id: &str) -> QueueEntry {
    QueueEntry::new(
        entry_id,
        AudioSourceDescriptor::new(source_id, "Track.wav", Some(4_096), Some(2_000))
            .expect("valid source"),
    )
    .expect("valid entry")
}

fn ids(queue: &HostQueue) -> Vec<String> {
    queue
        .entries()
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect()
}

#[test]
fn append_is_atomic_and_allows_the_same_source_as_distinct_entries() {
    let mut queue = HostQueue::default();
    queue
        .append(0, vec![entry("one", "source"), entry("two", "source")])
        .expect("duplicate source entries are allowed");
    assert_eq!(ids(&queue), ["one", "two"]);
    assert_eq!(queue.revision(), 1);

    let before = queue.clone();
    let result = queue.append(1, vec![entry("three", "other"), entry("two", "other")]);
    assert_eq!(result, Err(HostQueueError::DuplicateEntryId));
    assert_eq!(queue, before);
}

#[test]
fn stale_mutations_and_oversized_append_leave_queue_unchanged() {
    let mut queue = HostQueue::default();
    queue.append(0, vec![entry("one", "one")]).expect("append");
    let before = queue.clone();
    assert!(matches!(
        queue.append(0, vec![entry("two", "two")]),
        Err(HostQueueError::StaleRevision { .. })
    ));
    assert_eq!(queue, before);

    let too_many = (0..MAX_HOST_QUEUE_ENTRIES)
        .map(|index| entry(&format!("entry-{index}"), &format!("source-{index}")))
        .collect();
    assert_eq!(
        queue.append(1, too_many),
        Err(HostQueueError::CapacityExceeded)
    );
    assert_eq!(queue, before);
}

#[test]
fn reorder_is_atomic_and_locks_current_when_playing() {
    let mut queue = HostQueue::default();
    queue
        .append(
            0,
            vec![
                entry("current", "one"),
                entry("next", "two"),
                entry("later", "three"),
            ],
        )
        .expect("append");
    assert_eq!(
        queue.reorder(1, &["next".into(), "current".into(), "later".into()], true),
        Err(HostQueueError::CurrentEntryLocked)
    );
    assert_eq!(ids(&queue), ["current", "next", "later"]);
    queue
        .reorder(1, &["current".into(), "later".into(), "next".into()], true)
        .expect("reorder upcoming");
    assert_eq!(ids(&queue), ["current", "later", "next"]);
    queue
        .reorder(2, &["next".into(), "current".into(), "later".into()], false)
        .expect("setup may reorder the first item");
    assert_eq!(
        queue.current().map(|item| item.entry_id.as_str()),
        Some("next")
    );
}

#[test]
fn invalid_order_remove_and_current_lock_do_not_mutate_state() {
    let mut queue = HostQueue::default();
    queue
        .append(0, vec![entry("current", "one"), entry("upcoming", "two")])
        .expect("append");
    let before = queue.clone();
    assert_eq!(
        queue.reorder(1, &["current".into(), "current".into()], true),
        Err(HostQueueError::InvalidOrder)
    );
    assert_eq!(
        queue.remove(1, "current", true),
        Err(HostQueueError::CurrentEntryLocked)
    );
    assert_eq!(
        queue.remove(1, "missing", false),
        Err(HostQueueError::EntryNotFound)
    );
    assert_eq!(queue, before);
}

#[test]
fn completing_tracks_promotes_next_and_leaves_an_empty_stopped_queue_at_end() {
    let mut queue = HostQueue::default();
    queue
        .append(0, vec![entry("one", "one"), entry("two", "two")])
        .expect("append");
    queue.complete_current(1).expect("finish first");
    assert_eq!(
        queue.current().map(|item| item.entry_id.as_str()),
        Some("two")
    );
    queue.complete_current(2).expect("finish last");
    assert!(queue.entries().is_empty());
    queue
        .append(3, vec![entry("new", "new")])
        .expect("append while idle");
    assert_eq!(
        queue.current().map(|item| item.entry_id.as_str()),
        Some("new")
    );
}

#[test]
fn clearing_a_live_queue_keeps_its_locked_current_item() {
    let mut queue = HostQueue::default();
    queue
        .append(0, vec![entry("current", "one"), entry("upcoming", "two")])
        .expect("append");
    queue.clear(1, true).expect("clear upcoming entries");
    assert_eq!(ids(&queue), ["current"]);
    queue.clear(2, false).expect("clear stopped queue");
    assert!(queue.entries().is_empty());
}

#[test]
fn entry_ids_are_bounded_and_safe() {
    let source = AudioSourceDescriptor::new("source", "Track.wav", None, None).expect("source");
    assert_eq!(
        QueueEntry::new(" bad", source.clone()),
        Err(HostQueueError::InvalidEntryId)
    );
    assert_eq!(
        QueueEntry::new("bad\n", source),
        Err(HostQueueError::InvalidEntryId)
    );
}

#[test]
fn unique_source_bytes_are_bounded_but_duplicate_entries_share_the_source_budget() {
    let large = |entry_id: &str, source_id: &str, byte_length| {
        QueueEntry::new(
            entry_id,
            AudioSourceDescriptor::new(source_id, "Large.wav", Some(byte_length), None)
                .expect("source descriptor"),
        )
        .expect("queue entry")
    };
    let mut queue = HostQueue::default();
    queue
        .append(
            0,
            vec![
                large("one", "shared", MAX_HOST_QUEUE_SOURCE_BYTES / 2),
                large("two", "shared", MAX_HOST_QUEUE_SOURCE_BYTES / 2),
                large("three", "second", MAX_HOST_QUEUE_SOURCE_BYTES / 2),
            ],
        )
        .expect("duplicates count once against source storage");
    let before = queue.clone();
    assert_eq!(
        queue.append(1, vec![large("four", "third", 1)],),
        Err(HostQueueError::SourceCapacityExceeded)
    );
    assert_eq!(queue, before);
}
