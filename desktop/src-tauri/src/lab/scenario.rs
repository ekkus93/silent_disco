//! Scenario schema, runner, and assertions (Block 40; spec section 29.4).
//!
//! ## 40.1 Format
//!
//! JSON via `serde`/`serde_json` -- both already pinned exact-version
//! dependencies of `desktop/src-tauri` and `rust/silent-disco-core` (see
//! their `Cargo.toml`s) and the format every other IPC/DTO boundary in this
//! crate already uses (`crate::runtime_dto`, `crate::dto`, ...). Adding a
//! YAML dependency for one more file format, with no existing YAML use
//! anywhere in this workspace, would be a new dependency for no concrete
//! benefit -- CLAUDE.md's dependency-selection discipline favors the
//! already-pinned, already-audited choice.
//!
//! A scenario file is untrusted input: a human or another tool can hand-edit
//! it. Every collection below has an explicit bound (Block 40.1 "bound
//! nodes, links, steps, strings, and duration"), schema version is checked
//! before the rest of the document is even structurally parsed (`parse`),
//! and every enum that names a command or assertion kind is a closed Rust
//! enum decoded through `serde`'s internally-tagged representation --
//! an unrecognized `"kind"` string is a hard deserialization error, never a
//! silently ignored or defaulted step (Block 40.1 "reject unknown commands
//! and assertions"). Nothing here parses, evaluates, or loads anything
//! resembling a script or expression language -- every field is a plain,
//! bounded, statically typed value (Block 40.1 "no arbitrary code
//! execution").
//!
//! ## Deliberate scope boundaries (read before extending)
//!
//! - **`links` are captured, bounded, and validated, but not yet wired into
//!   any node's live transport.** `LabRuntime` (`super::mod`) has never
//!   connected a Lab node to `silent_disco_core::transport`'s virtual
//!   transport/fault-injection stack (`super::fault`, the core's own
//!   `virtual_transport`/`virtual_fault`) -- `lab/mod.rs`'s own doc comment
//!   has said so since Block 37 ("virtual transport/fault-injection wiring
//!   ... is a later Lab Mode block's concern"), and that is still true after
//!   Block 39 built the fault model standalone. Wiring `links` into live
//!   per-node transport, and adding scenario steps that change a link's
//!   fault configuration mid-run, is exactly that later block's work, not
//!   Block 40's. A scenario can still declare and bound them today so that
//!   block can consume an already-stable schema.
//! - **Commands are a deliberately curated, real subset of
//!   [`silent_disco_core::runtime::CoreCommand`]**, not every variant.
//!   Excluded: `UpdateHostDraft`/`UpdateTuning` (nested patch types with
//!   their own large optional-field surface -- a reasonable follow-up, not
//!   essential to prove the schema/runner/assertion machinery is real) and
//!   `SelectSession` (meaningless without live discovery, which needs the
//!   transport wiring above). Every included variant is submitted through
//!   the exact real `CoreActorHandle::submit_command` production entry
//!   point -- never a synthesized shortcut.
//! - **`InjectUnderrun`/`InjectSynchronizationUpdated`/`InjectDeliveryCompleted`
//!   submit real [`AudioEvent`]/[`TransportEvent`] values** through the same
//!   `CoreActorHandle::submit_audio_event`/`submit_transport_event` entry
//!   points a real platform adapter uses. This is not a "high-level success"
//!   bypass in the sense spec section 29.3 forbids for the *transport wire*
//!   boundary (these events never claim to have crossed a wire at all); it
//!   is the only way to exercise sync/delivery/underrun assertions
//!   (Block 40.3) before the transport wiring above exists. Each one is
//!   still validated exactly like production input and can genuinely fail
//!   (e.g. `InjectDeliveryCompleted` against no pending transport operation
//!   deterministically produces a real
//!   `stale or duplicate transport delivery completion` error) --
//!   documented per-variant below.
//! - **Audio fixtures never carry a filesystem path.** A
//!   [`ScenarioFixture`] is purely descriptive metadata (id, display name,
//!   optional byte length/duration) used to build a validated
//!   `AudioSourceDescriptor`. "Source fixture references restricted to Lab
//!   assets" (Block 40.2) is satisfied by construction: since the schema has
//!   no path field at all, a scenario cannot reference anything on the real
//!   filesystem, Lab or otherwise. No Lab node has a wired audio pipeline
//!   yet (`super::mod`'s `LabNodeHandle` has no audio output), so there is
//!   nothing today a real path would resolve against regardless.
//!
//! ## Step settlement (why this module never spells `Instant::now()`)
//!
//! `CoreActorHandle::submit_command`'s own doc comment says it plainly: "The
//! receipt does not prove command completion." Processing happens
//! asynchronously on the actor's own worker thread. Worse, a **rejected**
//! command is invisible in `CoreSnapshot` entirely --
//! `ActorState::process` only assigns the mutated candidate state back
//! (`*self = candidate`) on the `Ok` branch; on `Err` the snapshot is left
//! completely unchanged and the only trace is a `CoreNotification::Error`
//! delivered to the observer. So the runner cannot just poll
//! `current_snapshot()` after submitting -- it also needs to observe that
//! observer stream. [`super::recorder::ScenarioRecorder`] gives it a
//! `Condvar`-based bounded wait for that (`wait_for_progress`), the same
//! shape as this crate's own established `Receiver::recv_timeout` test
//! helpers (e.g. `platform/start_playback_tests.rs`'s `wait_snapshot_for`),
//! promoted to production Lab code. This is a *bounded real-time safety
//! valve* against a stuck background thread, not a way to make *virtual
//! scenario time* pass -- virtual time only ever moves through
//! `LabRuntime::advance`, and this module never reads the wall clock
//! directly (Block 38.3's grep-verified evidence for `desktop/src-tauri/src/lab/`
//! stays true for this file).

use super::clock::LabClockError;
use super::recorder::{RecordedNotificationKind, RecordingObserver, ScenarioRecorder};
use super::{LabNodeId, LabRuntime, MAX_LAB_NODES};
use crate::dto::DesktopErrorDto;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use silent_disco_core::domain::{
    AppRole, DeliverySeverity, DeviceId, EnumDecodeError, HostLifecycle, ListenerLifecycle,
    OperationId, PlaybackState, RequestId, SyncConfidence,
};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, CoreCommand, CoreCommandRequest, CoreSnapshot,
    DeliveryReport, PermissionCapability, RuntimeContractError, RuntimeRecordValidationError,
    SnapshotRevision, SynchronizationSummary, TransportEvent,
};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

/// The only schema version this runner understands (Block 40.1 "version
/// schema" / "reject unknown versions").
pub(crate) const SCHEMA_VERSION: u32 = 1;

