use super::command::CoreCommandRequest;
use super::errors::{
    RuntimeContractError, validate_unique_listeners, validate_unique_requests,
    validate_unique_sessions,
};
use super::events::{AudioEvent, StorageEvent, TransportEvent};
use super::platform::PlatformEvent;
use super::{MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS, MAX_PENDING_JOIN_REQUESTS};
use crate::domain::{
    AppRole, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, SessionId,
    TransportState, TuningSettings,
};
use crate::error::CoreError;
use crate::runtime::{
    CapabilitySnapshot, DeliveryReport, HostDraft, HostQueue, JoinRequestSummary, ListenerSummary,
    QueueEntry, SessionAdvertisement, SnapshotRevision, SynchronizationSummary,
};

/// Presentation action that remains available after a recoverable failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverableAction {
    Retry,
    Rescan,
    Reconnect,
    Resynchronize,
    ReselectAudioSource,
}

/// Upcoming queue transition announced to the playback transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostQueueBoundary {
    pub entry_id: String,
    pub first_sample_index: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostQueuePreparationState {
    Empty,
    Prepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostQueueReadiness {
    Empty,
    Ready,
    Buffering,
    Playing,
    Paused,
    Underrun,
    Failed,
}

/// Bounded display-oriented queue projection derived from the actor snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostQueueSnapshot {
    pub revision: u64,
    pub current: Option<QueueEntry>,
    pub upcoming: Vec<QueueEntry>,
    pub preparation_state: HostQueuePreparationState,
    pub readiness: HostQueueReadiness,
    pub error: Option<CoreError>,
    pub next_boundary: Option<HostQueueBoundary>,
}

/// Immutable authoritative presentation state.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSnapshot {
    pub revision: SnapshotRevision,
    pub selected_role: Option<AppRole>,
    pub capabilities: CapabilitySnapshot,
    pub host_draft: HostDraft,
    pub host_queue: HostQueue,
    /// Next sample-indexed queue transition announced by the playback worker.
    pub host_queue_next_boundary: Option<HostQueueBoundary>,
    pub host_lifecycle: HostLifecycle,
    pub listener_lifecycle: ListenerLifecycle,
    pub transport_state: TransportState,
    pub discovery_active: bool,
    pub discovered_sessions: Vec<SessionAdvertisement>,
    pub selected_session: Option<SessionId>,
    pub pending_join_requests: Vec<JoinRequestSummary>,
    pub listeners: Vec<ListenerSummary>,
    pub playback_state: PlaybackState,
    pub playback_position_ms: u64,
    /// True from the moment the current/most recent stream reached
    /// [`AudioEvent::EndOfStream`] until the next stream genuinely starts.
    /// Explicit stops never set this: it exists so a listener-facing UI can
    /// show "the source finished" rather than folding that into the same
    /// generic `Stopped` status a manual stop produces.
    pub stream_ended_naturally: bool,
    pub synchronization: Option<SynchronizationSummary>,
    pub tuning: TuningSettings,
    pub last_delivery: Option<DeliveryReport>,
    /// Operation that produced `last_delivery`, when the report came from a
    /// correlated transport effect rather than an unsolicited event.
    pub last_delivery_operation_id: Option<OperationId>,
    pub recoverable_action: Option<RecoverableAction>,
    pub last_error: Option<CoreError>,
    pub shutting_down: bool,
}

impl Default for CoreSnapshot {
    fn default() -> Self {
        Self {
            revision: SnapshotRevision::default(),
            selected_role: None,
            capabilities: CapabilitySnapshot::default(),
            host_draft: HostDraft::default(),
            host_queue: HostQueue::default(),
            host_queue_next_boundary: None,
            host_lifecycle: HostLifecycle::Idle,
            listener_lifecycle: ListenerLifecycle::Idle,
            transport_state: TransportState::Idle,
            discovery_active: false,
            discovered_sessions: Vec::new(),
            selected_session: None,
            pending_join_requests: Vec::new(),
            listeners: Vec::new(),
            playback_state: PlaybackState::Stopped,
            playback_position_ms: 0,
            stream_ended_naturally: false,
            synchronization: None,
            tuning: TuningSettings::default(),
            last_delivery: None,
            last_delivery_operation_id: None,
            recoverable_action: None,
            last_error: None,
            shutting_down: false,
        }
    }
}

