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

        match &event {
            AudioEvent::PlaybackStateChanged(PlaybackState::Buffering) => {
                if self.snapshot.host_lifecycle != HostLifecycle::Ready
                    || !matches!(
                        self.snapshot.playback_state,
                        PlaybackState::Stopped | PlaybackState::Ready
                    )
                {
                    return Err(invalid_state(
                        "host playback can begin buffering only from a ready stopped session",
                        None,
                    ));
                }
            }
            AudioEvent::PlaybackStateChanged(PlaybackState::Playing) => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready | HostLifecycle::Paused | HostLifecycle::Streaming
                ) || !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Buffering | PlaybackState::Paused | PlaybackState::Playing
                ) {
                    return Err(invalid_state(
                        "host playback can enter playing only after buffering or from paused",
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
            AudioEvent::PlaybackStateChanged(PlaybackState::Error)
            | AudioEvent::Failed(_)
            | AudioEvent::PlaybackStateChanged(PlaybackState::Underrun)
            | AudioEvent::PositionAdvanced { .. }
            | AudioEvent::SynchronizationUpdated { .. }
            | AudioEvent::Underrun { .. } => {}
        }

        self.apply_audio_with_host_lifecycle(event)
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorState, AppRole, AudioEvent, HostLifecycle, PlaybackState};
    use crate::domain::{DeviceId, SessionId};
    use crate::error::CoreErrorCode;

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
        assert_eq!(state.snapshot.host_lifecycle, HostLifecycle::WaitingForListeners);
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
}