pub(crate) const MAX_NODES: usize = MAX_LAB_NODES;
pub(crate) const MAX_LINKS: usize = 64;
pub(crate) const MAX_FIXTURES: usize = 32;
pub(crate) const MAX_STEPS: usize = 256;
pub(crate) const MAX_ASSERTIONS: usize = 128;
pub(crate) const MAX_ID_BYTES: usize = 64;
pub(crate) const MAX_DISPLAY_NAME_BYTES: usize = 256;
pub(crate) const MAX_TOKEN_BYTES: usize = 256;
pub(crate) const MAX_ERROR_CODE_BYTES: usize = 128;
/// Simulated-time bound shared by `timeoutMs`, every step's `atMs`, and
/// every assertion's `byMs` (Block 40.1 "bound ... duration"): 24 simulated
/// hours is generous for any realistic Lab scenario while still rejecting a
/// pathological/malformed value outright.
pub(crate) const MAX_SCENARIO_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
pub(crate) const MAX_LINK_LATENCY_MS: u64 = 60_000;
pub(crate) const MAX_LINK_JITTER_MS: u64 = 60_000;
pub(crate) const MAX_LOSS_PERMILLE: u16 = 1_000;
/// Hard cap on a raw scenario file's byte length, enforced by
/// [`load_scenario_json`] before any JSON parsing is attempted (Block 40.4
/// "bounded malformed file behavior").
pub(crate) const MAX_SCENARIO_FILE_BYTES: usize = 1024 * 1024;

/// Bounded real-time safety valve for one step's settlement wait -- see the
/// module doc comment's "Step settlement" section. Not simulated scenario
/// time.
const STEP_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const STEP_SETTLE_POLL: Duration = Duration::from_millis(20);

fn parse_bounded_token(kind: &'static str, value: &str) -> Result<String, ScenarioValidationError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(ScenarioValidationError::InvalidToken {
            kind,
            reason: "blank or oversized",
        });
    }
    let first_last_ok = value
        .as_bytes()
        .first()
        .zip(value.as_bytes().last())
        .is_some_and(|(first, last)| first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric());
    if !first_last_ok {
        return Err(ScenarioValidationError::InvalidToken {
            kind,
            reason: "must start and end with an ASCII letter or digit",
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ScenarioValidationError::InvalidToken {
            kind,
            reason: "may contain only ASCII letters, digits, '-' and '_'",
        });
    }
    Ok(value.to_owned())
}

macro_rules! define_scenario_token {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub(crate) struct $name(String);

        impl $name {
            #[must_use]
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                parse_bounded_token($kind, &value)
                    .map(Self)
                    .map_err(D::Error::custom)
            }
        }
    };
}

define_scenario_token!(NodeId, "node id");
define_scenario_token!(FixtureId, "fixture id");

/// Every way a scenario document can be rejected before or during
/// validation (Block 40.1's bound/reference-integrity rules, distinct from
/// [`ScenarioParseError`]'s JSON-shape rejections).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScenarioValidationError {
    InvalidToken {
        kind: &'static str,
        reason: &'static str,
    },
    TooMany {
        field: &'static str,
        limit: usize,
    },
    DurationOutOfBounds {
        field: &'static str,
        limit: u64,
    },
    DuplicateNodeId(String),
    UnknownNode {
        field: &'static str,
        node: String,
    },
    UnknownFixture {
        field: &'static str,
        fixture: String,
    },
    LinkOutOfBounds {
        field: &'static str,
        limit: u64,
    },
    StepsNotTimeOrdered {
        index: usize,
    },
}

impl fmt::Display for ScenarioValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToken { kind, reason } => {
                write!(formatter, "invalid {kind}: {reason}")
            }
            Self::TooMany { field, limit } => {
                write!(formatter, "{field} exceeds the bound of {limit}")
            }
            Self::DurationOutOfBounds { field, limit } => {
                write!(formatter, "{field} exceeds the bound of {limit} ms")
            }
            Self::DuplicateNodeId(id) => write!(formatter, "duplicate node id '{id}'"),
            Self::UnknownNode { field, node } => {
                write!(formatter, "{field} references undeclared node '{node}'")
            }
            Self::UnknownFixture { field, fixture } => {
                write!(
                    formatter,
                    "{field} references undeclared fixture '{fixture}'"
                )
            }
            Self::LinkOutOfBounds { field, limit } => {
                write!(formatter, "link {field} exceeds the bound of {limit}")
            }
            Self::StepsNotTimeOrdered { index } => {
                write!(
                    formatter,
                    "step {index} has an earlier atMs than the step before it"
                )
            }
        }
    }
}

impl std::error::Error for ScenarioValidationError {}

/// Every way loading a scenario document can fail before validation even
/// starts (Block 40.1 "reject unknown versions").
#[derive(Debug)]
pub(crate) enum ScenarioParseError {
    TooLarge { limit: usize },
    NotUtf8OrJson(serde_json::Error),
    MissingSchemaVersion,
    UnknownSchemaVersion { found: u64 },
    Shape(serde_json::Error),
}

impl fmt::Display for ScenarioParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit } => {
                write!(
                    formatter,
                    "scenario file exceeds the bound of {limit} bytes"
                )
            }
            Self::NotUtf8OrJson(error) => {
                write!(formatter, "scenario file is not valid JSON: {error}")
            }
            Self::MissingSchemaVersion => {
                formatter.write_str("scenario file has no schemaVersion field")
            }
            Self::UnknownSchemaVersion { found } => {
                write!(
                    formatter,
                    "unsupported schemaVersion {found}, expected {SCHEMA_VERSION}"
                )
            }
            Self::Shape(error) => write!(
                formatter,
                "scenario file does not match the schema: {error}"
            ),
        }
    }
}

impl std::error::Error for ScenarioParseError {}

