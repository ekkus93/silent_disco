//! Persisted, versioned scenario recordings and divergence detection
//! against a fresh replay (Block 41; spec section 29.5).
//!
//! Block 40 already built [`super::scenario::run_scenario`] (a deterministic
//! runner) and a first-cut [`super::replay`] that checked a scenario's own
//! `schemaVersion`/`seed` before re-executing it. This module is Block 41's
//! own extension, explicitly deferred by Block 40's own doc comments:
//!
//! - a real **on-disk, versioned, bounded persisted format**
//!   ([`ScenarioRecording`], [`save_recording_to_path`]/
//!   [`load_recording_from_path`]) -- not just an in-process struct;
//! - **protocol/core version stamping** ([`RecordedCoreVersion`],
//!   `protocol_version`) -- recorded for diagnosis, deliberately **not** a
//!   compatibility gate (see "Which versions gate replay" below);
//! - **divergence detection** ([`first_divergence`]/[`first_trace_divergence`])
//!   comparing a fresh replay's semantic report and then its persisted
//!   deterministic trace against the recording, returning one bounded
//!   [`Divergence`] at the first stored mismatch rather than an unbounded list.
//!
//! ## Which versions gate replay, and why "later core build" is not a
//! ## contradiction
//!
//! Spec 29.5 requires replay to "detect version incompatibility rather than
//! silently reinterpret an old recording", and Block 41's own acceptance
//! criterion is "a difficult failure can be saved and replayed **against a
//! later core build**". Those two requirements are only compatible if
//! "version incompatibility" means *structural* incompatibility -- a
//! recording whose on-disk shape or whose scenario document no longer
//! matches what was captured, which cannot be safely compared or
//! re-executed at all -- rather than *semantic* version drift, which is
//! exactly what replaying against a later build is supposed to surface as a
//! [`Divergence`], not refuse outright. This module therefore splits the
//! recorded versions into two groups:
//!
//! - **Hard gates, checked by [`super::replay::replay`] before any
//!   re-execution is attempted**: [`ScenarioRecording::recording_format_version`]
//!   (this module's own on-disk shape), the scenario's `schemaVersion`, and
//!   its `seed`. A mismatch on any of these means the recorded trace and a
//!   fresh run are not even talking about the same input, so comparing them
//!   would be meaningless or actively misleading -- refused outright
//!   ([`super::replay::ReplayError`]), never silently reinterpreted.
//! - **Informational, always recorded, never gating**:
//!   [`ScenarioRecording::protocol_version`] and
//!   [`ScenarioRecording::core_version`]. These are expected to differ
//!   between the original capture and a later replay -- that difference is
//!   precisely what a maintainer investigating a regression wants surfaced,
//!   not blocked. [`super::replay::ReplayOutcome`] carries both the
//!   recorded and current values of each alongside the [`Divergence`] (if
//!   any), so a caller can present "captured under core 0.1.0, replayed
//!   under 0.1.1: <divergence>" rather than a bare pass/fail.
//!
//! ## Secret redaction
//!
//! This module persists exactly what [`super::scenario::ScenarioTrace`] and
//! [`super::scenario::ScenarioReport`] already contain -- it does not read
//! [`silent_disco_core::runtime::CoreSnapshot`] itself. The one place a raw
//! snapshot is ever read is [`super::recorder::SnapshotSummary::capture`],
//! whose own doc comment lists exactly what is deliberately excluded (most
//! importantly, a host session's plaintext `invite_code`) and why. See
//! `super::recorder::tests::snapshot_summary_never_carries_the_raw_invite_code`
//! for the direct test. The live-transport trace follows the same boundary:
//! it omits raw peer device IDs and raw frame/audio bytes, and a `JoinRequest`
//! carrying an invite code is redacted before its diagnostic frame hash is
//! calculated so the recording cannot become a low-entropy secret verifier.
//!
//! ## Deliberately excluded
//!
//! - **Raw packet bodies and raw audio payloads** are intentionally not persisted.
//!   Block 41 records canonical encoded-frame hashes plus bounded metadata, and
//!   audio payload length/SHA-256, but never the raw frame bytes or audio bytes.
//!   Packet observations and the actual pass/drop/hold/release decisions share
//!   one bounded monotonically ordered fact stream (`ScenarioTrace::transport_trace`).
//! - **A conversion tool for an older, structurally incompatible
//!   recording** (Block 41.2 "support conversion only through an explicit
//!   versioned future tool"): no such tool exists, and this module contains
//!   no code path that reinterprets an incompatible
//!   `recordingFormatVersion` -- [`load_recording_json`] and
//!   [`super::replay::replay`] only ever reject a mismatch. That absence
//!   *is* the deliberate design: a real conversion tool, if ever needed,
//!   would be new, explicitly versioned code added later, not a silent
//!   branch inside this loader.

use super::fault::trace::{MAX_TRANSPORT_FACTS, TransportFact};
use super::recorder::{MAX_RECORDED_NOTIFICATIONS, RecordedNotification};
use super::scenario::{
    AssertionResult, ClockAdvance, MAX_ASSERTIONS, MAX_NODES, MAX_STEPS, Scenario, ScenarioOutcome,
    ScenarioReport, ScenarioTrace, StepResult,
};
use serde::{Deserialize, Serialize};
use silent_disco_core::CoreVersion;
use silent_disco_core::protocol::PROTOCOL_VERSION;
use std::fmt;
use std::path::Path;

