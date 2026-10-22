//! Synchronous queue-boundary reservation before audio producer commitment.
use super::{
    ActorState, AudioEvent, CoreNotification, ProcessResult, StreamId, invalid_argument,
    invalid_state, resource_limit,
};
use crate::error::CoreError;

impl ActorState {
    /// Called on the serialized actor thread. Queue edits and this reservation
    /// cannot both win an interleaving: whichever input is dequeued first
    /// determines whether the next entry may change.
    pub(in crate::runtime::actor_runtime) fn reserve_track_boundary(
        &mut self,
        stream_id: StreamId,
        entry_id: String,
        first_sample_index: u64,
    ) -> Result<ProcessResult, CoreError> {
        if self.active_audio_stream_id.as_ref() != Some(&stream_id) {
            return Err(invalid_state(
                "track boundary reservation does not match the active stream",
                None,
            ));
        }
        let mut candidate = self.clone();
        let mut outcome = candidate.apply_audio_control(AudioEvent::TrackBoundaryScheduled {
            entry_id,
            first_sample_index,
        })?;
        if outcome.changed {
            candidate.snapshot.revision = candidate
                .snapshot
                .revision
                .checked_next()
                .map_err(|_| resource_limit("snapshot revision overflow", None))?;
            candidate
                .snapshot
                .validate()
                .map_err(|error| invalid_argument(error.to_string(), None))?;
            outcome
                .notifications
                .insert(0, CoreNotification::Snapshot(candidate.snapshot.clone()));
        }
        *self = candidate;
        Ok(ProcessResult {
            notifications: outcome.notifications,
            stop_requested: outcome.stop_requested,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AppRole, DeviceId, HostLifecycle, PlaybackState, SessionId};
    use crate::error::CoreErrorCode;
    use crate::runtime::{AudioSourceDescriptor, CoreCommand, QueueEntry};

    fn entry(id: &str) -> QueueEntry {
        QueueEntry::new(
            id,
            AudioSourceDescriptor::new(id, "track.wav", Some(4096), None).expect("source"),
        )
        .expect("entry")
    }

    fn host() -> (ActorState, StreamId) {
        let mut state = ActorState::new(DeviceId::new("reservation-host").expect("device ID"));
        state.snapshot.selected_role = Some(AppRole::Host);
        state.host_session_id = Some(SessionId::new("reservation-session").expect("session ID"));
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .snapshot
            .host_queue
            .append(0, vec![entry("one"), entry("two"), entry("three")])
            .expect("queue");
        let stream_id = StreamId::new("active-reservation-stream").expect("stream ID");
        state.active_audio_stream_id = Some(stream_id.clone());
        (state, stream_id)
    }

    #[test]
    fn reservation_winning_first_locks_its_target_but_not_later_entries() {
        let (mut state, stream_id) = host();
        state
            .reserve_track_boundary(stream_id, "two".into(), 48_000)
            .expect("authoritative reservation");
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "two"
        );
        let remove_revision = state.snapshot.host_queue.revision();
        let error = state
            .apply_command(
                crate::domain::OperationId::new("remove-reserved").expect("operation ID"),
                CoreCommand::RemoveHostQueueEntry {
                    expected_queue_revision: remove_revision,
                    entry_id: "two".into(),
                },
            )
            .expect_err("reserved next entry cannot be removed");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        let append_revision = state.snapshot.host_queue.revision();
        state
            .apply_command(
                crate::domain::OperationId::new("append-behind").expect("operation ID"),
                CoreCommand::AppendHostQueue {
                    expected_queue_revision: append_revision,
                    entries: vec![entry("four")],
                },
            )
            .expect("later items remain editable");
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "two");
    }

    #[test]
    fn multiple_short_track_boundaries_reserve_before_the_first_is_audible() {
        let (mut state, stream_id) = host();
        state
            .reserve_track_boundary(stream_id.clone(), "two".into(), 4_800)
            .expect("first successor");
        state
            .reserve_track_boundary(stream_id.clone(), "three".into(), 9_600)
            .expect("second successor prebuffered before first is applied");
        assert_eq!(state.pending_host_boundaries.len(), 2);
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "two"
        );

        let revision = state.snapshot.host_queue.revision();
        assert!(
            state
                .apply_command(
                    crate::domain::OperationId::new("remove-third-reserved").unwrap(),
                    CoreCommand::RemoveHostQueueEntry {
                        expected_queue_revision: revision,
                        entry_id: "three".into(),
                    },
                )
                .is_err()
        );
        assert!(
            state
                .apply_command(
                    crate::domain::OperationId::new("reorder-reserved-prefix").unwrap(),
                    CoreCommand::ReorderHostQueue {
                        expected_queue_revision: revision,
                        ordered_entry_ids: vec!["one".into(), "three".into(), "two".into()],
                    },
                )
                .is_err()
        );