/// Parses and bound-checks raw scenario JSON bytes (Block 40.1 "bound ...
/// duration" applied to the file itself; "reject unknown versions" checked
/// before the rest of the document is structurally parsed, so a
/// differently-shaped future schema version is reported as a version
/// mismatch rather than a generic shape error whenever possible).
///
/// # Errors
///
/// Returns [`ScenarioParseError`] for an oversized file, invalid JSON, a
/// missing/unsupported `schemaVersion`, or a document that otherwise does
/// not match this version's schema (including an unrecognized command or
/// assertion `"kind"`).
pub(crate) fn load_scenario_json(bytes: &[u8]) -> Result<Scenario, ScenarioParseError> {
    if bytes.len() > MAX_SCENARIO_FILE_BYTES {
        return Err(ScenarioParseError::TooLarge {
            limit: MAX_SCENARIO_FILE_BYTES,
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(ScenarioParseError::NotUtf8OrJson)?;
    let found_version = value
        .get("schemaVersion")
        .ok_or(ScenarioParseError::MissingSchemaVersion)?
        .as_u64()
        .ok_or(ScenarioParseError::MissingSchemaVersion)?;
    if found_version != u64::from(SCHEMA_VERSION) {
        return Err(ScenarioParseError::UnknownSchemaVersion {
            found: found_version,
        });
    }
    serde_json::from_value(value).map_err(ScenarioParseError::Shape)
}

// ---------------------------------------------------------------------
// Schema types (Block 40.2)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScenarioNode {
    pub(crate) id: NodeId,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScenarioClock {
    #[serde(default)]
    pub(crate) offset_ms: i64,
    #[serde(default)]
    pub(crate) drift_ppm: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScenarioLink {
    pub(crate) from: NodeId,
    pub(crate) to: NodeId,
    #[serde(default)]
    pub(crate) latency_ms: u64,
    #[serde(default)]
    pub(crate) jitter_ms: u64,
    #[serde(default)]
    pub(crate) loss_permille: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScenarioFixture {
    pub(crate) id: FixtureId,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) byte_length: Option<u64>,
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
}

fn deserialize_app_role<'de, D>(deserializer: D) -> Result<AppRole, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, AppRole::from_wire_name)
}

fn deserialize_host_lifecycle<'de, D>(deserializer: D) -> Result<HostLifecycle, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, HostLifecycle::from_wire_name)
}

fn deserialize_listener_lifecycle<'de, D>(deserializer: D) -> Result<ListenerLifecycle, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, ListenerLifecycle::from_wire_name)
}

fn deserialize_playback_state<'de, D>(deserializer: D) -> Result<PlaybackState, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, PlaybackState::from_wire_name)
}

fn deserialize_sync_confidence<'de, D>(deserializer: D) -> Result<SyncConfidence, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, SyncConfidence::from_wire_name)
}

fn deserialize_delivery_severity<'de, D>(deserializer: D) -> Result<DeliverySeverity, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, DeliverySeverity::from_wire_name)
}

fn decode_wire_name<'de, D, T>(
    deserializer: D,
    decode: impl FnOnce(&str) -> Result<T, EnumDecodeError>,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    decode(&value).map_err(D::Error::custom)
}

/// A scenario-declared audio capability check target (Block 40.3 "snapshot
/// capability"), directly reusing the real
/// [`PermissionCapability`] enum this codebase already models
/// `CapabilitySnapshot`'s six fields with -- no parallel enum invented.
fn deserialize_permission_capability<'de, D>(
    deserializer: D,
) -> Result<PermissionCapability, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "nearbyDiscovery" => Ok(PermissionCapability::NearbyDiscovery),
        "nearbyAdvertising" => Ok(PermissionCapability::NearbyAdvertising),
        "localNetwork" => Ok(PermissionCapability::LocalNetwork),
        "audioSourceSelection" => Ok(PermissionCapability::AudioSourceSelection),
        "audioOutput" => Ok(PermissionCapability::AudioOutput),
        "secureStore" => Ok(PermissionCapability::SecureStore),
        other => Err(D::Error::custom(format!("unknown capability '{other}'"))),
    }
}

/// One scenario action (Block 40.2 "timed commands"), targeting exactly one
/// node. A deliberately curated, real subset of `CoreCommand` plus three
/// directly-injected production event types -- see the module doc comment's
/// "Deliberate scope boundaries" section for exactly what is excluded and
/// why.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ScenarioAction {
    SelectRole {
        #[serde(deserialize_with = "deserialize_app_role")]
        role: AppRole,
    },
    CreateHostSession,
    EndHostSession,
    StartDiscovery,
    StopDiscovery,
    #[serde(rename_all = "camelCase")]
    SubmitJoin {
        #[serde(default)]
        invite_code: Option<String>,
    },
    CancelJoin,
    #[serde(rename_all = "camelCase")]
    ApproveJoin {
        request_id: String,
        #[serde(default)]
        remember_for_future: bool,
    },
    #[serde(rename_all = "camelCase")]
    RejectJoin {
        request_id: String,
    },
    #[serde(rename_all = "camelCase")]
    RemoveListener {
        listener_node: NodeId,
    },
    StartPlayback {
        fixture: FixtureId,
    },
    PausePlayback,
    ResumePlayback,
    StopPlayback,
    #[serde(rename_all = "camelCase")]
    SetLocalVolume {
        linear_gain: f32,
    },
    RequestResync,
    RetryRecoverableFailure,
    ExportDiagnostics,
    Shutdown,
    /// Injects a real `AudioEvent::Underrun` (Block 40.3 "underrun/concealment
    /// bounds"). Always changes `playback_state` to `Underrun` and emits a
    /// real `audio_underrun` diagnostic -- see `ScenarioAssertion::UnderrunFramesAtMost`.
    #[serde(rename_all = "camelCase")]
    InjectUnderrun {
        missing_frames: u32,
    },
    /// Injects a real `AudioEvent::SynchronizationUpdated` for the target
    /// node's own device identity (Block 40.3 "sync confidence", "bounded
    /// offset/RTT").
    #[serde(rename_all = "camelCase")]
    InjectSynchronizationUpdated {
        #[serde(deserialize_with = "deserialize_sync_confidence")]
        confidence: SyncConfidence,
        offset_ms: f64,
        round_trip_ms: f64,
        drift_ppm: f64,
    },
    /// Injects a real `TransportEvent::DeliveryCompleted`. Without live
    /// transport wiring (see the module doc comment), no pending transport
    /// operation the actor itself started ever exists, so this
    /// deterministically fails with a real
    /// "stale or duplicate transport delivery completion" error today --
    /// still useful for `ErrorCodeObserved`/timeout-style assertions.
    #[serde(rename_all = "camelCase")]
    InjectDeliveryCompleted {
        #[serde(default)]
        operation_id: Option<String>,
        intended_peers: u32,
        successful_peers: u32,
        failed_peers: u32,
    },
}

/// Thin wrapper giving a core domain enum its own real `Deserialize` impl
/// (rather than relying on a field-level `#[serde(deserialize_with = ...)]`)
/// so it can be used directly as a newtype-variant payload in an adjacently
/// tagged enum. `deserialize_with` alone does not work there: serde's
/// derive still generates a "content field missing" fallback path that
/// requires `T: Deserialize<'de>` to typecheck even though that path is
/// never actually taken once `content` is always supplied.
macro_rules! define_wire_enum_field {
    ($wrapper:ident, $inner:ty, $decode:path) => {
        #[derive(Debug, Clone, Copy)]
        pub(crate) struct $wrapper(pub(crate) $inner);

        impl<'de> Deserialize<'de> for $wrapper {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                decode_wire_name(deserializer, $decode).map(Self)
            }
        }
    };
}

