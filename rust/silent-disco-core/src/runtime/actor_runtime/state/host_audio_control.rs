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
            AudioEvent::PlaybackStateChanged(PlaybackState::Stopped) | AudioEvent::EndOfStream { .. } => {
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