        state
            .apply_audio_control(AudioEvent::TrackBoundaryApplied {
                entry_id: "two".into(),
                first_sample_index: 4_800,
            })
            .expect("first boundary");
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "two");
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "three"
        );

        state
            .apply_audio_control(AudioEvent::TrackBoundaryApplied {
                entry_id: "three".into(),
                first_sample_index: 9_600,
            })
            .expect("second boundary");
        assert_eq!(
            state.snapshot.host_queue.current().unwrap().entry_id,
            "three"
        );
        assert!(state.pending_host_boundaries.is_empty());
        assert!(state.snapshot.host_queue_next_boundary.is_none());
    }

    #[test]
    fn pause_during_producer_reservation_preserves_the_next_track_until_resume() {
        let (mut state, stream_id) = host();
        state.snapshot.host_lifecycle = HostLifecycle::Streaming;
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
            .expect("pause while producer is prebuffering");
        state
            .reserve_track_boundary(stream_id, "two".into(), 4_800)
            .expect("paused producer may lock the next entry");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Paused);
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "two"
        );

        let revision = state.snapshot.host_queue.revision();
        assert!(
            state
                .apply_command(
                    crate::domain::OperationId::new("remove-during-pause").unwrap(),
                    CoreCommand::RemoveHostQueueEntry {
                        expected_queue_revision: revision,
                        entry_id: "two".into(),
                    },
                )
                .is_err(),
            "a paused producer reservation must still reject stale queue edits"
        );
        assert!(
            state
                .apply_audio_control(AudioEvent::TrackBoundaryApplied {
                    entry_id: "two".into(),
                    first_sample_index: 4_800,
                })
                .is_err(),
            "the next track cannot become current while paused"
        );

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("resume");
        state
            .apply_audio_control(AudioEvent::TrackBoundaryApplied {
                entry_id: "two".into(),
                first_sample_index: 4_800,
            })
            .expect("apply the reserved boundary after resume");
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "two");
        assert!(state.snapshot.host_queue_next_boundary.is_none());
    }

    #[test]
    fn stop_discards_all_prefetched_reservations_before_replay() {
        let (mut state, stream_id) = host();
        state
            .reserve_track_boundary(stream_id.clone(), "two".into(), 4_800)
            .expect("first successor");
        state
            .reserve_track_boundary(stream_id, "three".into(), 9_600)
            .expect("second successor");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop");
        assert!(state.pending_host_boundaries.is_empty());
        assert!(state.snapshot.host_queue_next_boundary.is_none());
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
    }

    #[test]
    fn old_stream_cannot_reserve_after_new_stream_starts() {
        let (mut state, _) = host();
        let revision = state.snapshot.revision;
        let error = state
            .reserve_track_boundary(
                StreamId::new("retired-stream").expect("stream ID"),
                "two".into(),
                48_000,
            )
            .expect_err("stale producer must not reserve");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.revision, revision);
        assert!(state.snapshot.host_queue_next_boundary.is_none());
    }

    #[test]
    fn queue_edit_winning_first_rejects_stale_target_reservation() {
        let (mut state, stream_id) = host();
        let queue_revision = state.snapshot.host_queue.revision();
        state
            .apply_command(
                crate::domain::OperationId::new("remove-before-reserve").unwrap(),
                CoreCommand::RemoveHostQueueEntry {
                    expected_queue_revision: queue_revision,
                    entry_id: "two".into(),
                },
            )
            .expect("actor accepts the edit before producer reservation");
        let revision = state.snapshot.revision;
        let error = state
            .reserve_track_boundary(stream_id.clone(), "two".into(), 48_000)
            .expect_err("removed target is stale");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.revision, revision);
        state
            .reserve_track_boundary(stream_id, "three".into(), 48_000)
            .expect("new authoritative successor");
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "three"
        );
    }

    #[test]
    fn queue_reorder_winning_first_rejects_the_old_successor_reservation() {
        let (mut state, stream_id) = host();
        let queue_revision = state.snapshot.host_queue.revision();
        state
            .apply_command(
                crate::domain::OperationId::new("reorder-before-reserve").unwrap(),
                CoreCommand::ReorderHostQueue {
                    expected_queue_revision: queue_revision,
                    ordered_entry_ids: vec!["one".into(), "three".into(), "two".into()],
                },
            )
            .expect("actor reorders unreserved upcoming tracks");
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "three");

        let revision = state.snapshot.revision;
        let error = state
            .reserve_track_boundary(stream_id.clone(), "two".into(), 4_800)
            .expect_err("old successor must not be reserved after a reorder");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.revision, revision);
        assert!(state.snapshot.host_queue_next_boundary.is_none());

        state
            .reserve_track_boundary(stream_id, "three".into(), 4_800)
            .expect("producer can reserve the newly selected successor");
        assert_eq!(
            state
                .snapshot
                .host_queue_next_boundary
                .as_ref()
                .unwrap()
                .entry_id,
            "three"
        );
    }
}