define_wire_enum_field!(WireAppRole, AppRole, AppRole::from_wire_name);
define_wire_enum_field!(
    WireHostLifecycle,
    HostLifecycle,
    HostLifecycle::from_wire_name
);
define_wire_enum_field!(
    WireListenerLifecycle,
    ListenerLifecycle,
    ListenerLifecycle::from_wire_name
);
define_wire_enum_field!(
    WirePlaybackState,
    PlaybackState,
    PlaybackState::from_wire_name
);

/// Which of the three explicit, strongly typed lifecycle machines
/// (CLAUDE.md "Model host and listener behavior explicitly with strongly
/// typed state machines") -- or the selected role -- an assertion targets
/// (Block 40.3 "lifecycle by deadline").
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "machine", content = "state", rename_all = "camelCase")]
pub(crate) enum ScenarioLifecycleTarget {
    Role(WireAppRole),
    Host(WireHostLifecycle),
    Listener(WireListenerLifecycle),
    Playback(WirePlaybackState),
}

/// One typed, bounded-deadline assertion (Block 40.3). Every kind carries
/// `by_ms`: the scenario's virtual-time deadline by which it must hold.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ScenarioAssertion {
    #[serde(rename_all = "camelCase")]
    LifecycleReached {
        by_ms: u64,
        node: NodeId,
        target: ScenarioLifecycleTarget,
    },
    #[serde(rename_all = "camelCase")]
    CapabilityAvailable {
        by_ms: u64,
        node: NodeId,
        #[serde(deserialize_with = "deserialize_permission_capability")]
        capability: PermissionCapability,
        available: bool,
    },
    #[serde(rename_all = "camelCase")]
    ListenerCountAtLeast {
        by_ms: u64,
        node: NodeId,
        count: u32,
    },
    #[serde(rename_all = "camelCase")]
    SyncConfidenceAtLeast {
        by_ms: u64,
        node: NodeId,
        #[serde(deserialize_with = "deserialize_sync_confidence")]
        confidence: SyncConfidence,
    },
    #[serde(rename_all = "camelCase")]
    SynchronizationWithinBounds {
        by_ms: u64,
        node: NodeId,
        #[serde(default)]
        max_abs_offset_ms: Option<f64>,
        #[serde(default)]
        max_round_trip_ms: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    ErrorCodeObserved {
        by_ms: u64,
        node: NodeId,
        code: String,
    },
    #[serde(rename_all = "camelCase")]
    DeliverySeverityIs {
        by_ms: u64,
        node: NodeId,
        #[serde(deserialize_with = "deserialize_delivery_severity")]
        severity: DeliverySeverity,
    },
    #[serde(rename_all = "camelCase")]
    UnderrunFramesAtMost {
        by_ms: u64,
        node: NodeId,
        max_total_missing_frames: u32,
    },
    #[serde(rename_all = "camelCase")]
    CleanShutdown { by_ms: u64, node: NodeId },
    #[serde(rename_all = "camelCase")]
    NoUnexpectedFatalError { by_ms: u64, node: NodeId },
}

impl ScenarioAssertion {
    fn by_ms(&self) -> u64 {
        match self {
            Self::LifecycleReached { by_ms, .. }
            | Self::CapabilityAvailable { by_ms, .. }
            | Self::ListenerCountAtLeast { by_ms, .. }
            | Self::SyncConfidenceAtLeast { by_ms, .. }
            | Self::SynchronizationWithinBounds { by_ms, .. }
            | Self::ErrorCodeObserved { by_ms, .. }
            | Self::DeliverySeverityIs { by_ms, .. }
            | Self::UnderrunFramesAtMost { by_ms, .. }
            | Self::CleanShutdown { by_ms, .. }
            | Self::NoUnexpectedFatalError { by_ms, .. } => *by_ms,
        }
    }