impl CoreSnapshot {
    /// Projects current/upcoming entries with actor-owned playback readiness.
    #[must_use]
    pub fn host_queue_snapshot(&self) -> HostQueueSnapshot {
        let preparation_state = if self.host_queue.entries().is_empty() {
            HostQueuePreparationState::Empty
        } else {
            // Queue commands only accept sources after platform preparation succeeds.
            HostQueuePreparationState::Prepared
        };
        let readiness = match (self.playback_state, self.host_queue.entries().is_empty()) {
            (PlaybackState::Error, _) => HostQueueReadiness::Failed,
            (_, true) => HostQueueReadiness::Empty,
            (PlaybackState::Buffering, false) => HostQueueReadiness::Buffering,
            (PlaybackState::Playing, false) => HostQueueReadiness::Playing,
            (PlaybackState::Paused, false) => HostQueueReadiness::Paused,
            (PlaybackState::Underrun, false) => HostQueueReadiness::Underrun,
            (PlaybackState::Stopped | PlaybackState::Ready, false) => HostQueueReadiness::Ready,
        };
        HostQueueSnapshot {
            revision: self.host_queue.revision(),
            current: self.host_queue.current().cloned(),
            upcoming: self.host_queue.upcoming().to_vec(),
            preparation_state,
            readiness,
            error: (self.playback_state == PlaybackState::Error)
                .then(|| self.last_error.clone())
                .flatten(),
            next_boundary: self.host_queue_next_boundary.clone(),
        }
    }

    /// Validates bounded collections and cross-field state invariants.
    ///
    /// # Errors
    ///
    /// Rejects excessive collections, duplicate identifiers, discovery flag/state
    /// disagreement, or an unknown selected session.
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        if self.discovered_sessions.len() > MAX_DISCOVERED_SESSIONS {
            return Err(RuntimeContractError::DiscoveredSessionLimit);
        }
        if self.pending_join_requests.len() > MAX_PENDING_JOIN_REQUESTS {
            return Err(RuntimeContractError::PendingRequestLimit);
        }
        if self.listeners.len() > MAX_CONNECTED_LISTENERS {
            return Err(RuntimeContractError::ListenerLimit);
        }
        validate_unique_sessions(&self.discovered_sessions)?;
        validate_unique_requests(&self.pending_join_requests)?;
        validate_unique_listeners(&self.listeners)?;
        if self.discovery_active != (self.transport_state == TransportState::Discovering) {
            return Err(RuntimeContractError::DiscoveryStateMismatch);
        }
        if let Some(selected_session) = &self.selected_session
            && !self
                .discovered_sessions
                .iter()
                .any(|advertisement| &advertisement.session_id == selected_session)
        {
            return Err(RuntimeContractError::UnknownSelectedSession);
        }
        Ok(())
    }
}

/// Source-ordered input accepted by the serialized actor.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreActorInput {
    Command {
        operation_id: OperationId,
        request: CoreCommandRequest,
    },
    Platform(PlatformEvent),
    Transport(TransportEvent),
    Audio(AudioEvent),
    Storage(StorageEvent),
}

impl CoreActorInput {
    /// Returns the correlation ID when the input is command- or operation-derived.
    #[must_use]
    pub const fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::Command { operation_id, .. } => Some(operation_id),
            Self::Platform(event) => event.operation_id(),
            Self::Storage(event) => Some(event.operation_id()),
            Self::Transport(TransportEvent::DeliveryCompleted { operation_id, .. }) => {
                Some(operation_id)
            }
            Self::Transport(
                TransportEvent::StateChanged(_)
                | TransportEvent::JoinRequested(_)
                | TransportEvent::ListenerConnected(_)
                | TransportEvent::ListenerDisconnected { .. }
                | TransportEvent::SessionEnded { .. }
                | TransportEvent::Failed(_)
                | TransportEvent::AwaitingApproval
                | TransportEvent::JoinApproved { .. }
                | TransportEvent::JoinRejected { .. },
            )
            | Self::Audio(_) => None,
        }
    }
}
