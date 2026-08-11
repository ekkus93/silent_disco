use crate::lab::MAX_LAB_NODES;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use silent_disco_core::domain::{
    AppRole, DeliverySeverity, EnumDecodeError, HostLifecycle, ListenerLifecycle, PlaybackState,
    SyncConfidence,
};
use silent_disco_core::runtime::PermissionCapability;
use std::collections::HashMap;
use std::fmt;

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_NODES: usize = MAX_LAB_NODES;
pub(crate) const MAX_LINKS: usize = 64;
pub(crate) const MAX_FIXTURES: usize = 32;
pub(crate) const MAX_STEPS: usize = 256;
pub(crate) const MAX_ASSERTIONS: usize = 128;
pub(crate) const MAX_ID_BYTES: usize = 64;
pub(crate) const MAX_DISPLAY_NAME_BYTES: usize = 256;
pub(super) const MAX_TOKEN_BYTES: usize = 256;
pub(super) const MAX_ERROR_CODE_BYTES: usize = 128;
pub(crate) const MAX_SCENARIO_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
pub(crate) const MAX_LINK_LATENCY_MS: u64 = 60_000;
pub(crate) const MAX_LINK_JITTER_MS: u64 = 60_000;
pub(crate) const MAX_LOSS_PERMILLE: u16 = 1_000;
pub(crate) const MAX_SCENARIO_FILE_BYTES: usize = 1024 * 1024;

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
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(pub(crate) String);

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
    AmbiguousInboundLinkFaults {
        node: String,
    },
    StepsNotTimeOrdered {
        index: usize,
    },
}

impl fmt::Display for ScenarioValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToken { kind, reason } => write!(formatter, "invalid {kind}: {reason}"),
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
            Self::AmbiguousInboundLinkFaults { node } => write!(
                formatter,
                "node '{node}' has conflicting inbound receive-fault profiles; the current virtual transport applies latency/jitter/loss per receiving node, not per peer"
            ),
            Self::StepsNotTimeOrdered { index } => write!(
                formatter,
                "step {index} has an earlier atMs than the step before it"
            ),
        }
    }
}

impl std::error::Error for ScenarioValidationError {}

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
    #[serde(deserialize_with = "deserialize_display_name")]
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) byte_length: Option<u64>,
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
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

fn deserialize_app_role<'de, D>(deserializer: D) -> Result<AppRole, D::Error>
where
    D: Deserializer<'de>,
{
    decode_wire_name(deserializer, AppRole::from_wire_name)
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

fn deserialize_display_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_DISPLAY_NAME_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(D::Error::custom(
            "display name is blank, oversized, whitespace-surrounded, or contains control characters",
        ));
    }
    Ok(value)
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || value.len() > MAX_TOKEN_BYTES {
        return Err(D::Error::custom("sessionId is blank or oversized"));
    }
    Ok(value)
}

fn deserialize_bounded_optional_token<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if value
        .as_ref()
        .is_some_and(|token| token.len() > MAX_TOKEN_BYTES)
    {
        return Err(D::Error::custom("token is oversized"));
    }
    Ok(value)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ScenarioAction {
    SelectRole {
        #[serde(deserialize_with = "deserialize_app_role")]
        role: AppRole,
    },
    #[serde(rename_all = "camelCase")]
    ConfigureHost {
        #[serde(deserialize_with = "deserialize_display_name")]
        session_name: String,
        fixture: FixtureId,
    },
    CreateHostSession,
    EndHostSession,
    StartDiscovery,
    StopDiscovery,
    #[serde(rename_all = "camelCase")]
    SelectSession {
        #[serde(deserialize_with = "deserialize_session_id")]
        session_id: String,
    },
    #[serde(rename_all = "camelCase")]
    SubmitJoin {
        #[serde(default, deserialize_with = "deserialize_bounded_optional_token")]
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
    #[serde(rename_all = "camelCase")]
    InjectUnderrun {
        missing_frames: u32,
    },
    #[serde(rename_all = "camelCase")]
    InjectSynchronizationUpdated {
        #[serde(deserialize_with = "deserialize_sync_confidence")]
        confidence: SyncConfidence,
        offset_ms: f64,
        round_trip_ms: f64,
        drift_ppm: f64,
    },
    #[serde(rename_all = "camelCase")]
    InjectDeliveryCompleted {
        #[serde(default)]
        operation_id: Option<String>,
        intended_peers: u32,
        successful_peers: u32,
        failed_peers: u32,
    },
}

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

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "machine", content = "state", rename_all = "camelCase")]
pub(crate) enum ScenarioLifecycleTarget {
    Role(WireAppRole),
    Host(WireHostLifecycle),
    Listener(WireListenerLifecycle),
    Playback(WirePlaybackState),
}

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
    pub(in crate::lab::scenario) fn by_ms(&self) -> u64 {
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

    pub(in crate::lab::scenario) fn node(&self) -> &NodeId {
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

    pub(in crate::lab::scenario) fn kind_name(&self) -> &'static str {
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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminationPolicy {
    #[serde(default = "default_true")]
    pub(crate) stop_on_assertion_failure: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TerminationPolicy {
    fn default() -> Self {
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
    #[serde(default)]
    pub(crate) termination: TerminationPolicy,
}
