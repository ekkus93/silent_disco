use crate::profile::ProfileId;
use keyring::{Entry, Error as KeyringError};
use sha2::{Digest, Sha256};
use silent_disco_core::domain::DeviceId;
use std::fmt;

const KEYRING_SERVICE: &str = "com.ekkus.silent-disco.desktop";
const IDENTITY_SECRET_BYTES: usize = 32;
const IDENTITY_ACCOUNT_PREFIX: &str = "profile-identity-v1:";

/// Public identity material retained by the desktop runtime.
///
/// The secret used to derive this identifier never leaves the operating-system
/// credential store and is never returned through Tauri IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopIdentity {
    device_id: DeviceId,
}

impl DesktopIdentity {
    #[must_use]
    pub const fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub(crate) fn from_secret(secret: &[u8]) -> Result<Self, DesktopIdentityError> {
        if secret.len() != IDENTITY_SECRET_BYTES {
            return Err(DesktopIdentityError::InvalidStoredSecretLength {
                found: secret.len(),
                expected: IDENTITY_SECRET_BYTES,
            });
        }

        let digest = Sha256::digest(secret);
        let mut encoded = String::with_capacity(8 + digest.len() * 2);
        encoded.push_str("desktop-");
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut encoded, "{byte:02x}")
                .map_err(|_| DesktopIdentityError::IdentifierEncodingFailed)?;
        }
        let device_id =
            DeviceId::new(encoded).map_err(|_| DesktopIdentityError::DerivedIdentifierInvalid)?;
        Ok(Self { device_id })
    }
}

/// Source of persistent desktop identity material.
pub trait DesktopIdentityProvider: Send + Sync {
    /// Loads or creates the identity for one profile.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed error when secure storage is unavailable, locked,
    /// malformed, or cannot persist a newly generated secret.
    fn load_or_create(
        &self,
        profile_id: &ProfileId,
    ) -> Result<DesktopIdentity, DesktopIdentityError>;
}

/// Production provider backed by the operating-system credential store.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemDesktopIdentityProvider;

impl DesktopIdentityProvider for SystemDesktopIdentityProvider {
    fn load_or_create(
        &self,
        profile_id: &ProfileId,
    ) -> Result<DesktopIdentity, DesktopIdentityError> {
        let account = format!("{IDENTITY_ACCOUNT_PREFIX}{}", profile_id.as_str());
        let entry = Entry::new(KEYRING_SERVICE, &account)
            .map_err(DesktopIdentityError::OpenCredentialStore)?;

        let mut secret = match entry.get_secret() {
            Ok(secret) => secret,
            Err(KeyringError::NoEntry) => {
                let mut generated = vec![0_u8; IDENTITY_SECRET_BYTES];
                getrandom::fill(&mut generated).map_err(DesktopIdentityError::GenerateSecret)?;
                if let Err(error) = entry.set_secret(&generated) {
                    generated.fill(0);
                    return Err(DesktopIdentityError::PersistSecret(error));
                }
                match entry.get_secret() {
                    Ok(stored) => {
                        generated.fill(0);
                        stored
                    }
                    Err(error) => {
                        generated.fill(0);
                        return Err(DesktopIdentityError::VerifyPersistedSecret(error));
                    }
                }
            }
            Err(error) => return Err(DesktopIdentityError::ReadSecret(error)),
        };

        let identity = DesktopIdentity::from_secret(&secret);
        secret.fill(0);
        identity
    }
}

/// Failure while opening or deriving the production desktop identity.
#[derive(Debug)]
pub enum DesktopIdentityError {
    OpenCredentialStore(KeyringError),
    ReadSecret(KeyringError),
    GenerateSecret(getrandom::Error),
    PersistSecret(KeyringError),
    VerifyPersistedSecret(KeyringError),
    InvalidStoredSecretLength { found: usize, expected: usize },
    IdentifierEncodingFailed,
    DerivedIdentifierInvalid,
}

impl fmt::Display for DesktopIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenCredentialStore(error) => {
                write!(
                    formatter,
                    "could not open the operating-system credential store: {error}"
                )
            }
            Self::ReadSecret(error) => {
                write!(
                    formatter,
                    "could not read desktop identity from secure storage: {error}"
                )
            }
            Self::GenerateSecret(error) => {
                write!(
                    formatter,
                    "could not generate desktop identity secret: {error}"
                )
            }
            Self::PersistSecret(error) => {
                write!(
                    formatter,
                    "could not persist desktop identity in secure storage: {error}"
                )
            }
            Self::VerifyPersistedSecret(error) => write!(
                formatter,
                "desktop identity was written but could not be verified from secure storage: {error}"
            ),
            Self::InvalidStoredSecretLength { found, expected } => write!(
                formatter,
                "secure desktop identity has invalid length {found}; expected {expected} bytes"
            ),
            Self::IdentifierEncodingFailed => {
                formatter.write_str("could not encode the public desktop device identifier")
            }
            Self::DerivedIdentifierInvalid => {
                formatter.write_str("secure desktop identity produced an invalid device identifier")
            }
        }
    }
}

impl std::error::Error for DesktopIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenCredentialStore(source)
            | Self::ReadSecret(source)
            | Self::PersistSecret(source)
            | Self::VerifyPersistedSecret(source) => Some(source),
            Self::GenerateSecret(_)
            | Self::InvalidStoredSecretLength { .. }
            | Self::IdentifierEncodingFailed
            | Self::DerivedIdentifierInvalid => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopIdentity, DesktopIdentityError, IDENTITY_SECRET_BYTES};

    #[test]
    fn same_secret_produces_same_public_device_id() {
        let secret = [7_u8; IDENTITY_SECRET_BYTES];
        let first = DesktopIdentity::from_secret(&secret).expect("derive first identity");
        let second = DesktopIdentity::from_secret(&secret).expect("derive second identity");
        assert_eq!(first, second);
        assert!(first.device_id().as_str().starts_with("desktop-"));
        assert!(!first.device_id().as_str().contains("070707"));
    }

    #[test]
    fn different_secrets_produce_different_device_ids() {
        let first =
            DesktopIdentity::from_secret(&[1_u8; IDENTITY_SECRET_BYTES]).expect("first identity");
        let second =
            DesktopIdentity::from_secret(&[2_u8; IDENTITY_SECRET_BYTES]).expect("second identity");
        assert_ne!(first, second);
    }

    #[test]
    fn malformed_stored_secret_is_rejected() {
        assert!(matches!(
            DesktopIdentity::from_secret(&[1_u8; 8]),
            Err(DesktopIdentityError::InvalidStoredSecretLength { .. })
        ));
    }
}
