use super::types::{MAX_SCENARIO_FILE_BYTES, SCHEMA_VERSION, Scenario};
use std::fmt;

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
            Self::UnknownSchemaVersion { found } => write!(
                formatter,
                "unsupported schemaVersion {found}, expected {SCHEMA_VERSION}"
            ),
            Self::Shape(error) => write!(
                formatter,
                "scenario file does not match the schema: {error}"
            ),
        }
    }
}

impl std::error::Error for ScenarioParseError {}

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