/// On-disk shape version for [`ScenarioRecording`] itself, distinct from a
/// scenario document's own `schemaVersion` -- the persisted recording
/// format can evolve independently of the scenario schema it wraps.
pub(crate) const RECORDING_FORMAT_VERSION: u32 = 2;

/// Hard cap on a serialized recording's byte length (Block 41.3 "bounded
/// output"), checked both before writing ([`ScenarioRecording::to_bounded_json`])
/// and before parsing untrusted bytes ([`load_recording_json`]) -- mirrors
/// `scenario::MAX_SCENARIO_FILE_BYTES`'s "check the raw byte length first"
/// discipline. Larger than the scenario file bound since a recording also
/// carries every node's own bounded notification trace and clock-advance
/// log on top of the scenario's own bounded shape.
pub(crate) const MAX_RECORDING_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Bound on the clock-advance trace: at most one advance per step, plus one
/// final advance to `timeoutMs`.
pub(crate) const MAX_CLOCK_ADVANCES: usize = MAX_STEPS + 1;

/// ABI-independent shared-core version, mirrored locally (rather than
/// reusing [`CoreVersion`] directly) so this module never needs to depend
/// on the shared core crate gaining `Serialize`/`Deserialize` derives it
/// does not otherwise need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecordedCoreVersion {
    pub(crate) major: u16,
    pub(crate) minor: u16,
    pub(crate) patch: u16,
}

impl From<CoreVersion> for RecordedCoreVersion {
    fn from(version: CoreVersion) -> Self {
        Self {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
        }
    }
}

impl fmt::Display for RecordedCoreVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A complete, versioned, bounded, redacted recording of one scenario run
/// (Block 41.1), the unit [`super::replay::replay`] checks and re-executes
/// against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenarioRecording {
    pub(crate) recording_format_version: u32,
    pub(crate) scenario_schema_version: u32,
    /// Informational only -- see this module's own doc comment, "Which
    /// versions gate replay".
    pub(crate) protocol_version: u16,
    /// Informational only -- see this module's own doc comment, "Which
    /// versions gate replay".
    pub(crate) core_version: RecordedCoreVersion,
    pub(crate) seed: u64,
    pub(crate) report: ScenarioReport,
    pub(crate) trace: ScenarioTrace,
}

impl ScenarioRecording {
    /// Captures a completed run's report and trace, stamped with the
    /// current recording format, protocol, and core versions.
    #[must_use]
    pub(crate) fn capture(
        scenario: &Scenario,
        report: ScenarioReport,
        trace: ScenarioTrace,
    ) -> Self {
        Self {
            recording_format_version: RECORDING_FORMAT_VERSION,
            scenario_schema_version: scenario.schema_version,
            protocol_version: PROTOCOL_VERSION,
            core_version: RecordedCoreVersion::from(silent_disco_core::core_version()),
            seed: scenario.seed,
            report,
            trace,
        }
    }

    fn validate(&self) -> Result<(), RecordingValidationError> {
        if self.trace.node_notifications.len() > MAX_NODES {
            return Err(RecordingValidationError::TooMany {
                field: "trace.nodeNotifications",
                limit: MAX_NODES,
            });
        }
        for (_, entries) in &self.trace.node_notifications {
            if entries.len() > MAX_RECORDED_NOTIFICATIONS {
                return Err(RecordingValidationError::TooMany {
                    field: "trace.nodeNotifications[].entries",
                    limit: MAX_RECORDED_NOTIFICATIONS,
                });
            }
        }
        if self.trace.clock_advances.len() > MAX_CLOCK_ADVANCES {
            return Err(RecordingValidationError::TooMany {
                field: "trace.clockAdvances",
                limit: MAX_CLOCK_ADVANCES,
            });
        }
        if self.trace.transport_trace.facts.len() > MAX_TRANSPORT_FACTS {
            return Err(RecordingValidationError::TooMany {
                field: "trace.transportTrace.facts",
                limit: MAX_TRANSPORT_FACTS,
            });
        }
        if self.report.step_results.len() > MAX_STEPS {
            return Err(RecordingValidationError::TooMany {
                field: "report.stepResults",
                limit: MAX_STEPS,
            });
        }
        if self.report.assertion_results.len() > MAX_ASSERTIONS {
            return Err(RecordingValidationError::TooMany {
                field: "report.assertionResults",
                limit: MAX_ASSERTIONS,
            });
        }
        Ok(())
    }

