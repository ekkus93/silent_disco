use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Current on-disk profile metadata schema version.
pub const PROFILE_METADATA_SCHEMA_VERSION: u16 = 1;
const MAX_PROFILE_ID_BYTES: usize = 64;
const MAX_PROFILE_DISPLAY_NAME_CHARS: usize = 80;

/// Stable, path-safe identifier for a desktop profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProfileId(String);

impl ProfileId {
    /// Parses a profile identifier without silently normalizing it.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileValidationError`] when the identifier is blank,
    /// oversized, contains unsafe characters, or does not start and end with
    /// an ASCII letter or digit.
    pub fn parse(value: &str) -> Result<Self, ProfileValidationError> {
        if value.is_empty() {
            return Err(ProfileValidationError::InvalidId(
                "profile ID must not be blank",
            ));
        }
        if value.len() > MAX_PROFILE_ID_BYTES {
            return Err(ProfileValidationError::InvalidId(
                "profile ID exceeds 64 bytes",
            ));
        }
        if value.trim() != value {
            return Err(ProfileValidationError::InvalidId(
                "profile ID must not contain leading or trailing whitespace",
            ));
        }

        let first = value
            .chars()
            .next()
            .ok_or(ProfileValidationError::InvalidId(
                "profile ID must not be blank",
            ))?;
        let last = value
            .chars()
            .next_back()
            .ok_or(ProfileValidationError::InvalidId(
                "profile ID must not be blank",
            ))?;

        if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
            return Err(ProfileValidationError::InvalidId(
                "profile ID must start and end with an ASCII letter or digit",
            ));
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        }) {
            return Err(ProfileValidationError::InvalidId(
                "profile ID may contain only lowercase ASCII letters, digits, '-' and '_'",
            ));
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Bounded user-visible profile name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProfileDisplayName(String);

impl ProfileDisplayName {
    /// Parses and trims a user-visible profile name.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileValidationError`] for a blank, oversized, or
    /// control-character-containing name.
    pub fn parse(value: &str) -> Result<Self, ProfileValidationError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ProfileValidationError::InvalidDisplayName(
                "profile display name must not be blank",
            ));
        }
        if trimmed.chars().count() > MAX_PROFILE_DISPLAY_NAME_CHARS {
            return Err(ProfileValidationError::InvalidDisplayName(
                "profile display name exceeds 80 characters",
            ));
        }
        if trimmed.chars().any(char::is_control) {
            return Err(ProfileValidationError::InvalidDisplayName(
                "profile display name must not contain control characters",
            ));
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the validated display name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProfileDisplayName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let parsed = Self::parse(&value).map_err(D::Error::custom)?;
        if parsed.as_str() != value {
            return Err(D::Error::custom(
                "profile display name is not in canonical trimmed form",
            ));
        }
        Ok(parsed)
    }
}

impl fmt::Display for ProfileDisplayName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Versioned metadata stored inside one desktop profile root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProfileMetadata {
    schema_version: u16,
    profile_id: ProfileId,
    display_name: ProfileDisplayName,
}

impl ProfileMetadata {
    /// Creates metadata using the current schema version.
    #[must_use]
    pub fn new(profile_id: ProfileId, display_name: ProfileDisplayName) -> Self {
        Self {
            schema_version: PROFILE_METADATA_SCHEMA_VERSION,
            profile_id,
            display_name,
        }
    }

    /// Returns the metadata schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the profile identifier recorded in the metadata.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    /// Returns the user-visible profile name.
    #[must_use]
    pub const fn display_name(&self) -> &ProfileDisplayName {
        &self.display_name
    }

    /// Validates a deserialized metadata record against the expected profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileValidationError`] when the metadata schema is unsupported
    /// or the record names a different profile.
    pub fn validate_for(&self, expected: &ProfileId) -> Result<(), ProfileValidationError> {
        if self.schema_version != PROFILE_METADATA_SCHEMA_VERSION {
            return Err(ProfileValidationError::UnsupportedMetadataVersion {
                found: self.schema_version,
                supported: PROFILE_METADATA_SCHEMA_VERSION,
            });
        }
        if &self.profile_id != expected {
            return Err(ProfileValidationError::ProfileIdMismatch);
        }
        Ok(())
    }
}

