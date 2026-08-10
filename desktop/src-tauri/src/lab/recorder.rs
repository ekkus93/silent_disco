//! Bounded, ordered capture of one Lab node's [`CoreNotification`] stream
//! (Block 40.2/40.3), reused by both the scenario runner (`super::scenario`)
//! and replay's version-compatibility check (`super::replay`).
//!
//! Spec section 29.5 requires a scenario recording to capture "effects;
//! ...; errors; ...; assertion results" as the scenario runs. The actor
//! architecture (`silent_disco_core::runtime::CoreActorRuntime`) already has
//! exactly one channel that carries all of that: the [`CoreObserver`] the
//! runtime is started with. Every notification -- including a rejected
//! command's `CoreNotification::Error`, which is otherwise **invisible**
//! (`ActorState::process` leaves `CoreSnapshot` completely unchanged on
//! rejection; see the module doc comment on `super::scenario` for why this
//! matters for the scenario runner's own step-settlement wait) -- passes
//! through here once and only once.
//!
//! Kept deliberately narrow for Block 40: full payloads are not retained
//! (effects/diagnostics are recorded by stable kind name plus their own
//! already-bounded fields, matching the exact `platform_effect_name`-style
//! convention `crate::runtime_dto` already uses for the same enums, not a
//! parallel one). Protocol/core version stamping, payload hashes, and a
//! persisted on-disk recording format are Block 41's explicit extension
//! (`docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`'s own Block 41 section),
//! not duplicated here.

