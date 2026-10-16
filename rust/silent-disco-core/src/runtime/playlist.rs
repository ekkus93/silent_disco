//! Validated bounded host playlist state shared by platform adapters.

use super::AudioSourceDescriptor;
use crate::audio::MAX_SOURCE_BYTES;
use core::fmt;
use std::error::Error;

pub const MAX_HOST_QUEUE_ENTRIES: usize = 100;
/// Total unique source size accepted into one host queue (duplicate entries
/// referencing the same opaque source count once).
pub const MAX_HOST_QUEUE_SOURCE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_QUEUE_ENTRY_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub entry_id: String,
    pub source: AudioSourceDescriptor,
}

impl QueueEntry {
    /// # Errors
    /// Returns [`HostQueueError::InvalidEntryId`] for an empty, oversized,
    /// whitespace-padded, or control-character-containing ID.
    pub fn new(
        entry_id: impl Into<String>,
        source: AudioSourceDescriptor,
    ) -> Result<Self, HostQueueError> {
        let entry_id = entry_id.into();
        if entry_id.is_empty()
            || entry_id.len() > MAX_QUEUE_ENTRY_ID_BYTES
            || entry_id.trim() != entry_id
            || entry_id.chars().any(char::is_control)
        {
            return Err(HostQueueError::InvalidEntryId);
        }
        Ok(Self { entry_id, source })
    }
}

/// Ordered queue with the selected/current item at index zero.
///
/// The source may appear more than once, but every queue entry has its own ID.
/// Platform source handles are deliberately absent; `source_id` is opaque.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostQueue {
    entries: Vec<QueueEntry>,
    revision: u64,
}

impl HostQueue {
    #[must_use]
    pub fn entries(&self) -> &[QueueEntry] {
        &self.entries
    }

    #[must_use]
    pub fn current(&self) -> Option<&QueueEntry> {
        self.entries.first()
    }

    #[must_use]
    pub fn upcoming(&self) -> &[QueueEntry] {
        self.entries.get(1..).unwrap_or_default()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds a set of prepared entries as one revision. Validation is complete
    /// before the queue changes, so a failed item cannot partially append.
    /// # Errors
    /// Returns an error for a stale revision, duplicate ID, capacity overflow,
    /// or exhausted revision counter.
    pub fn append(
        &mut self,
        expected_revision: u64,
        entries: Vec<QueueEntry>,
    ) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        if entries.is_empty() {
            return Ok(());
        }
        if self.entries.len().saturating_add(entries.len()) > MAX_HOST_QUEUE_ENTRIES {
            return Err(HostQueueError::CapacityExceeded);
        }
        let mut sources = Vec::new();
        for entry in &self.entries {
            if let Some((_, known_size)) = sources
                .iter()
                .find(|(source_id, _)| *source_id == entry.source.source_id)
            {
                if *known_size != entry.source.byte_length {
                    return Err(HostQueueError::InconsistentSourceMetadata);
                }
            } else {
                sources.push((entry.source.source_id.as_str(), entry.source.byte_length));
            }
        }
        let mut ids = self
            .entries
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect::<Vec<_>>();
        for entry in &entries {
            if ids.contains(&entry.entry_id.as_str()) {
                return Err(HostQueueError::DuplicateEntryId);
            }
            ids.push(&entry.entry_id);
            if let Some((_, known_size)) = sources
                .iter()
                .find(|(source_id, _)| *source_id == entry.source.source_id)
            {
                if *known_size != entry.source.byte_length {
                    return Err(HostQueueError::InconsistentSourceMetadata);
                }
            } else {
                sources.push((entry.source.source_id.as_str(), entry.source.byte_length));
            }
        }
        validate_source_total(&sources)?;
        let revision = self.next_revision()?;
        self.entries.extend(entries);
        self.revision = revision;
        Ok(())
    }

    /// Replaces every entry in one atomic revision. Used by the single-source
    /// compatibility path while older platform adapters are being migrated.
    /// # Errors
    /// Returns an error for a stale revision, duplicate ID, capacity overflow,
    /// or exhausted revision counter.
    pub fn replace_all(
        &mut self,
        expected_revision: u64,
        entries: Vec<QueueEntry>,
    ) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        if entries.len() > MAX_HOST_QUEUE_ENTRIES {
            return Err(HostQueueError::CapacityExceeded);
        }
        for (index, entry) in entries.iter().enumerate() {
            if entries[..index]
                .iter()
                .any(|previous| previous.entry_id == entry.entry_id)
            {
                return Err(HostQueueError::DuplicateEntryId);
            }
        }
        let mut sources = Vec::new();
        for entry in &entries {
            if let Some((_, known_size)) = sources
                .iter()
                .find(|(source_id, _)| *source_id == entry.source.source_id)
            {
                if *known_size != entry.source.byte_length {
                    return Err(HostQueueError::InconsistentSourceMetadata);
                }
            } else {
                sources.push((entry.source.source_id.as_str(), entry.source.byte_length));
            }
        }
        validate_source_total(&sources)?;
        if entries == self.entries {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.entries = entries;
        self.revision = revision;
        Ok(())
    }