/// Validation failure for profile identifiers and metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileValidationError {
    /// Invalid path-safe profile identifier.
    InvalidId(&'static str),
    /// Invalid user-visible profile name.
    InvalidDisplayName(&'static str),
    /// Metadata uses a schema this build does not understand.
    UnsupportedMetadataVersion { found: u16, supported: u16 },
    /// Metadata belongs to a different profile root.
    ProfileIdMismatch,
}

impl fmt::Display for ProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(message) | Self::InvalidDisplayName(message) => {
                formatter.write_str(message)
            }
            Self::UnsupportedMetadataVersion { found, supported } => write!(
                formatter,
                "profile metadata schema {found} is unsupported; this build supports schema {supported}"
            ),
            Self::ProfileIdMismatch => {
                formatter.write_str("profile metadata identifier does not match the profile root")
            }
        }
    }
}

impl std::error::Error for ProfileValidationError {}

#[cfg(test)]
mod tests {
    use super::{
        PROFILE_METADATA_SCHEMA_VERSION, ProfileDisplayName, ProfileId, ProfileMetadata,
        ProfileValidationError,
    };

    #[test]
    fn accepts_path_safe_profile_id() {
        let id = ProfileId::parse("main-stage_2").expect("valid profile ID");
        assert_eq!(id.as_str(), "main-stage_2");
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_profile_ids() {
        for value in ["", " Main", "main/other", "..", "UPPER", "-main", "main-"] {
            assert!(
                ProfileId::parse(value).is_err(),
                "unexpectedly accepted {value:?}"
            );
        }
        assert!(ProfileId::parse(&"a".repeat(65)).is_err());
    }

    #[test]
    fn deserialize_revalidates_profile_id() {
        assert!(serde_json::from_str::<ProfileId>("\"../escape\"").is_err());
        let id: ProfileId = serde_json::from_str("\"main\"").expect("valid ID JSON");
        assert_eq!(id.as_str(), "main");
    }

    #[test]
    fn preserves_bounded_unicode_display_name() {
        let name = ProfileDisplayName::parse("  Oakland 主机  ").expect("valid name");
        assert_eq!(name.as_str(), "Oakland 主机");
    }

    #[test]
    fn rejects_invalid_display_name() {
        assert!(ProfileDisplayName::parse("   ").is_err());
        assert!(ProfileDisplayName::parse("line\nbreak").is_err());
        assert!(ProfileDisplayName::parse(&"é".repeat(81)).is_err());
    }

    #[test]
    fn deserialize_rejects_noncanonical_display_name() {
        assert!(serde_json::from_str::<ProfileDisplayName>("\" Main \"").is_err());
        let name: ProfileDisplayName =
            serde_json::from_str("\"Oakland 主机\"").expect("valid name JSON");
        assert_eq!(name.as_str(), "Oakland 主机");
    }

    #[test]
    fn metadata_rejects_unsupported_schema_and_wrong_profile() {
        let expected = ProfileId::parse("main").expect("valid ID");
        let other = ProfileId::parse("other").expect("valid ID");
        let name = ProfileDisplayName::parse("Main").expect("valid name");
        let metadata = ProfileMetadata::new(other, name);

        assert_eq!(
            metadata.validate_for(&expected),
            Err(ProfileValidationError::ProfileIdMismatch)
        );

        let json = format!(
            "{{\"schemaVersion\":{},\"profileId\":\"main\",\"displayName\":\"Main\"}}",
            PROFILE_METADATA_SCHEMA_VERSION + 1
        );
        let newer: ProfileMetadata = serde_json::from_str(&json).expect("valid JSON shape");
        assert!(matches!(
            newer.validate_for(&expected),
            Err(ProfileValidationError::UnsupportedMetadataVersion { .. })
        ));
    }
}