    /// Serializes to bounded JSON bytes (Block 41.3 "bounded output").
    ///
    /// # Errors
    ///
    /// Returns [`RecordingSaveError::Encode`] if serialization itself fails
    /// (only possible for a non-finite float, which no field here ever
    /// holds), or [`RecordingSaveError::TooLarge`] when the encoded form
    /// exceeds [`MAX_RECORDING_FILE_BYTES`].
    pub(crate) fn to_bounded_json(&self) -> Result<Vec<u8>, RecordingSaveError> {
        let bytes = serde_json::to_vec(self).map_err(RecordingSaveError::Encode)?;
        if bytes.len() > MAX_RECORDING_FILE_BYTES {
            return Err(RecordingSaveError::TooLarge {
                limit: MAX_RECORDING_FILE_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(bytes)
    }
}

#[derive(Debug)]
pub(crate) enum RecordingSaveError {
    Encode(serde_json::Error),
    TooLarge { limit: usize, actual: usize },
}

impl fmt::Display for RecordingSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => write!(formatter, "recording could not be encoded: {error}"),
            Self::TooLarge { limit, actual } => write!(
                formatter,
                "recording of {actual} bytes exceeds the bound of {limit} bytes"
            ),
        }
    }
}

impl std::error::Error for RecordingSaveError {}

/// Every way loading a recording document can fail before its contents are
/// trusted (Block 41.1's bound discipline applied to a recording file, the
/// same way `scenario::ScenarioParseError` applies it to a scenario file).
#[derive(Debug)]
pub(crate) enum RecordingLoadError {
    TooLarge { limit: usize },
    Malformed(serde_json::Error),
    Validation(RecordingValidationError),
}

impl fmt::Display for RecordingLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit } => {
                write!(
                    formatter,
                    "recording file exceeds the bound of {limit} bytes"
                )
            }
            Self::Malformed(error) => {
                write!(formatter, "recording file is not valid JSON: {error}")
            }
            Self::Validation(error) => write!(formatter, "recording file is invalid: {error}"),
        }
    }
}

impl std::error::Error for RecordingLoadError {}

#[derive(Debug)]
pub(crate) enum RecordingValidationError {
    TooMany { field: &'static str, limit: usize },
}

impl fmt::Display for RecordingValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany { field, limit } => {
                write!(formatter, "{field} exceeds the bound of {limit}")
            }
        }
    }
}

impl std::error::Error for RecordingValidationError {}

/// Parses and bound-checks raw recording JSON bytes (Block 41.3 "truncated
/// recording rejected", "bounded output"). Unlike
/// `scenario::load_scenario_json`, validation is folded into this single
/// entry point rather than left to the caller -- a recording's sole
/// production consumer ([`super::replay::replay`]) always wants a fully
/// trustworthy value or a reported error, never a partially-checked one.
///
/// # Errors
///
/// Returns [`RecordingLoadError::TooLarge`] for an oversized file,
/// [`RecordingLoadError::Malformed`] for invalid or truncated JSON, or
/// [`RecordingLoadError::Validation`] when the parsed document exceeds a
/// declared bound.
pub(crate) fn load_recording_json(bytes: &[u8]) -> Result<ScenarioRecording, RecordingLoadError> {
    if bytes.len() > MAX_RECORDING_FILE_BYTES {
        return Err(RecordingLoadError::TooLarge {
            limit: MAX_RECORDING_FILE_BYTES,
        });
    }
    let recording: ScenarioRecording =
        serde_json::from_slice(bytes).map_err(RecordingLoadError::Malformed)?;
    recording
        .validate()
        .map_err(RecordingLoadError::Validation)?;
    Ok(recording)
}

#[derive(Debug)]
pub(crate) enum RecordingIoError {
    Save(RecordingSaveError),
    Load(RecordingLoadError),
    Write(std::io::Error),
    Read(std::io::Error),
}

impl fmt::Display for RecordingIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Save(error) => write!(formatter, "{error}"),
            Self::Load(error) => write!(formatter, "{error}"),
            Self::Write(error) => write!(formatter, "recording could not be written: {error}"),
            Self::Read(error) => write!(formatter, "recording could not be read: {error}"),
        }
    }
}

impl std::error::Error for RecordingIoError {}

/// Saves a recording to `path` as bounded JSON (Block 41's own acceptance:
/// "a difficult failure can be saved").
///
/// # Errors
///
/// Returns [`RecordingIoError::Save`] when the recording itself is too
/// large to encode, or [`RecordingIoError::Write`] for a filesystem
/// failure.
pub(crate) fn save_recording_to_path(
    recording: &ScenarioRecording,
    path: &Path,
) -> Result<(), RecordingIoError> {
    let bytes = recording
        .to_bounded_json()
        .map_err(RecordingIoError::Save)?;
    std::fs::write(path, bytes).map_err(RecordingIoError::Write)
}

/// Loads and bound-checks a recording previously saved by
/// [`save_recording_to_path`] (Block 41's own acceptance: "... and replayed
/// [later]").
///
/// # Errors
///
/// Returns [`RecordingIoError::Read`] for a filesystem failure, or
/// [`RecordingIoError::Load`] when the file is oversized, malformed, or
/// exceeds a declared bound.
pub(crate) fn load_recording_from_path(path: &Path) -> Result<ScenarioRecording, RecordingIoError> {
    let bytes = std::fs::read(path).map_err(RecordingIoError::Read)?;
    load_recording_json(&bytes).map_err(RecordingIoError::Load)
}

include!("recording/divergence.rs");

#[cfg(test)]
mod tests;
