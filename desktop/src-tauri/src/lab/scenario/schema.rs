use super::legacy;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use silent_disco_core::domain::{AppRole, EnumDecodeError, SyncConfidence};
use std::collections::{HashMap, HashSet};

pub(crate) use legacy::{
    FixtureId, NodeId, ScenarioAssertion, ScenarioClock, ScenarioFixture, ScenarioLink,
    ScenarioLifecycleTarget, ScenarioParseError, ScenarioValidationError, TerminationPolicy,
    MAX_ASSERTIONS, MAX_DISPLAY_NAME_BYTES, MAX_FIXTURES, MAX_ID_BYTES, MAX_LINKS,
    MAX_LINK_JITTER_MS, MAX_LINK_LATENCY_MS, MAX_LOSS_PERMILLE, MAX_NODES,
    MAX_SCENARIO_DURATION_MS, MAX_SCENARIO_FILE_BYTES, MAX_STEPS,
};

pub(crate) const SCHEMA_VERSION: u32 = legacy::SCHEMA_VERSION;

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

fn deserialize_session_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_DISPLAY_NAME_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(D::Error::custom("sessionName is blank, oversized, whitespace-surrounded, or contains control characters"));
    }
    Ok(value)
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || value.len() > legacy::MAX_TOKEN_BYTES {
        return Err(D::Error::custom("sessionId is blank or oversized"));
    }
    Ok(value)
}