use serde::{Deserialize, Serialize};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot, PlatformEffectRequest,
    StorageEffectRequest, TransportEffectRequest,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Hard cap on retained notifications per node (Block 40.1 "bound ...
/// duration" applied to recording, not only to the scenario file itself;
/// CLAUDE.md "High-frequency packet/frame telemetry must not be written
/// directly to `SQLite`" -- this never touches `SQLite` at all, but the
/// same "never unbounded" discipline applies to any in-memory trace too). Once
/// reached, further notifications are counted as dropped rather than
/// silently discarded without a trace -- see [`ScenarioRecorder::dropped_count`].
pub(crate) const MAX_RECORDED_NOTIFICATIONS: usize = 4_096;

/// One captured fact about a node's actor, in the exact order the actor's
/// own notification worker delivered it (single-threaded per node, so this
/// order is already deterministic -- see `CoreActorRuntime`'s notification
/// worker, which processes one `NotificationMessage` at a time).
///
/// `Serialize`/`Deserialize` (Block 41.1 "commands; events; effects;
/// snapshot revisions and safe hashes/full bounded snapshots; ... errors")
/// make this the literal on-disk representation a
/// [`super::recording::ScenarioRecording`] persists -- no separate mirror
/// DTO. Every field is already an owned, bounded, non-secret value (see
/// [`SnapshotSummary`]'s own doc comment for the one field this module
/// deliberately never captures), so there is nothing left to redact at the
/// serialization boundary itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordedNotification {
    /// Monotonically increasing within one node's recorder; never reused.
    pub(crate) sequence: u64,
    pub(crate) kind: RecordedNotificationKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum RecordedNotificationKind {
    Snapshot {
        revision: u64,
        /// Redacted, bounded projection of the snapshot that produced this
        /// notification (Block 41.1 "safe hashes/full bounded snapshots" --
        /// a full bounded, pre-redacted snapshot chosen over an opaque hash
        /// so a saved recording is genuinely useful to a human reading it,
        /// see [`SnapshotSummary`]'s own doc comment).
        summary: SnapshotSummary,
    },
    Effect {
        name: String,
    },
    TransportEffect {
        name: String,
    },
    StorageEffect {
        name: String,
    },
    Error {
        code: String,
        subsystem: String,
        severity: String,
        message: String,
    },
    Diagnostic {
        name: String,
        fields: Vec<(String, String)>,
    },
}

/// A redacted, bounded, non-secret projection of [`CoreSnapshot`] captured
/// at every [`CoreNotification::Snapshot`] (Block 41.1's "snapshot
/// revisions and safe hashes/full bounded snapshots", and 41.3's "secret
/// redaction").
///
/// **This is the one and only boundary where a full [`CoreSnapshot`] is
/// ever read for recording purposes** -- every field below is copied out
/// individually and deliberately. Fields of `CoreSnapshot` that are
/// intentionally **never** copied here, because they can carry
/// human-authored or session-identifying text rather than bounded
/// domain enums/counts/numbers:
///
/// - `host_draft.invite_code` (`Option<String>`) -- the plaintext admission
///   secret a host session may be gated behind. A saved recording is
///   explicitly meant to be attachable to a bug report and shared outside
///   the machine that produced it (spec 29.5 / Block 41's own acceptance:
///   "a difficult failure can be saved and replayed"), so this must never
///   appear in it. Only `host_draft.approval_mode` (an enum, not a secret)
///   is captured.
/// - `host_draft.session_name`, `listeners[].display_name`,
///   `pending_join_requests[].display_name`, `discovered_sessions[].session_name`
///   -- free-form human-authored text. Only bounded counts/enums derived
///   from these collections are captured (e.g. `listener_count`), never the
///   collections themselves.
/// - `listeners[].device_id`, `pending_join_requests[].device_id` -- while a
///   Lab node's identity is always synthetic (never keyring-backed, see
///   `super::synthetic_identity`), device identifiers are still excluded on
///   principle: this module has no way to know at capture time whether a
///   future scenario extension might reference a real device identity, and
///   the redaction boundary is deliberately conservative rather than
///   depending on that staying true forever.
/// - `host_draft.audio_source`, `discovered_sessions[].endpoint`,
///   `last_error` (already captured separately and in full via
///   [`RecordedNotificationKind::Error`], not duplicated here) -- out of
///   scope for a snapshot summary.
///
/// Tested directly by
/// `tests::snapshot_summary_never_carries_the_raw_invite_code`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotSummary {
    pub(crate) selected_role: Option<String>,
    pub(crate) host_lifecycle: String,
    pub(crate) listener_lifecycle: String,
    pub(crate) playback_state: String,
    pub(crate) discovery_active: bool,
    pub(crate) listener_count: u32,
    pub(crate) synchronization: Option<SnapshotSynchronization>,
    pub(crate) last_delivery_severity: Option<String>,
    pub(crate) shutting_down: bool,
    /// The host draft's admission policy -- an enum, never the
    /// `invite_code` it may be paired with. See this struct's own doc
    /// comment.
    pub(crate) host_approval_mode: String,
    pub(crate) capabilities: SnapshotCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotSynchronization {
    pub(crate) confidence: String,
    pub(crate) offset_ms: f64,
    pub(crate) round_trip_ms: f64,
    pub(crate) drift_ppm: f64,
}

/// Mirrors `CapabilitySnapshot`'s own bitmap shape (`crate::runtime_dto::CapabilitySnapshotDto`
/// carries the identical, already-precedented `#[allow]` for the same reason:
/// these are six independent, stable, named IPC/recording capability flags,
/// not state-machine-shaped booleans a `clippy::struct_excessive_bools` refactor
/// would meaningfully simplify).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct SnapshotCapabilities {
    pub(crate) nearby_discovery_available: bool,
    pub(crate) nearby_advertising_available: bool,
    pub(crate) local_network_available: bool,
    pub(crate) audio_source_selection_available: bool,
    pub(crate) audio_output_available: bool,
    pub(crate) secure_store_available: bool,
}

impl SnapshotSummary {
    fn capture(snapshot: &CoreSnapshot) -> Self {
        Self {
            selected_role: snapshot
                .selected_role
                .map(|role| role.wire_name().to_owned()),
            host_lifecycle: snapshot.host_lifecycle.wire_name().to_owned(),
            listener_lifecycle: snapshot.listener_lifecycle.wire_name().to_owned(),
            playback_state: snapshot.playback_state.wire_name().to_owned(),
            discovery_active: snapshot.discovery_active,
            listener_count: u32::try_from(snapshot.listeners.len()).unwrap_or(u32::MAX),
            synchronization: snapshot
                .synchronization
                .map(|summary| SnapshotSynchronization {
                    confidence: summary.confidence.wire_name().to_owned(),
                    offset_ms: summary.offset_ms,
                    round_trip_ms: summary.round_trip_ms,
                    drift_ppm: summary.drift_ppm,
                }),
            last_delivery_severity: snapshot
                .last_delivery
                .map(|report| report.severity.wire_name().to_owned()),
            shutting_down: snapshot.shutting_down,
            host_approval_mode: snapshot.host_draft.approval_mode.wire_name().to_owned(),
            capabilities: SnapshotCapabilities {
                nearby_discovery_available: snapshot.capabilities.nearby_discovery_available,
                nearby_advertising_available: snapshot.capabilities.nearby_advertising_available,
                local_network_available: snapshot.capabilities.local_network_available,
                audio_source_selection_available: snapshot
                    .capabilities
                    .audio_source_selection_available,
                audio_output_available: snapshot.capabilities.audio_output_available,
                secure_store_available: snapshot.capabilities.secure_store_available,
            },
        }
    }
}

