use super::*;

mod track_switch_tests {
    use super::{ActorState, AppRole, HostLifecycle, OperationId, PlaybackState};
    use crate::domain::DeviceId;
    use crate::error::CoreErrorCode;
    use crate::runtime::AudioSourceDescriptor;
    use crate::runtime::types::{AudioSourcePatch, HostDraftPatch, InviteCodePatch};

    fn stopped_ready_host() -> ActorState {
        let mut state =
            ActorState::new(DeviceId::new("host-track-switch-test").expect("valid device ID"));
        state.snapshot.selected_role = Some(AppRole::Host);
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Stopped;
        state
    }

    fn audio_source_only_patch(source_id: &str) -> HostDraftPatch {
        HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(
                AudioSourceDescriptor::new(source_id, "fixture.wav", Some(4_096), Some(2_000))
                    .expect("valid staged source descriptor"),
            ),
            remember_approved_devices: None,
        }
    }

    #[test]
    fn a_pure_audio_source_swap_is_allowed_once_a_hosting_session_has_stopped() {
        let mut state = stopped_ready_host();
        state
            .update_host_draft(
                OperationId::new("op-1").expect("valid operation ID"),
                &audio_source_only_patch("song-b"),
            )
            .expect("a pure track switch is allowed while ready and stopped");
        assert_eq!(
            state
                .snapshot
                .host_draft
                .audio_source
                .as_ref()
                .map(|source| source.source_id.clone()),
            Some("song-b".to_owned())
        );
    }

    #[test]
    fn the_same_swap_is_allowed_with_no_listeners_connected_yet() {
        let mut state = stopped_ready_host();
        state.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;
        state
            .update_host_draft(
                OperationId::new("op-1").expect("valid operation ID"),
                &audio_source_only_patch("song-b"),
            )
            .expect("a pure track switch is allowed while waiting for listeners and stopped");
    }

    #[test]
    fn a_track_switch_is_rejected_while_still_playing() {
        let mut state = stopped_ready_host();
        state.snapshot.playback_state = PlaybackState::Playing;
        let error = state
            .update_host_draft(
                OperationId::new("op-1").expect("valid operation ID"),
                &audio_source_only_patch("song-b"),
            )
            .expect_err("a track switch must be rejected while still playing");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
    }

    #[test]
    fn a_patch_touching_the_session_name_is_rejected_even_while_stopped() {
        let mut state = stopped_ready_host();
        let mut patch = audio_source_only_patch("song-b");
        patch.session_name = Some("Renamed session".to_owned());
        let error = state
            .update_host_draft(
                OperationId::new("op-1").expect("valid operation ID"),
                &patch,
            )
            .expect_err(
                "a patch touching session identity fields must stay rejected during a live \
                 session, even alongside an otherwise-legal track switch",
            );
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.host_draft.session_name, "");
    }
}

mod host_queue_actor_tests {
    use super::{ActorState, AppRole, HostLifecycle, OperationId, PlaybackState};
    use crate::domain::{DeviceId, StreamId};
    use crate::error::CoreErrorCode;
    use crate::runtime::{AudioEvent, AudioSourceDescriptor, QueueEntry};

    fn host() -> ActorState {
        let mut state = ActorState::new(DeviceId::new("host-queue-test").expect("valid device ID"));
        state.snapshot.selected_role = Some(AppRole::Host);
        state
    }

    fn entry(id: &str, source: &str) -> QueueEntry {
        QueueEntry::new(
            id,
            AudioSourceDescriptor::new(source, "Track.wav", Some(4_096), Some(2_000))
                .expect("valid source"),
        )
        .expect("valid queue entry")
    }

