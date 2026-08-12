use super::NodeId;
use crate::dto::DesktopErrorDto;
use crate::lab::clock::LabClockError;
use crate::lab::fault::trace::TransportTrace;
use crate::lab::recorder::RecordedNotification;
use serde::{Deserialize, Serialize};
use silent_disco_core::runtime::{RuntimeContractError, RuntimeRecordValidationError};
use std::fmt;

#[derive(Debug)]
pub(crate) enum ScenarioExecutionError {
    Validation(super::ScenarioValidationError),
    Lab(DesktopErrorDto),
    ClockAdvance(LabClockError),
    CommandShape(RuntimeContractError),
    Descriptor(RuntimeRecordValidationError),
    IdentifierInvalid(String),
    UnknownNode(NodeId),
    Teardown {
        primary: Box<ScenarioExecutionError>,
        cleanup: DesktopErrorDto,
    },
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
            Self::Teardown { primary, cleanup } => write!(
                formatter,
                "{primary}; scenario teardown also failed: {}",
                cleanup.message
            ),
        }
    }
}

impl std::error::Error for ScenarioExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StepSettlement {
    Settled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StepResult {
    pub(crate) index: usize,
    pub(crate) at_ms: u64,
    pub(crate) node: NodeId,
    pub(crate) submit_error: Option<String>,
    pub(crate) settlement: StepSettlement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AssertionOutcome {
    Held,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssertionResult {
    pub(crate) kind: String,
    pub(crate) node: NodeId,
    pub(crate) by_ms: u64,
    pub(crate) outcome: AssertionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ScenarioOutcome {
    Completed,
    TimedOut,
    ExecutionError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenarioReport {
    pub(crate) schema_version: u32,
    pub(crate) seed: u64,
    pub(crate) outcome: ScenarioOutcome,
    pub(crate) final_time_ms: u64,
    pub(crate) step_results: Vec<StepResult>,
    pub(crate) assertion_results: Vec<AssertionResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClockAdvance {
    pub(crate) requested_delta_ms: u64,
    pub(crate) resulting_now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenarioTrace {
    pub(crate) clock_advances: Vec<ClockAdvance>,
    pub(crate) node_notifications: Vec<(String, Vec<RecordedNotification>)>,
    pub(crate) transport_trace: TransportTrace,
}