impl RecordedNotificationKind {
    /// `true` for a [`CoreNotification::Error`] whose severity is Fatal
    /// (Block 40.3 "no unexpected fatal error").
    pub(crate) fn is_fatal_error(&self) -> bool {
        matches!(self, Self::Error { severity, .. } if severity == "fatal")
    }
}

struct RecorderState {
    entries: Vec<RecordedNotification>,
    next_sequence: u64,
    /// Count of notifications that arrived after [`MAX_RECORDED_NOTIFICATIONS`]
    /// was already reached -- reported, never silently absorbed.
    dropped: u64,
}

/// A shared, thread-safe recorder attached to exactly one Lab node as its
/// [`CoreObserver`] (through [`RecordingObserver`]). Cheap to poll from the
/// scenario runner's own thread while the node's actor and notification
/// worker threads keep writing to it concurrently.
pub(crate) struct ScenarioRecorder {
    state: Mutex<RecorderState>,
    /// Signaled every time a new notification is recorded, so the runner
    /// can block on real progress instead of busy-polling
    /// (`Condvar::wait_timeout` -- like `Receiver::recv_timeout`, already
    /// used throughout this crate's own test helpers, e.g.
    /// `platform/start_playback_tests.rs`'s `wait_snapshot_for` -- reads the
    /// OS clock internally, but neither call site nor this module ever
    /// spells `Instant::now()`/`SystemTime::now()` itself, preserving Block
    /// 38.3's grep-verified "no wall-clock sleep required for deterministic
    /// scenarios" evidence for actual *virtual scenario time*. This is a
    /// different, narrower concern: a bounded real-time safety valve against
    /// a genuinely stuck background actor thread, not a way to make virtual
    /// time pass -- virtual time only ever moves through
    /// `LabRuntime::advance`.
    progress: Condvar,
}

impl ScenarioRecorder {
    #[must_use]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RecorderState {
                entries: Vec::new(),
                next_sequence: 0,
                dropped: 0,
            }),
            progress: Condvar::new(),
        })
    }

    fn record(&self, notification: &CoreNotification) {
        let kind = classify(notification);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        if state.entries.len() >= MAX_RECORDED_NOTIFICATIONS {
            state.dropped = state.dropped.saturating_add(1);
        } else {
            state.entries.push(RecordedNotification { sequence, kind });
        }
        self.progress.notify_all();
    }

    /// Returns the sequence number the *next* recorded notification will
    /// receive -- a scenario runner captures this immediately before
    /// submitting an input, then later checks whether it has changed.
    #[must_use]
    pub(crate) fn next_sequence(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_sequence
    }

    /// Blocks (bounded) until at least one new notification has been
    /// recorded since `since_sequence`, or `timeout` elapses. Returns
    /// `true` on real progress, `false` on timeout -- never panics or
    /// spins the caller's own clock.
    pub(crate) fn wait_for_progress(&self, since_sequence: u64, timeout: Duration) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.next_sequence > since_sequence {
            return true;
        }
        let (state, result) = self
            .progress
            .wait_timeout_while(state, timeout, |state| {
                state.next_sequence <= since_sequence
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !result.timed_out() && state.next_sequence > since_sequence
    }

    /// A bounded, ordered snapshot of everything recorded so far.
    #[must_use]
    pub(crate) fn entries(&self) -> Vec<RecordedNotification> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .clone()
    }

    /// Notifications dropped because [`MAX_RECORDED_NOTIFICATIONS`] was
    /// already reached -- `0` for every scenario bounded enough to fit.
    #[must_use]
    pub(crate) fn dropped_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dropped
    }
}

