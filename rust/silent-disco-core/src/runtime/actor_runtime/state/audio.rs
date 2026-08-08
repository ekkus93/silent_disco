use super::{
    ActorState, ApplyOutcome, AudioEvent, CoreError, CoreNotification, PlaybackState, invalid_state,
};

impl ActorState {
    pub(super) fn apply_audio(&mut self, event: AudioEvent) -> Result<ApplyOutcome, CoreError> {
        match event {
            AudioEvent::PlaybackStateChanged(state) => {
                // A genuinely new stream starts playing from a state other
                // than Paused or Playing -- resuming a pause must continue
                // from where it left off, not reset, and a duplicate/stale
                // Playing submission while already Playing (e.g. a second
                // Resume command arriving after the first already applied)
                // is a no-op, not a fresh start either. This is the one
                // place a fresh start is distinguishable from a resume or a
                // repeated resume, since all three submit the same
                // `Playing` state.
                if state == PlaybackState::Playing
                    && !matches!(
                        self.snapshot.playback_state,
                        PlaybackState::Paused | PlaybackState::Playing
                    )
                {
                    self.snapshot.playback_position_ms = 0;
                    self.snapshot.stream_ended_naturally = false;
                }
                self.snapshot.playback_state = state;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::PositionAdvanced { position_ms, .. } => {
                if !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Playing | PlaybackState::Paused
                ) {
                    return Err(invalid_state(
                        "playback position advanced while playback was inactive",
                        None,
                    ));
                }
                if position_ms < self.snapshot.playback_position_ms {
                    return Err(invalid_state("playback position moved backward", None));
                }
                self.snapshot.playback_position_ms = position_ms;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::SynchronizationUpdated { device_id, summary } => {
                self.snapshot.synchronization = Some(summary);
                if let Some(listener) = self
                    .snapshot
                    .listeners
                    .iter_mut()
                    .find(|listener| listener.device_id == device_id)
                {
                    listener.synchronization = Some(summary);
                }
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::EndOfStream { .. } => {
                self.snapshot.playback_state = PlaybackState::Stopped;
                self.snapshot.stream_ended_naturally = true;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::Underrun { missing_frames } => {
                self.snapshot.playback_state = PlaybackState::Underrun;
                let mut outcome = self.diagnostic(
                    "audio_underrun",
                    vec![Self::field("missing_frames", &missing_frames.to_string())?],
                )?;
                outcome.changed = true;
                Ok(outcome)
            }
            AudioEvent::Failed(error) => {
                self.snapshot.playback_state = PlaybackState::Error;
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![CoreNotification::Error(error)],
                    changed: true,
                    stop_requested: false,
                })
            }
        }
    }
}