    fn node(&self) -> &NodeId {
        match self {
            Self::LifecycleReached { node, .. }
            | Self::CapabilityAvailable { node, .. }
            | Self::ListenerCountAtLeast { node, .. }
            | Self::SyncConfidenceAtLeast { node, .. }
            | Self::SynchronizationWithinBounds { node, .. }
            | Self::ErrorCodeObserved { node, .. }
            | Self::DeliverySeverityIs { node, .. }
            | Self::UnderrunFramesAtMost { node, .. }
            | Self::CleanShutdown { node, .. }
            | Self::NoUnexpectedFatalError { node, .. } => node,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::LifecycleReached { .. } => "lifecycleReached",
            Self::CapabilityAvailable { .. } => "capabilityAvailable",
            Self::ListenerCountAtLeast { .. } => "listenerCountAtLeast",
            Self::SyncConfidenceAtLeast { .. } => "syncConfidenceAtLeast",
            Self::SynchronizationWithinBounds { .. } => "synchronizationWithinBounds",
            Self::ErrorCodeObserved { .. } => "errorCodeObserved",
            Self::DeliverySeverityIs { .. } => "deliverySeverityIs",
            Self::UnderrunFramesAtMost { .. } => "underrunFramesAtMost",
            Self::CleanShutdown { .. } => "cleanShutdown",
            Self::NoUnexpectedFatalError { .. } => "noUnexpectedFatalError",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminationPolicy {
    /// When `true` (the default), the runner stops executing further steps
    /// as soon as one assertion has definitively failed (its deadline
    /// passed without holding), rather than continuing to burn the
    /// remaining `timeoutMs` budget on a scenario already known to fail.
    #[serde(default = "default_true")]
    pub(crate) stop_on_assertion_failure: bool,
}

fn default_true() -> bool {
    true
}

impl TerminationPolicy {
    fn new_default() -> Self {
        Self {
            stop_on_assertion_failure: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScenarioStep {
    pub(crate) at_ms: u64,
    pub(crate) node: NodeId,
    pub(crate) action: ScenarioAction,
}

/// A complete, versioned, bounded scenario document (Block 40.1/40.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Scenario {
    pub(crate) schema_version: u32,
    pub(crate) seed: u64,
    #[serde(default)]
    pub(crate) nodes: Vec<ScenarioNode>,
    #[serde(default)]
    pub(crate) links: Vec<ScenarioLink>,
    #[serde(default)]
    pub(crate) clocks: HashMap<String, ScenarioClock>,
    #[serde(default)]
    pub(crate) fixtures: Vec<ScenarioFixture>,
    #[serde(default)]
    pub(crate) steps: Vec<ScenarioStep>,
    #[serde(default)]
    pub(crate) assertions: Vec<ScenarioAssertion>,
    pub(crate) timeout_ms: u64,
    #[serde(default = "TerminationPolicy::new_default")]
    pub(crate) termination: TerminationPolicy,
}

impl Scenario {
    /// Validates bounds and cross-references beyond what `serde` already
    /// enforces at parse time (Block 40.1's bound and reference-integrity
    /// rules).
    ///
    /// # Errors
    ///
    /// Returns the first [`ScenarioValidationError`] found.
    pub(crate) fn validate(&self) -> Result<(), ScenarioValidationError> {
        self.validate_bounds()?;
        let known_nodes = self.known_node_ids()?;
        let known_fixtures: std::collections::HashSet<&str> = self
            .fixtures
            .iter()
            .map(|fixture| fixture.id.as_str())
            .collect();

        self.validate_links(&known_nodes)?;
        self.validate_clocks(&known_nodes)?;
        self.validate_steps(&known_nodes, &known_fixtures)?;
        self.validate_assertions(&known_nodes)
    }

    fn validate_bounds(&self) -> Result<(), ScenarioValidationError> {
        if self.nodes.len() > MAX_NODES {
            return Err(ScenarioValidationError::TooMany {
                field: "nodes",
                limit: MAX_NODES,
            });
        }
        if self.links.len() > MAX_LINKS {
            return Err(ScenarioValidationError::TooMany {
                field: "links",
                limit: MAX_LINKS,
            });
        }
        if self.fixtures.len() > MAX_FIXTURES {
            return Err(ScenarioValidationError::TooMany {
                field: "fixtures",
                limit: MAX_FIXTURES,
            });
        }
        if self.steps.len() > MAX_STEPS {
            return Err(ScenarioValidationError::TooMany {
                field: "steps",
                limit: MAX_STEPS,
            });
        }
        if self.assertions.len() > MAX_ASSERTIONS {
            return Err(ScenarioValidationError::TooMany {
                field: "assertions",
                limit: MAX_ASSERTIONS,
            });
        }
        if self.timeout_ms > MAX_SCENARIO_DURATION_MS {
            return Err(ScenarioValidationError::DurationOutOfBounds {
                field: "timeoutMs",
                limit: MAX_SCENARIO_DURATION_MS,
            });
        }
        Ok(())
    }

    fn known_node_ids(&self) -> Result<std::collections::HashSet<&str>, ScenarioValidationError> {
        let mut known_nodes = std::collections::HashSet::new();
        for node in &self.nodes {
            if !known_nodes.insert(node.id.as_str()) {
                return Err(ScenarioValidationError::DuplicateNodeId(
                    node.id.to_string(),
                ));
            }
        }
        Ok(known_nodes)
    }

    fn validate_links(
        &self,
        known_nodes: &std::collections::HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        for link in &self.links {
            if !known_nodes.contains(link.from.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "links[].from",
                    node: link.from.to_string(),
                });
            }
            if !known_nodes.contains(link.to.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "links[].to",
                    node: link.to.to_string(),
                });
            }
            if link.latency_ms > MAX_LINK_LATENCY_MS {
                return Err(ScenarioValidationError::LinkOutOfBounds {
                    field: "latencyMs",
                    limit: MAX_LINK_LATENCY_MS,
                });
            }
            if link.jitter_ms > MAX_LINK_JITTER_MS {
                return Err(ScenarioValidationError::LinkOutOfBounds {
                    field: "jitterMs",
                    limit: MAX_LINK_JITTER_MS,
                });
            }
            if link.loss_permille > MAX_LOSS_PERMILLE {
                return Err(ScenarioValidationError::LinkOutOfBounds {
                    field: "lossPermille",
                    limit: u64::from(MAX_LOSS_PERMILLE),
                });
            }
        }
        Ok(())
    }

    fn validate_clocks(
        &self,
        known_nodes: &std::collections::HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        for clock_node in self.clocks.keys() {
            if !known_nodes.contains(clock_node.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "clocks",
                    node: clock_node.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_steps(
        &self,
        known_nodes: &std::collections::HashSet<&str>,
        known_fixtures: &std::collections::HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        let mut last_at_ms = 0_u64;
        for (index, step) in self.steps.iter().enumerate() {
            if step.at_ms > MAX_SCENARIO_DURATION_MS {
                return Err(ScenarioValidationError::DurationOutOfBounds {
                    field: "steps[].atMs",
                    limit: MAX_SCENARIO_DURATION_MS,
                });
            }
            if index > 0 && step.at_ms < last_at_ms {
                return Err(ScenarioValidationError::StepsNotTimeOrdered { index });
            }
            last_at_ms = step.at_ms;
            if !known_nodes.contains(step.node.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "steps[].node",
                    node: step.node.to_string(),
                });
            }
            match &step.action {
                ScenarioAction::RemoveListener { listener_node }
                    if !known_nodes.contains(listener_node.as_str()) =>
                {
                    return Err(ScenarioValidationError::UnknownNode {
                        field: "steps[].action.listenerNode",
                        node: listener_node.to_string(),
                    });
                }
                ScenarioAction::StartPlayback { fixture }
                    if !known_fixtures.contains(fixture.as_str()) =>
                {
                    return Err(ScenarioValidationError::UnknownFixture {
                        field: "steps[].action.fixture",
                        fixture: fixture.to_string(),
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_assertions(
        &self,
        known_nodes: &std::collections::HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        for assertion in &self.assertions {
            if assertion.by_ms() > MAX_SCENARIO_DURATION_MS {
                return Err(ScenarioValidationError::DurationOutOfBounds {
                    field: "assertions[].byMs",
                    limit: MAX_SCENARIO_DURATION_MS,
                });
            }
            if !known_nodes.contains(assertion.node().as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "assertions[].node",
                    node: assertion.node().to_string(),
                });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Runner and report (Block 40, "runner")
// ---------------------------------------------------------------------

/// Everything that can stop a scenario run before every step/assertion has
/// been evaluated to completion.
#[derive(Debug)]
pub(crate) enum ScenarioExecutionError {
    Validation(ScenarioValidationError),
    Lab(DesktopErrorDto),
    ClockAdvance(LabClockError),
    CommandShape(RuntimeContractError),
    Descriptor(RuntimeRecordValidationError),
    IdentifierInvalid(String),
    UnknownNode(NodeId),
}

impl fmt::Display for ScenarioExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(formatter, "scenario is invalid: {error}"),
            Self::Lab(error) => write!(formatter, "lab runtime error: {}", error.message),
            Self::ClockAdvance(error) => write!(formatter, "clock advance failed: {error}"),
            Self::CommandShape(error) => write!(formatter, "command is invalid: {error}"),
            Self::Descriptor(error) => {
                write!(formatter, "audio source descriptor invalid: {error}")
            }
            Self::IdentifierInvalid(message) => write!(formatter, "identifier invalid: {message}"),
            Self::UnknownNode(node) => {
                write!(formatter, "unknown node '{node}' referenced at runtime")
            }
        }
    }
}

impl std::error::Error for ScenarioExecutionError {}

/// Whether a submitted step was observed to settle before
/// [`STEP_SETTLE_TIMEOUT`]. Deliberately collapses "settled because the
/// snapshot revision advanced" and "settled because a notification arrived"
/// into one `Settled` value: which of those two concurrent signals a given
/// run happens to notice first is a real thread-scheduling race between the
/// actor and notification worker threads, not scenario-semantic
/// information -- keeping it out of this enum is what keeps
/// [`ScenarioReport`] itself genuinely deterministic (Block 40.4
/// "deterministic report") instead of merely deterministic modulo which
/// thread won a race. A legitimate idempotent no-op command (e.g.
/// re-selecting an already-held role) can never be observed as `Settled`,
/// since neither the snapshot nor the notification stream changes for it --
/// see the module doc comment's "Step settlement" section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepSettlement {
    Settled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StepResult {
    pub(crate) index: usize,
    pub(crate) at_ms: u64,
    pub(crate) node: NodeId,
    pub(crate) submit_error: Option<String>,
    pub(crate) settlement: StepSettlement,
}

/// The runner evaluates every assertion exactly once, after virtual time
/// has been advanced to `scenario.timeoutMs` -- so there are only two
/// possible outcomes: the condition held at that point, or it did not
/// (Block 40.4 "timeout": the deadline was reached without the condition
/// ever holding, which for a single deadline-checked evaluation is the same
/// event as "never becomes true", covering both "impossible assertion" and
/// "timeout" -- see `scenario::tests` for scenarios distinguishing why each
/// one failed to hold even though the mechanical outcome is the same type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssertionOutcome {
    Held,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssertionResult {
    pub(crate) kind: &'static str,
    pub(crate) node: NodeId,
    pub(crate) by_ms: u64,
    pub(crate) outcome: AssertionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScenarioOutcome {
    Completed,
    TimedOut,
    ExecutionError,
}

/// A deterministic, bounded record of one scenario run (Block 40.4
/// "deterministic report": two runs of the same scenario and seed produce
/// an equal report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScenarioReport {
    pub(crate) schema_version: u32,
    pub(crate) seed: u64,
    pub(crate) outcome: ScenarioOutcome,
    pub(crate) final_time_ms: u64,
    pub(crate) step_results: Vec<StepResult>,
    pub(crate) assertion_results: Vec<AssertionResult>,
}

/// Executes one validated [`Scenario`] against a fresh set of nodes on a
/// [`LabRuntime`] (Block 40 "runner"). Each call starts its own nodes and
/// tears them down at the end -- callers wanting to inspect a still-running
/// scenario's nodes should not use this entry point (there is none yet;
/// out of scope for Block 40, see `LabScreen` in Block 42).
pub(crate) fn run_scenario(
    lab: &LabRuntime,
    scenario: &Scenario,
) -> Result<ScenarioReport, ScenarioExecutionError> {
    scenario
        .validate()
        .map_err(ScenarioExecutionError::Validation)?;

    let mut lab_node_ids: HashMap<&str, LabNodeId> = HashMap::new();
    let mut recorders: HashMap<&str, std::sync::Arc<ScenarioRecorder>> = HashMap::new();
    for node in &scenario.nodes {
        let clock = scenario
            .clocks
            .get(node.id.as_str())
            .copied()
            .unwrap_or(ScenarioClock {
                offset_ms: 0,
                drift_ppm: 0,
            });
        let recorder = ScenarioRecorder::new();
        let observer = RecordingObserver(std::sync::Arc::clone(&recorder));
        let lab_node_id = lab
            .start_node_with_clock_and_observer(clock.offset_ms, clock.drift_ppm, observer)
            .map_err(ScenarioExecutionError::Lab)?;
        lab_node_ids.insert(node.id.as_str(), lab_node_id);
        recorders.insert(node.id.as_str(), recorder);
    }

    let run_result = execute_steps_and_assertions(lab, scenario, &lab_node_ids, &recorders);

    // Every node started for this run is torn down regardless of outcome --
    // a scenario that fails midway must never leak Lab nodes.
    let mut teardown_ok = true;
    for lab_node_id in lab_node_ids.values() {
        if lab.stop_node(*lab_node_id).is_err() {
            teardown_ok = false;
        }
    }

    let mut report = run_result?;
    if !teardown_ok && report.outcome == ScenarioOutcome::Completed {
        report.outcome = ScenarioOutcome::ExecutionError;
    }
    Ok(report)
}

fn execute_steps_and_assertions(
    lab: &LabRuntime,
    scenario: &Scenario,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    recorders: &HashMap<&str, std::sync::Arc<ScenarioRecorder>>,
) -> Result<ScenarioReport, ScenarioExecutionError> {
    let mut current_ms = lab.now().get();
    let mut step_results = Vec::with_capacity(scenario.steps.len());

    for (index, step) in scenario.steps.iter().enumerate() {
        // A step scheduled at or beyond the scenario's own bounded timeout
        // budget never runs (Block 40.2 "timeout and termination policy";
        // Block 40.4 "timeout": whatever that step would have satisfied can
        // now only time out, distinctly from an assertion that could never
        // hold regardless of budget -- see `tests::a_step_scheduled_past_the_scenario_timeout_never_runs`).
        if step.at_ms >= scenario.timeout_ms {
            break;
        }
        if step.at_ms > current_ms {
            lab.advance(step.at_ms - current_ms)
                .map_err(ScenarioExecutionError::Lab)?;
            current_ms = step.at_ms;
        }

        let lab_node_id = *lab_node_ids
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let handle = lab
            .node_handle(lab_node_id)
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let recorder = recorders
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;

        let revision_before = current_revision(&handle);
        let sequence_before = recorder.next_sequence();

        let submit_error = submit_action(lab, &handle, lab_node_id, lab_node_ids, &step.action)?;

        let settlement = if submit_error.is_some() {
            StepSettlement::Settled
        } else {
            wait_for_step_settled(&handle, recorder, revision_before, sequence_before)
        };

        step_results.push(StepResult {
            index,
            at_ms: step.at_ms,
            node: step.node.clone(),
            submit_error,
            settlement,
        });
    }

    // Advance to the scenario's overall timeout so every assertion's
    // deadline has genuinely been reached before being evaluated.
    if scenario.timeout_ms > current_ms {
        lab.advance(scenario.timeout_ms - current_ms)
            .map_err(ScenarioExecutionError::Lab)?;
        current_ms = scenario.timeout_ms;
    }

    let mut assertion_results = Vec::with_capacity(scenario.assertions.len());
    let mut outcome = ScenarioOutcome::Completed;
    for assertion in &scenario.assertions {
        let lab_node_id = *lab_node_ids
            .get(assertion.node().as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(assertion.node().clone()))?;
        let handle = lab
            .node_handle(lab_node_id)
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(assertion.node().clone()))?;
        let recorder = recorders
            .get(assertion.node().as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(assertion.node().clone()))?;

        let snapshot = handle.current_snapshot().ok();
        let entries = recorder.entries();
        let held = evaluate_assertion(assertion, snapshot.as_ref(), &entries);
        let assertion_outcome = if held {
            AssertionOutcome::Held
        } else {
            AssertionOutcome::TimedOut
        };
        if assertion_outcome != AssertionOutcome::Held {
            outcome = ScenarioOutcome::TimedOut;
        }
        assertion_results.push(AssertionResult {
            kind: assertion.kind_name(),
            node: assertion.node().clone(),
            by_ms: assertion.by_ms(),
            outcome: assertion_outcome,
        });
        if assertion_outcome != AssertionOutcome::Held
            && scenario.termination.stop_on_assertion_failure
        {
            break;
        }
    }

    Ok(ScenarioReport {
        schema_version: scenario.schema_version,
        seed: scenario.seed,
        outcome,
        final_time_ms: current_ms,
        step_results,
        assertion_results,
    })
}

fn wait_for_step_settled(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    recorder: &ScenarioRecorder,
    revision_before: SnapshotRevision,
    sequence_before: u64,
) -> StepSettlement {
    let mut remaining = STEP_SETTLE_TIMEOUT;
    loop {
        if let Ok(snapshot) = handle.current_snapshot()
            && snapshot.revision.get() > revision_before.get()
        {
            return StepSettlement::Settled;
        }
        // Deliberately does *not* treat "any new notification recorded" as
        // settlement: a freshly started node's actor also sends its own
        // startup `Snapshot` notification asynchronously, which can race
        // with `sequence_before` being captured just before this step's own
        // submission and land afterward -- a false positive that once
        // caused a genuinely still-in-flight command to be treated as
        // already settled under load. Only a `Snapshot` whose revision has
        // genuinely advanced past `revision_before`, or a real `Error`
        // (the actor's only other outcome, see the module doc comment's
        // "Step settlement" section), counts.
        for entry in recorder.entries() {
            if entry.sequence < sequence_before {
                continue;
            }
            match entry.kind {
                RecordedNotificationKind::Snapshot { revision }
                    if revision > revision_before.get() =>
                {
                    return StepSettlement::Settled;
                }
                RecordedNotificationKind::Error { .. } => return StepSettlement::Settled,
                _ => {}
            }
        }
        if remaining.is_zero() {
            return StepSettlement::TimedOut;
        }
        let chunk = remaining.min(STEP_SETTLE_POLL);
        remaining -= chunk;
        recorder.wait_for_progress(sequence_before, chunk);
    }
}

/// The current revision, or revision zero when the snapshot cannot be read
/// (a poisoned or already-failed actor) -- callers always have a real
/// `CoreCommandRequest::new` validation step downstream that will reject an
/// obviously stale revision through the normal command path rather than
/// this helper silently pretending the read succeeded.
fn current_revision(handle: &silent_disco_core::runtime::CoreActorHandle) -> SnapshotRevision {
    handle
        .current_snapshot()
        .map_or(SnapshotRevision::new(0), |snapshot| snapshot.revision)
}

fn submit_action(
    lab: &LabRuntime,
    handle: &silent_disco_core::runtime::CoreActorHandle,
    lab_node_id: LabNodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    action: &ScenarioAction,
) -> Result<Option<String>, ScenarioExecutionError> {
    match action {
        ScenarioAction::RemoveListener { listener_node } => {
            let listener_id = resolve_remove_listener(lab, listener_node, lab_node_ids)?;
            let revision = current_revision(handle);
            let request =
                CoreCommandRequest::new(revision, CoreCommand::RemoveListener { listener_id })
                    .map_err(ScenarioExecutionError::CommandShape)?;
            return Ok(match handle.submit_command(request) {
                Ok(_receipt) => None,
                Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
            });
        }
        ScenarioAction::InjectUnderrun { missing_frames } => {
            return Ok(submit_audio(
                handle,
                AudioEvent::Underrun {
                    missing_frames: *missing_frames,
                },
            ));
        }
        ScenarioAction::InjectSynchronizationUpdated {
            confidence,
            offset_ms,
            round_trip_ms,
            drift_ppm,
        } => {
            let identity = lab.node_identity(lab_node_id).ok_or_else(|| {
                ScenarioExecutionError::IdentifierInvalid("node has no identity".to_owned())
            })?;
            let summary =
                SynchronizationSummary::new(*confidence, *offset_ms, *round_trip_ms, *drift_ppm)
                    .map_err(ScenarioExecutionError::Descriptor)?;
            let device_id = identity.device_id().clone();
            return Ok(submit_audio(
                handle,
                AudioEvent::SynchronizationUpdated { device_id, summary },
            ));
        }
        ScenarioAction::InjectDeliveryCompleted {
            operation_id,
            intended_peers,
            successful_peers,
            failed_peers,
        } => {
            let operation_id = match operation_id {
                Some(value) => OperationId::new(value.clone()).map_err(|error| {
                    ScenarioExecutionError::IdentifierInvalid(error.to_string())
                })?,
                None => OperationId::new(format!("lab-scenario-delivery-{lab_node_id:?}"))
                    .map_err(|error| {
                        ScenarioExecutionError::IdentifierInvalid(error.to_string())
                    })?,
            };
            let report = DeliveryReport::new(*intended_peers, *successful_peers, *failed_peers)
                .map_err(ScenarioExecutionError::Descriptor)?;
            return Ok(submit_transport(
                handle,
                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                },
            ));
        }
        _ => {}
    }

    let command = build_command(action)?;
    let revision = current_revision(handle);
    let request =
        CoreCommandRequest::new(revision, command).map_err(ScenarioExecutionError::CommandShape)?;
    match handle.submit_command(request) {
        Ok(_receipt) => Ok(None),
        Err(error) => Ok(Some(format!(
            "{}: {}",
            error.code.stable_name(),
            error.message
        ))),
    }
}

fn submit_audio(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    event: AudioEvent,
) -> Option<String> {
    match handle.submit_audio_event(event) {
        Ok(()) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    }
}

fn submit_transport(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    event: TransportEvent,
) -> Option<String> {
    match handle.submit_transport_event(event) {
        Ok(()) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    }
}

fn build_command(action: &ScenarioAction) -> Result<CoreCommand, ScenarioExecutionError> {
    Ok(match action {
        ScenarioAction::SelectRole { role } => CoreCommand::SelectRole { role: *role },
        ScenarioAction::CreateHostSession => CoreCommand::CreateHostSession,
        ScenarioAction::EndHostSession => CoreCommand::EndHostSession,
        ScenarioAction::StartDiscovery => CoreCommand::StartDiscovery,
        ScenarioAction::StopDiscovery => CoreCommand::StopDiscovery,
        ScenarioAction::SubmitJoin { invite_code } => CoreCommand::SubmitJoin {
            invite_code: invite_code.clone(),
        },
        ScenarioAction::CancelJoin => CoreCommand::CancelJoin,
        ScenarioAction::ApproveJoin {
            request_id,
            remember_for_future,
        } => CoreCommand::ApproveJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
            remember_for_future: *remember_for_future,
        },
        ScenarioAction::RejectJoin { request_id } => CoreCommand::RejectJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
        },
        ScenarioAction::StartPlayback { fixture } => {
            let descriptor =
                AudioSourceDescriptor::new(fixture.as_str(), fixture.as_str(), None, None)
                    .map_err(ScenarioExecutionError::Descriptor)?;
            CoreCommand::StartPlayback { source: descriptor }
        }
        ScenarioAction::PausePlayback => CoreCommand::PausePlayback,
        ScenarioAction::ResumePlayback => CoreCommand::ResumePlayback,
        ScenarioAction::StopPlayback => CoreCommand::StopPlayback,
        ScenarioAction::SetLocalVolume { linear_gain } => CoreCommand::SetLocalVolume {
            linear_gain: *linear_gain,
        },
        ScenarioAction::RequestResync => CoreCommand::RequestResync,
        ScenarioAction::RetryRecoverableFailure => CoreCommand::RetryRecoverableFailure,
        ScenarioAction::ExportDiagnostics => CoreCommand::ExportDiagnostics,
        ScenarioAction::Shutdown => CoreCommand::Shutdown,
        ScenarioAction::RemoveListener { .. }
        | ScenarioAction::InjectUnderrun { .. }
        | ScenarioAction::InjectSynchronizationUpdated { .. }
        | ScenarioAction::InjectDeliveryCompleted { .. } => {
            unreachable!("handled directly in submit_action before build_command is called")
        }
    })
}

/// `RemoveListener` needs the target listener node's own `DeviceId`, which
/// `build_command` alone cannot resolve (it has no access to `LabRuntime`).
/// Resolved here instead, before falling back to `build_command` for every
/// other action.
fn resolve_remove_listener(
    lab: &LabRuntime,
    listener_node_scenario_id: &NodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
) -> Result<DeviceId, ScenarioExecutionError> {
    let lab_node_id = *lab_node_ids
        .get(listener_node_scenario_id.as_str())
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(listener_node_scenario_id.clone()))?;
    let identity = lab
        .node_identity(lab_node_id)
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(listener_node_scenario_id.clone()))?;
    Ok(identity.device_id().clone())
}

// ---------------------------------------------------------------------
// Assertion evaluation (Block 40.3)
// ---------------------------------------------------------------------

fn evaluate_assertion(
    assertion: &ScenarioAssertion,
    snapshot: Option<&CoreSnapshot>,
    entries: &[super::recorder::RecordedNotification],
) -> bool {
    match assertion {
        ScenarioAssertion::LifecycleReached { target, .. } => {
            let Some(snapshot) = snapshot else { return false };
            match target {
                ScenarioLifecycleTarget::Role(role) => snapshot.selected_role == Some(role.0),
                ScenarioLifecycleTarget::Host(state) => snapshot.host_lifecycle == state.0,
                ScenarioLifecycleTarget::Listener(state) => snapshot.listener_lifecycle == state.0,
                ScenarioLifecycleTarget::Playback(state) => snapshot.playback_state == state.0,
            }
        }
        ScenarioAssertion::CapabilityAvailable { capability, available, .. } => {
            let Some(snapshot) = snapshot else { return false };
            let actual = match capability {
                PermissionCapability::NearbyDiscovery => snapshot.capabilities.nearby_discovery_available,
                PermissionCapability::NearbyAdvertising => {
                    snapshot.capabilities.nearby_advertising_available
                }
                PermissionCapability::LocalNetwork => snapshot.capabilities.local_network_available,
                PermissionCapability::AudioSourceSelection => {
                    snapshot.capabilities.audio_source_selection_available
                }
                PermissionCapability::AudioOutput => snapshot.capabilities.audio_output_available,
                PermissionCapability::SecureStore => snapshot.capabilities.secure_store_available,
            };
            actual == *available
        }
        ScenarioAssertion::ListenerCountAtLeast { count, .. } => {
            let Some(snapshot) = snapshot else { return false };
            u32::try_from(snapshot.listeners.len()).unwrap_or(u32::MAX) >= *count
        }
        ScenarioAssertion::SyncConfidenceAtLeast { confidence, .. } => {
            let Some(snapshot) = snapshot else { return false };
            snapshot
                .synchronization
                .is_some_and(|summary| summary.confidence.stable_code() >= confidence.stable_code())
        }
        ScenarioAssertion::SynchronizationWithinBounds { max_abs_offset_ms, max_round_trip_ms, .. } => {
            let Some(snapshot) = snapshot else { return false };
            let Some(summary) = snapshot.synchronization else { return false };
            let offset_ok = max_abs_offset_ms.is_none_or(|bound| summary.offset_ms.abs() <= bound);
            let rtt_ok = max_round_trip_ms.is_none_or(|bound| summary.round_trip_ms <= bound);
            offset_ok && rtt_ok
        }
        ScenarioAssertion::ErrorCodeObserved { code, .. } => entries.iter().any(|entry| {
            matches!(&entry.kind, RecordedNotificationKind::Error { code: observed, .. } if observed == code)
        }),
        ScenarioAssertion::DeliverySeverityIs { severity, .. } => {
            let Some(snapshot) = snapshot else { return false };
            snapshot.last_delivery.is_some_and(|report| report.severity.stable_code() == severity.stable_code())
        }
        ScenarioAssertion::UnderrunFramesAtMost { max_total_missing_frames, .. } => {
            let total: u64 = entries
                .iter()
                .filter_map(|entry| match &entry.kind {
                    RecordedNotificationKind::Diagnostic { name, fields } if name == "audio_underrun" => {
                        fields
                            .iter()
                            .find(|(key, _)| key == "missing_frames")
                            .and_then(|(_, value)| value.parse::<u64>().ok())
                    }
                    _ => None,
                })
                .sum();
            total <= u64::from(*max_total_missing_frames)
        }
        ScenarioAssertion::CleanShutdown { .. } => {
            !entries.iter().any(|entry| entry.kind.is_fatal_error())
        }
        ScenarioAssertion::NoUnexpectedFatalError { .. } => {
            !entries.iter().any(|entry| entry.kind.is_fatal_error())
        }
    }
}

#[cfg(test)]
mod tests;
