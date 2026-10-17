use super::{
    ActorState, ApplyOutcome, AudioEvent, CoreError, CoreNotification, HostQueueBoundary,
    PlaybackState, invalid_state,
};

impl ActorState {
    pub(super) fn apply_audio(&mut self, event: AudioEvent) -> Result<ApplyOutcome, CoreError> {
        match event {
            AudioEvent::PlaybackStateChanged(state) => {
                if state == PlaybackState::Buffering {
                    self.clear_failure();
                }
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
                    self.active_audio_stream_id = None;
                    self.snapshot.stream_ended_naturally = false;
                    self.snapshot.host_queue_next_boundary = None;
                }
                if matches!(state, PlaybackState::Stopped | PlaybackState::Error) {
                    self.snapshot.host_queue_next_boundary = None;
                    self.active_audio_stream_id = None;
                }
                if state == PlaybackState::Stopped {
                    self.snapshot.playback_position_ms = 0;
                }
                self.snapshot.playback_state = state;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::PositionAdvanced {
                stream_id,
                position_ms,
            } => {
                if !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Playing | PlaybackState::Paused
                ) {
                    return Err(invalid_state(
                        "playback position advanced while playback was inactive",
                        None,
                    ));
                }
                // Position events are already stream-scoped on the wire.
                // Reject late progress from a different stream once this
                // playback generation has reported its first position.
                if self
                    .active_audio_stream_id
                    .as_ref()
                    .is_some_and(|active| active != &stream_id)
                {
                    return Err(invalid_state(
                        "playback position belongs to a different stream",
                        None,
                    ));
                }
                if position_ms < self.snapshot.playback_position_ms {
                    return Err(invalid_state("playback position moved backward", None));
                }
                self.active_audio_stream_id = Some(stream_id);
                self.snapshot.playback_position_ms = position_ms;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::TrackBoundaryApplied {
                entry_id,
                first_sample_index,
            } => {
                if self.snapshot.playback_state != PlaybackState::Playing {
                    return Err(invalid_state(
                        "track boundary applied while playback was inactive",
                        None,
                    ));
                }
                let expected = self.snapshot.host_queue.upcoming().first().ok_or_else(|| {
                    invalid_state("track boundary has no matching queued entry", None)
                })?;
                if expected.entry_id != entry_id {
                    return Err(invalid_state(
                        "track boundary does not match the next queued entry",
                        None,
                    ));
                }
                // The transport must first announce and reserve the exact
                // boundary. Without this check a stale event can advance the
                // authoritative queue after a legal edit, Stop, or replay
                // even if the next entry ID happens to match.
                let Some(boundary) = &self.snapshot.host_queue_next_boundary else {
                    return Err(invalid_state(
                        "track boundary was applied without being scheduled",
                        None,
                    ));
                };
                if boundary.entry_id != entry_id
                    || boundary.first_sample_index != first_sample_index
                {
                    return Err(invalid_state(
                        "applied track boundary differs from the announced queue boundary",
                        None,
                    ));
                }
                let revision = self.snapshot.host_queue.revision();
                self.snapshot
                    .host_queue
                    .complete_current(revision)
                    .map_err(|error| invalid_state(error.to_string(), None))?;
                self.snapshot.host_draft.audio_source = self
                    .snapshot
                    .host_queue
                    .current()
                    .map(|entry| entry.source.clone());
                self.snapshot.host_queue_next_boundary = None;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::TrackBoundaryScheduled {
                entry_id,
                first_sample_index,
            } => {
                if self.snapshot.playback_state != PlaybackState::Playing {
                    return Err(invalid_state(
                        "track boundary scheduled while playback was inactive",
                        None,
                    ));
                }
                let expected = self.snapshot.host_queue.upcoming().first().ok_or_else(|| {
                    invalid_state("track boundary has no matching queued entry", None)
                })?;
                if expected.entry_id != entry_id {
                    return Err(invalid_state(
                        "track boundary does not match the next queued entry",
                        None,
                    ));
                }
                if let Some(boundary) = &self.snapshot.host_queue_next_boundary
                    && (boundary.entry_id != entry_id
                        || boundary.first_sample_index != first_sample_index)
                {
                    return Err(invalid_state(
                        "a different queue boundary is already scheduled",
                        None,
                    ));
                }
                self.snapshot.host_queue_next_boundary = Some(HostQueueBoundary {
                    entry_id,
                    first_sample_index,
                });
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
            AudioEvent::EndOfStream { stream_id } => {
                // An old producer can finish after a new stream has begun.
                // Once the active generation reports position, its stream
                // identity fences both stale progress and stale natural EOF.
                if self
                    .active_audio_stream_id
                    .as_ref()
                    .is_some_and(|active| active != &stream_id)
                {
                    return Err(invalid_state(
                        "end of stream belongs to a different playback stream",
                        None,
                    ));
                }
                // A delayed EOF after explicit Stop or a playback failure is
                // not natural completion of the active stream. Keep the queue
                // intact for replay or retry instead of clearing it.
                if self.snapshot.playback_state != PlaybackState::Playing {
                    return Err(invalid_state(
                        "end of stream received while playback was inactive",
                        None,
                    ));
                }
                // An EOF with upcoming entries is not a completed playlist.
                // It can occur if the producer finished just as the host
                // appended a song, or if a pending boundary was not applied.
                // Stop the exhausted stream but keep the authoritative queue
                // and source alias available for an explicit replay.
                if !self.snapshot.host_queue.upcoming().is_empty()
                    || self.snapshot.host_queue_next_boundary.is_some()
                {
                    self.snapshot.playback_state = PlaybackState::Stopped;
                    self.snapshot.playback_position_ms = 0;
                    self.active_audio_stream_id = None;
                    self.snapshot.host_queue_next_boundary = None;
                    self.snapshot.stream_ended_naturally = false;
                    return Ok(ApplyOutcome::changed());
                }
                let queue_revision = self.snapshot.host_queue.revision();
                self.snapshot
                    .host_queue
                    .clear(queue_revision, false)
                    .map_err(|error| invalid_state(error.to_string(), None))?;
                self.snapshot.host_draft.audio_source = None;
                self.snapshot.host_queue_next_boundary = None;
                self.active_audio_stream_id = None;
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
                self.active_audio_stream_id = None;
                self.snapshot.playback_state = PlaybackState::Error;
                self.snapshot.host_queue_next_boundary = None;
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