/// Adapts a shared [`ScenarioRecorder`] to the [`CoreObserver`] contract a
/// Lab node's actor is started with. Kept separate from `ScenarioRecorder`
/// itself so the recorder's own inspection methods (`entries`,
/// `wait_for_progress`, ...) are usable from the owning thread without going
/// through `CoreObserver`'s trait object surface.
pub(crate) struct RecordingObserver(pub(crate) Arc<ScenarioRecorder>);

impl CoreObserver for RecordingObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        self.0.record(&notification);
        Ok(())
    }
}

fn classify(notification: &CoreNotification) -> RecordedNotificationKind {
    match notification {
        CoreNotification::Snapshot(snapshot) => RecordedNotificationKind::Snapshot {
            revision: snapshot.revision.get(),
            summary: SnapshotSummary::capture(snapshot),
        },
        CoreNotification::Effect(effect) => RecordedNotificationKind::Effect {
            name: platform_effect_name(&effect.request).to_owned(),
        },
        CoreNotification::TransportEffect(effect) => RecordedNotificationKind::TransportEffect {
            name: transport_effect_name(&effect.request).to_owned(),
        },
        CoreNotification::StorageEffect(effect) => RecordedNotificationKind::StorageEffect {
            name: storage_effect_name(&effect.request).to_owned(),
        },
        CoreNotification::Error(error) => RecordedNotificationKind::Error {
            code: error.code.stable_name().to_owned(),
            subsystem: error.subsystem.stable_name().to_owned(),
            severity: error.severity.stable_name().to_owned(),
            message: error.message.clone(),
        },
        CoreNotification::Diagnostic(diagnostic) => RecordedNotificationKind::Diagnostic {
            name: diagnostic.name.clone(),
            fields: diagnostic_fields(diagnostic),
        },
    }
}

fn diagnostic_fields(diagnostic: &CoreDiagnostic) -> Vec<(String, String)> {
    diagnostic
        .fields
        .iter()
        .map(|field| (field.key.clone(), field.value.clone()))
        .collect()
}

/// Mirrors `crate::runtime_dto`'s private effect-name matchers (not reused
/// directly -- that module's are private to it, and duplicating three short,
/// stable match arms is clearer than threading a new `pub(crate)` export
/// across an unrelated module boundary for this).
fn platform_effect_name(request: &PlatformEffectRequest) -> &'static str {
    match request {
        PlatformEffectRequest::RequestCapabilities(_) => "request_capabilities",
        PlatformEffectRequest::StartAdvertising(_) => "start_advertising",
        PlatformEffectRequest::StopAdvertising => "stop_advertising",
        PlatformEffectRequest::StartDiscovery(_) => "start_discovery",
        PlatformEffectRequest::StopDiscovery => "stop_discovery",
        PlatformEffectRequest::EstablishNetwork(_) => "establish_network",
        PlatformEffectRequest::ReleaseNetwork => "release_network",
        PlatformEffectRequest::PrepareAudioSource(_) => "prepare_audio_source",
        PlatformEffectRequest::StartAudioOutput(_) => "start_audio_output",
        PlatformEffectRequest::StopAudioOutput => "stop_audio_output",
        PlatformEffectRequest::ShareDiagnostics { .. } => "share_diagnostics",
    }
}

fn transport_effect_name(request: &TransportEffectRequest) -> &'static str {
    match request {
        TransportEffectRequest::DeliverJoinApproval { .. } => "deliver_join_approval",
        TransportEffectRequest::DeliverJoinRejection { .. } => "deliver_join_rejection",
        TransportEffectRequest::DisconnectListener { .. } => "disconnect_listener",
    }
}

fn storage_effect_name(request: &StorageEffectRequest) -> &'static str {
    match request {
        StorageEffectRequest::PersistSettings { .. } => "persist_settings",
        StorageEffectRequest::PersistTrustedDevice { .. } => "persist_trusted_device",
    }
}

#[cfg(test)]
mod tests;
