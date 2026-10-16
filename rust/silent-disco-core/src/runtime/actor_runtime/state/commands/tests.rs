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
    fn natural_end_clears_completed_queue_and_preserves_host_session() {
        let mut state = host();
        state
            .append_host_queue(
                OperationId::new("append").expect("operation ID"),
                0,
                vec![entry("one", "one"), entry("two", "two")],
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