    #[test]
    fn actor_queue_append_and_reorder_update_the_snapshot_and_legacy_source() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append").expect("operation ID"),
                0,
                vec![entry("one", "same"), entry("two", "same")],
            )
            .expect("append");
        assert_eq!(state.snapshot.host_queue.entries().len(), 2);
        assert_eq!(
            state
                .snapshot
                .host_draft
                .audio_source
                .as_ref()
                .unwrap()
                .source_id,
            "same"
        );
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .reorder_host_queue(
                OperationId::new("reorder").expect("operation ID"),
                1,
                vec!["one".into(), "two".into()],
            )
            .expect("unchanged active order is allowed");
        let error = state
            .remove_host_queue_entry(OperationId::new("remove").expect("operation ID"), 1, "one")
            .expect_err("playing current entry is locked");
        assert_eq!(error.code, CoreErrorCode::InvalidArgument);
        assert_eq!(state.snapshot.host_queue.entries().len(), 2);
    }

    #[test]
    fn queue_edits_are_rejected_after_the_next_boundary_is_scheduled() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-scheduled").expect("operation ID"),
                0,
                vec![entry("one", "one"), entry("two", "two")],
            )
            .expect("append");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .apply_audio(AudioEvent::TrackBoundaryScheduled {
                entry_id: "two".into(),
                first_sample_index: 1_234,
            })
            .expect("schedule boundary");

        let error = state
            .remove_host_queue_entry(
                OperationId::new("remove-scheduled").expect("operation ID"),
                1,
                "two",
            )
            .expect_err("scheduled target must remain authoritative");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "two");
        state
            .append_host_queue(
                OperationId::new("append-after-scheduled").expect("operation ID"),
                1,
                vec![entry("three", "three")],
            )
            .expect("appending behind a fixed boundary keeps its target valid");
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "two");
        assert_eq!(state.snapshot.host_queue.entries().len(), 3);
        state
            .reorder_host_queue(
                OperationId::new("reorder-after-scheduled").expect("operation ID"),
                2,
                vec!["one".into(), "two".into(), "three".into()],
            )
            .expect("the fixed target may remain first while later entries are reordered");
    }

    #[test]
    fn an_unannounced_boundary_cannot_advance_the_queue_after_an_edit() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-unannounced").expect("operation ID"),
                0,
                vec![
                    entry("one", "source-one"),
                    entry("two", "source-two"),
                    entry("three", "source-three"),
                ],
            )
            .expect("append queue");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;

        // A queue mutation before scheduling is legal. A stale boundary
        // from an already-prefetched worker must not promote the new head
        // merely because its entry ID matches the revised upcoming item.
        state
            .remove_host_queue_entry(
                OperationId::new("remove-prefetched").expect("operation ID"),
                1,
                "two",
            )
            .expect("remove next entry before it was scheduled");
        let revision = state.snapshot.host_queue.revision();
        let error = state
            .apply_audio(AudioEvent::TrackBoundaryApplied {
                entry_id: "three".into(),
                first_sample_index: 48_000,
            })
            .expect_err("unannounced boundary must never advance the queue");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.host_queue.revision(), revision);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
        assert!(state.snapshot.host_queue_next_boundary.is_none());

        state
            .apply_audio(AudioEvent::TrackBoundaryScheduled {
                entry_id: "three".into(),
                first_sample_index: 48_000,
            })
            .expect("schedule the accepted next track");
        state
            .apply_audio(AudioEvent::TrackBoundaryApplied {
                entry_id: "three".into(),
                first_sample_index: 48_000,
            })
            .expect("exact scheduled boundary may advance the queue");
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "three");
    }

    #[test]
    fn a_stopped_boundary_is_not_resurrected_by_replay() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-replay-boundary").expect("operation ID"),
                0,
                vec![entry("one", "source-one"), entry("two", "source-two")],
            )
            .expect("append queue");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .apply_audio(AudioEvent::TrackBoundaryScheduled {
                entry_id: "two".into(),
                first_sample_index: 12_345,
            })
            .expect("schedule first stream's boundary");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("replay");
        assert!(state.snapshot.host_queue_next_boundary.is_none());

        let error = state
            .apply_audio(AudioEvent::TrackBoundaryApplied {
                entry_id: "two".into(),
                first_sample_index: 12_345,
            })
            .expect_err("boundary from stopped stream must not apply to replay");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
    }

    #[test]
    fn stopping_clears_scheduled_boundary_but_preserves_queue_for_replay() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-stop-replay").expect("operation ID"),
                0,
                vec![entry("one", "source-one"), entry("two", "source-two")],
            )
            .expect("append queue");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state.snapshot.playback_position_ms = 8_000;
        state
            .apply_audio(AudioEvent::TrackBoundaryScheduled {
                entry_id: "two".into(),
                first_sample_index: 384_000,
            })
            .expect("schedule next track");

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop playback");

        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert_eq!(state.snapshot.playback_position_ms, 0);
        assert!(state.snapshot.host_queue_next_boundary.is_none());
        assert_eq!(
            state
                .snapshot
                .host_queue
                .entries()
                .iter()
                .map(|entry| entry.entry_id.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(
            state
                .snapshot
                .host_draft
                .audio_source
                .as_ref()
                .map(|source| source.source_id.as_str()),
            Some("source-one")
        );

        let stale_boundary_error = state
            .apply_audio_control(AudioEvent::TrackBoundaryApplied {
                entry_id: "two".into(),
                first_sample_index: 384_000,
            })
            .expect_err("a canceled boundary must not advance a stopped queue");
        assert_eq!(
            stale_boundary_error.code,
            CoreErrorCode::InvalidStateTransition
        );
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("play restarts the current entry");
        assert_eq!(state.snapshot.playback_position_ms, 0);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
    }

    #[test]
    fn queue_add_is_rejected_for_listener_role_and_end_session_clears_source_alias() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append").expect("operation ID"),
                0,
                vec![entry("one", "one")],
            )
            .expect("append");
        state
            .clear_host_queue(OperationId::new("clear").expect("operation ID"), 1)
            .expect("clear");
        assert!(state.snapshot.host_queue.entries().is_empty());
        assert!(state.snapshot.host_draft.audio_source.is_none());

        state.snapshot.selected_role = Some(AppRole::Listener);
        let error = state
            .append_host_queue(
                OperationId::new("append-listener").expect("operation ID"),
                2,
                vec![entry("two", "two")],
            )
            .expect_err("listeners cannot mutate the host queue");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
    }

    #[test]
    fn ending_a_host_session_clears_authoritative_queue_and_source_alias() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append").expect("operation ID"),
                0,
                vec![entry("one", "one"), entry("two", "two")],
            )
            .expect("append");
        state.reset_host_session();
        assert!(state.snapshot.host_queue.entries().is_empty());
        assert!(state.snapshot.host_draft.audio_source.is_none());
    }

    #[test]
    fn delayed_end_of_stream_after_stop_preserves_queue_for_replay() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-stop-eof").expect("operation ID"),
                0,
                vec![entry("one", "one"), entry("two", "two")],
            )
            .expect("append");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("explicit Stop");

        let revision = state.snapshot.host_queue.revision();
        let error = state
            .apply_audio_control(AudioEvent::EndOfStream {
                stream_id: StreamId::new("stopped-stream").expect("stream ID"),
            })
            .expect_err("late EOF must not clear a stopped queue");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert_eq!(state.snapshot.host_queue.revision(), revision);
        assert_eq!(state.snapshot.host_queue.entries().len(), 2);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
        assert!(!state.snapshot.stream_ended_naturally);
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("replay");
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
    }

    #[test]
    fn delayed_end_of_stream_after_failure_keeps_retryable_queue() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-error-eof").expect("operation ID"),
                0,
                vec![entry("failed", "source-one"), entry("next", "source-two")],
            )
            .expect("append");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Error;
        let revision = state.snapshot.host_queue.revision();
        let error = state
            .apply_audio_control(AudioEvent::EndOfStream {
                stream_id: StreamId::new("failed-stream").expect("stream ID"),
            })
            .expect_err("late EOF must not erase retryable queue");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.playback_state, PlaybackState::Error);
        assert_eq!(state.snapshot.host_queue.revision(), revision);
        assert_eq!(state.snapshot.host_queue.entries().len(), 2);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "failed");
        assert!(!state.snapshot.stream_ended_naturally);
    }

    #[test]
    fn eof_after_late_append_stops_without_erasing_unplayed_song() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-current-before-eof").expect("operation ID"),
                0,
                vec![entry("one", "source-one")],
            )
            .expect("start with one song");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        // The packetizer may already have produced its final frame before
        // this authoritative append, leaving no way to emit the new song.
        state
            .append_host_queue(
                OperationId::new("append-after-producer-finished").expect("operation ID"),
                1,
                vec![entry("two", "source-two")],
            )
            .expect("late append is accepted");
        let revision = state.snapshot.host_queue.revision();
        state
            .apply_audio_control(AudioEvent::EndOfStream {
                stream_id: StreamId::new("old-stream").expect("stream ID"),
            })
            .expect("exhausted producer stops but cannot clear unplayed queue");

        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert!(!state.snapshot.stream_ended_naturally);
        assert_eq!(state.snapshot.host_queue.revision(), revision);
        assert_eq!(state.snapshot.host_queue.entries().len(), 2);
        assert_eq!(state.snapshot.host_queue.current().unwrap().entry_id, "one");
        assert_eq!(
            state.snapshot.host_draft.audio_source.as_ref().unwrap().source_id,
            "source-one"
        );
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("queue remains replayable");
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "two");
    }

    #[test]
    fn eof_before_a_scheduled_boundary_applies_preserves_the_queue() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append-boundary-before-eof").expect("operation ID"),
                0,
                vec![entry("one", "source-one"), entry("two", "source-two")],
            )
            .expect("append");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;
        state
            .apply_audio(AudioEvent::TrackBoundaryScheduled {
                entry_id: "two".into(),
                first_sample_index: 48_000,
            })
            .expect("schedule next song");
        let revision = state.snapshot.host_queue.revision();
        state
            .apply_audio_control(AudioEvent::EndOfStream {
                stream_id: StreamId::new("stream-with-pending-boundary").expect("stream ID"),
            })
            .expect("premature EOF stops transport without consuming queue");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert!(!state.snapshot.stream_ended_naturally);
        assert!(state.snapshot.host_queue_next_boundary.is_none());
        assert_eq!(state.snapshot.host_queue.revision(), revision);
        assert_eq!(state.snapshot.host_queue.upcoming()[0].entry_id, "two");
    }

    #[test]
    fn natural_end_clears_completed_queue_and_preserves_host_session() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append").expect("operation ID"),
                0,
                vec![entry("one", "one")],
            )
            .expect("append");
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Playing;

        state
            .apply_audio_control(AudioEvent::EndOfStream {
                stream_id: StreamId::new("stream-ended").expect("stream ID"),
            })
            .expect("end of stream");

        assert!(state.snapshot.host_queue.entries().is_empty());
        assert!(state.snapshot.host_draft.audio_source.is_none());
        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert!(state.snapshot.stream_ended_naturally);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Ready);
    }
}