/// A timed Lab action. Every domain operation still maps to the real
/// `CoreCommand`/event surface; the two additions here are the minimum needed
/// to make a complete live host/listener flow expressible now that discovery
/// and transport are real:
///
/// - `configureHost` maps to `CoreCommand::UpdateHostDraft`;
/// - `selectSession` maps to `CoreCommand::SelectSession`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ScenarioAction {
    SelectRole {
        #[serde(deserialize_with = "deserialize_app_role")]
        role: AppRole,
    },
    #[serde(rename_all = "camelCase")]
    ConfigureHost {
        #[serde(deserialize_with = "deserialize_session_name")]
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

impl ScenarioAction {
    pub(super) fn requires_live_runner(&self) -> bool {
        matches!(self, Self::ConfigureHost { .. } | Self::SelectSession { .. })
    }

    fn validation_surrogate(&self) -> legacy::ScenarioAction {
        match self {
            Self::SelectRole { role } => legacy::ScenarioAction::SelectRole { role: *role },
            // The legacy validator only needs a fixture-bearing action here;
            // StartPlayback applies the same declared-fixture cross-reference
            // check that configureHost needs.
            Self::ConfigureHost { fixture, .. } => legacy::ScenarioAction::StartPlayback {
                fixture: fixture.clone(),
            },
            Self::CreateHostSession => legacy::ScenarioAction::CreateHostSession,
            Self::EndHostSession => legacy::ScenarioAction::EndHostSession,
            Self::StartDiscovery => legacy::ScenarioAction::StartDiscovery,
            Self::StopDiscovery => legacy::ScenarioAction::StopDiscovery,
            // Session ID shape is bounded by this module's deserializer and
            // the real SessionId constructor during command creation. There
            // is no additional cross-reference the legacy validator can add.
            Self::SelectSession { .. } => legacy::ScenarioAction::ExportDiagnostics,
            Self::SubmitJoin { invite_code } => legacy::ScenarioAction::SubmitJoin {
                invite_code: invite_code.clone(),
            },
            Self::CancelJoin => legacy::ScenarioAction::CancelJoin,
            Self::ApproveJoin {
                request_id,
                remember_for_future,
            } => legacy::ScenarioAction::ApproveJoin {
                request_id: request_id.clone(),
                remember_for_future: *remember_for_future,
            },
            Self::RejectJoin { request_id } => legacy::ScenarioAction::RejectJoin {
                request_id: request_id.clone(),
            },
            Self::RemoveListener { listener_node } => legacy::ScenarioAction::RemoveListener {
                listener_node: listener_node.clone(),
            },
            Self::StartPlayback { fixture } => legacy::ScenarioAction::StartPlayback {
                fixture: fixture.clone(),
            },
            Self::PausePlayback => legacy::ScenarioAction::PausePlayback,
            Self::ResumePlayback => legacy::ScenarioAction::ResumePlayback,
            Self::StopPlayback => legacy::ScenarioAction::StopPlayback,
            Self::SetLocalVolume { linear_gain } => legacy::ScenarioAction::SetLocalVolume {
                linear_gain: *linear_gain,
            },
            Self::RequestResync => legacy::ScenarioAction::RequestResync,
            Self::RetryRecoverableFailure => legacy::ScenarioAction::RetryRecoverableFailure,
            Self::ExportDiagnostics => legacy::ScenarioAction::ExportDiagnostics,
            Self::Shutdown => legacy::ScenarioAction::Shutdown,
            Self::InjectUnderrun { missing_frames } => legacy::ScenarioAction::InjectUnderrun {
                missing_frames: *missing_frames,
            },
            Self::InjectSynchronizationUpdated {
                confidence,
                offset_ms,
                round_trip_ms,
                drift_ppm,
            } => legacy::ScenarioAction::InjectSynchronizationUpdated {
                confidence: *confidence,
                offset_ms: *offset_ms,
                round_trip_ms: *round_trip_ms,
                drift_ppm: *drift_ppm,
            },
            Self::InjectDeliveryCompleted {
                operation_id,
                intended_peers,
                successful_peers,
                failed_peers,
            } => legacy::ScenarioAction::InjectDeliveryCompleted {
                operation_id: operation_id.clone(),
                intended_peers: *intended_peers,
                successful_peers: *successful_peers,
                failed_peers: *failed_peers,
            },
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
    pub(crate) nodes: Vec<legacy::ScenarioNode>,
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
    #[serde(default = "default_termination_policy")]
    pub(crate) termination: TerminationPolicy,
}

fn default_termination_policy() -> TerminationPolicy {
    TerminationPolicy {
        stop_on_assertion_failure: true,
    }
}

impl Scenario {
    pub(crate) fn validate(&self) -> Result<(), ScenarioValidationError> {
        // Preserve Block 40's exact bounds/reference validation by converting
        // to a validation-only legacy shape. The two new actions use
        // semantically equivalent surrogates only for validation; execution
        // never goes through those surrogates.
        self.as_legacy_validation_scenario().validate()?;

        // A node-targeted receive fault is one real transport-node wrapper.
        // The current shared virtual transport cannot represent two different
        // receive profiles for the same target node. Reject that topology
        // explicitly rather than silently selecting one link's values.
        let mut incoming: HashMap<&str, (u64, u64, u16)> = HashMap::new();
        for link in &self.links {
            let profile = (link.latency_ms, link.jitter_ms, link.loss_permille);
            if let Some(existing) = incoming.insert(link.to.as_str(), profile)
                && existing != profile
            {
                return Err(ScenarioValidationError::LinkOutOfBounds {
                    field: "multiple inbound links require one receive fault profile per target node",
                    limit: 0,
                });
            }
        }

        let fixtures: HashSet<&str> = self.fixtures.iter().map(|fixture| fixture.id.as_str()).collect();
        for step in &self.steps {
            if let ScenarioAction::ConfigureHost { fixture, .. } = &step.action
                && !fixtures.contains(fixture.as_str())
            {
                return Err(ScenarioValidationError::UnknownFixture {
                    field: "steps[].action.fixture",
                    fixture: fixture.to_string(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn requires_live_runner(&self) -> bool {
        !self.links.is_empty() || self.steps.iter().any(|step| step.action.requires_live_runner())
    }

    pub(super) fn as_legacy_validation_scenario(&self) -> legacy::Scenario {
        legacy::Scenario {
            schema_version: self.schema_version,
            seed: self.seed,
            nodes: self.nodes.clone(),
            links: self.links.clone(),
            clocks: self.clocks.clone(),
            fixtures: self.fixtures.clone(),
            steps: self
                .steps
                .iter()
                .map(|step| legacy::ScenarioStep {
                    at_ms: step.at_ms,
                    node: step.node.clone(),
                    action: step.action.validation_surrogate(),
                })
                .collect(),
            assertions: self.assertions.clone(),
            timeout_ms: self.timeout_ms,
            termination: self.termination,
        }
    }
}

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