    /// Replaces the complete order atomically. Callers pass the current item
    /// unchanged at the front while playback is active.
    /// # Errors
    /// Returns an error for a stale revision, invalid order, locked current
    /// item, or exhausted revision counter.
    pub fn reorder(
        &mut self,
        expected_revision: u64,
        ordered_entry_ids: &[String],
        current_locked: bool,
    ) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        if ordered_entry_ids.len() != self.entries.len() {
            return Err(HostQueueError::InvalidOrder);
        }
        if current_locked
            && self
                .current()
                .is_some_and(|current| ordered_entry_ids.first() != Some(&current.entry_id))
        {
            return Err(HostQueueError::CurrentEntryLocked);
        }
        let mut reordered = Vec::with_capacity(self.entries.len());
        for entry_id in ordered_entry_ids {
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.entry_id == *entry_id)
            else {
                return Err(HostQueueError::InvalidOrder);
            };
            if reordered
                .iter()
                .any(|entry: &QueueEntry| entry.entry_id == *entry_id)
            {
                return Err(HostQueueError::InvalidOrder);
            }
            reordered.push(self.entries[index].clone());
        }
        if reordered == self.entries {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.entries = reordered;
        self.revision = revision;
        Ok(())
    }

    /// # Errors
    /// Returns an error for a stale revision, missing entry, locked current
    /// item, or exhausted revision counter.
    pub fn remove(
        &mut self,
        expected_revision: u64,
        entry_id: &str,
        current_locked: bool,
    ) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.entry_id == entry_id)
        else {
            return Err(HostQueueError::EntryNotFound);
        };
        if current_locked && index == 0 {
            return Err(HostQueueError::CurrentEntryLocked);
        }
        let revision = self.next_revision()?;
        self.entries.remove(index);
        self.revision = revision;
        Ok(())
    }

    /// Completes the current item and promotes the next entry. At queue end,
    /// the queue becomes empty and subsequent additions remain stopped.
    /// # Errors
    /// Returns an error for a stale revision, empty queue, or exhausted
    /// revision counter.
    pub fn complete_current(&mut self, expected_revision: u64) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        if self.entries.is_empty() {
            return Err(HostQueueError::EntryNotFound);
        }
        let revision = self.next_revision()?;
        self.entries.remove(0);
        self.revision = revision;
        Ok(())
    }

    /// # Errors
    /// Returns an error for a stale revision or exhausted revision counter.
    /// If the current item is locked, it is retained and only upcoming items
    /// are cleared.
    pub fn clear(
        &mut self,
        expected_revision: u64,
        current_locked: bool,
    ) -> Result<(), HostQueueError> {
        self.check_revision(expected_revision)?;
        let retained = usize::from(current_locked && !self.entries.is_empty());
        if self.entries.len() == retained {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.entries.truncate(retained);
        self.revision = revision;
        Ok(())
    }

    fn check_revision(&self, expected: u64) -> Result<(), HostQueueError> {
        if expected == self.revision {
            Ok(())
        } else {
            Err(HostQueueError::StaleRevision {
                expected,
                current: self.revision,
            })
        }
    }

    fn next_revision(&self) -> Result<u64, HostQueueError> {
        self.revision
            .checked_add(1)
            .ok_or(HostQueueError::RevisionExhausted)
    }
}

fn validate_source_total(sources: &[(&str, Option<u64>)]) -> Result<(), HostQueueError> {
    let total = sources.iter().try_fold(0_u64, |total, (_, byte_length)| {
        total.checked_add(byte_length.unwrap_or(MAX_SOURCE_BYTES))
    });
    if total.is_none_or(|total| total > MAX_HOST_QUEUE_SOURCE_BYTES) {
        return Err(HostQueueError::SourceCapacityExceeded);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostQueueError {
    InvalidEntryId,
    DuplicateEntryId,
    CapacityExceeded,
    SourceCapacityExceeded,
    InconsistentSourceMetadata,
    InvalidOrder,
    CurrentEntryLocked,
    EntryNotFound,
    StaleRevision { expected: u64, current: u64 },
    RevisionExhausted,
}

impl fmt::Display for HostQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEntryId => formatter.write_str("queue entry ID is invalid"),
            Self::DuplicateEntryId => formatter.write_str("queue entry ID already exists"),
            Self::CapacityExceeded => formatter.write_str("host queue is at its entry limit"),
            Self::SourceCapacityExceeded => {
                formatter.write_str("host queue sources exceed the total size limit")
            }
            Self::InconsistentSourceMetadata => {
                formatter.write_str("queue entries disagree about a shared source")
            }
            Self::InvalidOrder => formatter.write_str("queue order does not match its entries"),
            Self::CurrentEntryLocked => formatter.write_str("the current queue entry is locked"),
            Self::EntryNotFound => formatter.write_str("queue entry was not found"),
            Self::StaleRevision { expected, current } => write!(
                formatter,
                "queue revision changed (expected {expected}, current {current})"
            ),
            Self::RevisionExhausted => formatter.write_str("queue revision is exhausted"),
        }
    }
}

impl Error for HostQueueError {}

#[cfg(test)]
#[path = "playlist_tests.rs"]
mod tests;
