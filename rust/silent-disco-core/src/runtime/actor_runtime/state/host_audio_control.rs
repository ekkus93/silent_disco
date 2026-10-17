use super::{
    ActorState, AppRole, ApplyOutcome, AudioEvent, CoreError, HostLifecycle, PlaybackState,
    invalid_state,
};

impl ActorState {
    pub(super) fn apply_audio_control(
        &mut self,
        event: AudioEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        if self.snapshot.selected_role != Some(AppRole::Host) || self.host_session_id.is_none() {
            return self.apply_audio_with_host_lifecycle(event);
        }

        // Atomically start a buffered host stream and bind its identity.
        // The actor processes this as one input, preventing an old EOF
        // from interleaving between Playing and stream registration.
        if let AudioEvent::PositionAdvanced { position_ms: 0, .. } = &event
            && self.snapshot.playback_state == PlaybackState::Buffering
        {
            let mut candidate = self.clone();
            candidate.apply_audio_with_host_lifecycle(AudioEvent::PlaybackStateChanged(
                PlaybackState::Playing,
            ))?;
            let outcome = candidate.apply_audio(event)?;
            *self = candidate;
            return Ok(outcome);
        }

        match &event {
            AudioEvent::PlaybackStateChanged(PlaybackState::Buffering) => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready | HostLifecycle::WaitingForListeners
                ) || !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Stopped | PlaybackState::Ready | PlaybackState::Error
                ) {
                    return Err(invalid_state(
                        "host playback can begin buffering only from an active stopped session",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Playing) => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready
                        | HostLifecycle::WaitingForListeners
                        | HostLifecycle::Paused
                        | HostLifecycle::Streaming
                ) || !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Stopped
                        | PlaybackState::Ready
                        | PlaybackState::Buffering
                        | PlaybackState::Paused
                        | PlaybackState::Playing
                ) {
                    return Err(invalid_state(
                        "host playback can enter playing only from an active stopped, buffered, or paused session",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Paused) => {
                if self.snapshot.host_lifecycle != HostLifecycle::Streaming
                    || self.snapshot.playback_state != PlaybackState::Playing
                {
                    return Err(invalid_state(
                        "host playback can pause only while streaming",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Stopped)
            | AudioEvent::EndOfStream { .. } => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready
                        | HostLifecycle::WaitingForListeners
                        | HostLifecycle::Streaming
                        | HostLifecycle::Paused
                ) {
                    return Err(invalid_state(
                        "host playback can stop only during an active host session",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Ready) => {
                if !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Buffering | PlaybackState::Ready
                ) {
                    return Err(invalid_state(
                        "host playback can become ready only after buffering",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Error | PlaybackState::Underrun)
            | AudioEvent::Failed(_)
            | AudioEvent::PositionAdvanced { .. }
            | AudioEvent::TrackBoundaryScheduled { .. }
            | AudioEvent::TrackBoundaryApplied { .. }
            | AudioEvent::SynchronizationUpdated { .. }
            | AudioEvent::Underrun { .. } => {}
        }

        if self.snapshot.host_lifecycle == HostLifecycle::WaitingForListeners
            && matches!(
                &event,
                AudioEvent::PlaybackStateChanged(PlaybackState::Playing)
            )
        {
            let outcome = self.apply_audio(event)?;
            self.snapshot.host_lifecycle = HostLifecycle::Streaming;
            return Ok(outcome);
        }

        self.apply_audio_with_host_lifecycle(event)
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorState, AppRole, AudioEvent, HostLifecycle, PlaybackState};
    use crate::domain::{DeviceId, SessionId};
    use crate::error::{CoreError, CoreErrorCode, ErrorSeverity};
    use crate::runtime::RecoverableAction;

    fn ready_host() -> ActorState {
        let mut state = ActorState::new(DeviceId::new("host-audio-test").expect("valid device ID"));
        state.host_session_id =
            Some(SessionId::new("session-audio-test").expect("valid session ID"));
        state.snapshot.selected_role = Some(AppRole::Host);
        state.snapshot.host_lifecycle = HostLifecycle::Ready;
        state.snapshot.playback_state = PlaybackState::Stopped;
        state
    }

    #[test]
    fn buffered_start_atomically_binds_stream_and_rejects_old_eof_on_replay() {
        use crate::domain::StreamId;

        let mut state = ready_host();
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("buffer");
        let first = StreamId::new("first-generation").expect("stream ID");
        state
            .apply_audio_control(AudioEvent::PositionAdvanced {
                stream_id: first.clone(),
                position_ms: 0,
            })
            .expect("start and bind first stream in one event");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Playing);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Streaming);
        assert_eq!(state.active_audio_stream_id, Some(first.clone()));

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("buffer replay");
        let next = StreamId::new("next-generation").expect("stream ID");
        state
            .apply_audio_control(AudioEvent::PositionAdvanced {
                stream_id: next.clone(),
                position_ms: 0,
            })
            .expect("start and bind replay");
        assert_eq!(state.active_audio_stream_id, Some(next));
        let error = state
            .apply_audio_control(AudioEvent::EndOfStream { stream_id: first })
            .expect_err("old EOF cannot stop replay");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.playback_state, PlaybackState::Playing);
    }

    #[test]
    fn buffered_start_rejects_retired_identity_without_partial_transition() {
        use crate::domain::StreamId;

        let mut state = ready_host();
        let old = StreamId::new("retired-start").expect("stream ID");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("buffer");
        state
            .apply_audio_control(AudioEvent::PositionAdvanced {
                stream_id: old.clone(),
                position_ms: 0,
            })
            .expect("start");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop");
        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("buffer again");
        let error = state
            .apply_audio_control(AudioEvent::PositionAdvanced {
                stream_id: old,
                position_ms: 0,
            })
            .expect_err("retired stream cannot start another generation");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.playback_state, PlaybackState::Buffering);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::WaitingForListeners);
        assert!(state.active_audio_stream_id.is_none());
    }

    #[test]
    fn host_playback_follows_buffer_play_pause_resume_stop_sequence() {
        let mut state = ready_host();

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("buffering is legal from ready");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Buffering);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Ready);

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("playing is legal after buffering");
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Streaming);

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
            .expect("pause is legal while streaming");
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Paused);

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("resume is legal from paused");
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Streaming);

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
            .expect("stop is legal while streaming");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert_eq!(
            state.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
        );
    }

    #[test]
    fn host_can_start_while_waiting_for_listeners() {
        let mut state = ready_host();
        state.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .expect("playing is legal while waiting for listeners");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Playing);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Streaming);
    }

    #[test]
    fn host_pause_is_rejected_before_streaming() {
        let mut state = ready_host();
        let error = state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
            .expect_err("pause must be rejected before streaming");
        assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
        assert_eq!(state.snapshot.playback_state, PlaybackState::Stopped);
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::Ready);
    }

    #[test]
    fn retryable_playlist_failure_keeps_session_ready_and_play_can_retry() {
        let mut state = ready_host();
        state.snapshot.host_lifecycle = HostLifecycle::Streaming;
        state.snapshot.playback_state = PlaybackState::Playing;
        let failure = CoreError::new(
            CoreErrorCode::AudioOutputFailed,
            "playlist decoder failed",
            ErrorSeverity::Error,
            true,
            None,
        )
        .expect("valid playback failure");

        state
            .apply_audio_control(AudioEvent::Failed(failure))
            .expect("retryable audio failure is observable");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Error);
        assert_eq!(
            state.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
        );
        assert_eq!(
            state.snapshot.recoverable_action,
            Some(RecoverableAction::Retry)
        );
        assert!(state.snapshot.last_error.is_some());

        state
            .apply_audio_control(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
            .expect("Play retries from an audio error");
        assert_eq!(state.snapshot.playback_state, PlaybackState::Buffering);
        assert_eq!(
            state.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
        );
        assert_eq!(state.snapshot.recoverable_action, None);
        assert!(state.snapshot.last_error.is_none());
    }
}
