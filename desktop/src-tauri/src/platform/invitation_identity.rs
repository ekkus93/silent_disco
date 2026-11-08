//! Per-profile P-256 signing identity for desktop-issued QR invitations
//! (Block 31.1). Separate from [`super::identity::DesktopIdentity`], which
//! derives an opaque symmetric [`DeviceId`](silent_disco_core::domain::DeviceId)
//! -- this identity is asymmetric and its public half is meant to be handed
//! out (embedded in every signed invitation), mirroring Android's own
//! Keystore-backed `HostIdentityManager`. The private key never crosses
//! Tauri IPC: only [`DesktopHostSigningIdentity::public_key_der`] and
//! [`DesktopHostSigningIdentity::sign_base64url`]'s output (a signature, not
//! the key) ever leave this module.

use crate::profile::ProfileId;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::{Entry, Error as KeyringError};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
};
use silent_disco_core::transport::secure_control_tls::{
    HostTlsIdentity, host_tls_identity_from_pkcs8,
};
use std::fmt;

const KEYRING_SERVICE: &str = "com.ekkus.silent-disco.desktop";
const SIGNING_ACCOUNT_PREFIX: &str = "profile-invitation-signing-v1:";
/// Separate from the invitation key: the TLS key authenticates the control
/// channel, and its public half is signed into each invitation.
const TLS_ACCOUNT_PREFIX: &str = "profile-host-tls-v1:";
/// Retries against an astronomically unlikely (~2^-128 per attempt) invalid
/// P-256 scalar from a healthy CSPRNG -- not a realistic failure path, but
/// failing closed after bounded retries is still better than an unbounded
/// loop if the source of randomness were ever degraded.
const MAX_KEY_GENERATION_ATTEMPTS: u32 = 8;

/// A profile's persistent host keys: the invitation-signing key and the TLS
/// control-channel key. Private scalars live only in this struct and the OS
/// credential store they were loaded from.
#[derive(Clone)]
pub struct DesktopHostSigningIdentity {
    signing_key: SigningKey,
    public_key_der: Vec<u8>,
    tls_signing_key: SigningKey,
    tls_public_key_der: Vec<u8>,
}

impl DesktopHostSigningIdentity {
    #[must_use]
    pub fn public_key_der(&self) -> &[u8] {
        &self.public_key_der
    }

    /// SPKI DER of the TLS control-channel key. Signed into each invitation.
    #[must_use]
    pub fn tls_public_key_der(&self) -> &[u8] {
        &self.tls_public_key_der
    }

    /// Signs `message` and returns a `DER`-encoded `ECDSA` signature,
    /// base64url-encoded -- the exact shape [`silent_disco_core::p2::finalize_qr`]
    /// expects.
    #[must_use]
    pub fn sign_base64url(&self, message: &[u8]) -> String {
        let signature: Signature = self.signing_key.sign(message);
        URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes())
    }

    /// Builds the TLS identity this host presents on its control port from the
    /// TLS key. The PKCS#8 copy exists only for this call and is zeroed
    /// afterwards; it never leaves the process.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopHostSigningIdentityError::HostTlsIdentity`] when the
    /// key cannot be encoded or the TLS identity cannot be built from it.
    pub fn host_tls_identity(&self) -> Result<HostTlsIdentity, DesktopHostSigningIdentityError> {
        let mut pkcs8 = self
            .tls_signing_key
            .to_pkcs8_der()
            .map_err(|_| DesktopHostSigningIdentityError::EncodePrivateKey)?
            .as_bytes()
            .to_vec();
        let identity = host_tls_identity_from_pkcs8(&pkcs8)
            .map_err(|error| DesktopHostSigningIdentityError::HostTlsIdentity(error.to_string()));
        pkcs8.fill(0);
        identity
    }

    fn from_keys(
        signing_key: SigningKey,
        tls_signing_key: SigningKey,
    ) -> Result<Self, DesktopHostSigningIdentityError> {
        let public_key_der = encode_public_key(&signing_key)?;
        let tls_public_key_der = encode_public_key(&tls_signing_key)?;
        Ok(Self {
            signing_key,
            public_key_der,
            tls_signing_key,
            tls_public_key_der,
        })
    }
}

/// Source of a profile's persistent host-signing identity.
pub trait DesktopHostSigningIdentityProvider: Send + Sync {
    /// Loads or creates the signing identity for one profile.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed error when secure storage is unavailable, locked,
    /// malformed, or cannot persist a newly generated key.
    fn load_or_create(
        &self,
        profile_id: &ProfileId,
    ) -> Result<DesktopHostSigningIdentity, DesktopHostSigningIdentityError>;
}

/// Production provider backed by the operating-system credential store.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemDesktopHostSigningIdentityProvider;

impl DesktopHostSigningIdentityProvider for SystemDesktopHostSigningIdentityProvider {
    fn load_or_create(
        &self,
        profile_id: &ProfileId,
    ) -> Result<DesktopHostSigningIdentity, DesktopHostSigningIdentityError> {
        let signing_key =
            load_or_create_key(&format!("{SIGNING_ACCOUNT_PREFIX}{}", profile_id.as_str()))?;
        let tls_signing_key =
            load_or_create_key(&format!("{TLS_ACCOUNT_PREFIX}{}", profile_id.as_str()))?;
        DesktopHostSigningIdentity::from_keys(signing_key, tls_signing_key)
    }
}

/// Loads one persistent P-256 key from the credential store under `account`,
/// generating and verifying it on first use. Fails closed on any storage error.
fn load_or_create_key(account: &str) -> Result<SigningKey, DesktopHostSigningIdentityError> {
    let entry = Entry::new(KEYRING_SERVICE, account)
        .map_err(DesktopHostSigningIdentityError::OpenCredentialStore)?;
    let der = match entry.get_secret() {
        Ok(der) => der,
        Err(KeyringError::NoEntry) => {
            let signing_key = generate_signing_key()?;
            let mut generated = signing_key
                .to_pkcs8_der()
                .map_err(|_| DesktopHostSigningIdentityError::EncodePrivateKey)?
                .as_bytes()
                .to_vec();
            if let Err(error) = entry.set_secret(&generated) {
                generated.fill(0);
                return Err(DesktopHostSigningIdentityError::PersistSecret(error));
            }
            match entry.get_secret() {
                Ok(stored) => {
                    generated.fill(0);
                    stored
                }
                Err(error) => {
                    generated.fill(0);
                    return Err(DesktopHostSigningIdentityError::VerifyPersistedSecret(
                        error,
                    ));
                }
            }
        }
        Err(error) => return Err(DesktopHostSigningIdentityError::ReadSecret(error)),
    };

    SigningKey::from_pkcs8_der(&der).map_err(|_| DesktopHostSigningIdentityError::InvalidStoredKey)
}

fn encode_public_key(key: &SigningKey) -> Result<Vec<u8>, DesktopHostSigningIdentityError> {
    key.verifying_key()
        .to_public_key_der()
        .map_err(|_| DesktopHostSigningIdentityError::EncodePublicKey)
        .map(|der| der.as_bytes().to_vec())
}

/// Generates a fresh P-256 signing key from CSPRNG bytes, retrying on the
/// vanishingly unlikely event of an out-of-range scalar.
fn generate_signing_key() -> Result<SigningKey, DesktopHostSigningIdentityError> {
    for _ in 0..MAX_KEY_GENERATION_ATTEMPTS {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(DesktopHostSigningIdentityError::GenerateSecret)?;
        let candidate = SigningKey::from_bytes((&bytes).into());
        bytes.fill(0);
        if let Ok(key) = candidate {
            return Ok(key);
        }
    }
    Err(DesktopHostSigningIdentityError::KeyGenerationExhausted)
}

/// Failure while opening or deriving the production desktop signing identity.
#[derive(Debug)]
pub enum DesktopHostSigningIdentityError {
    OpenCredentialStore(KeyringError),
    ReadSecret(KeyringError),
    GenerateSecret(getrandom::Error),
    KeyGenerationExhausted,
    PersistSecret(KeyringError),
    VerifyPersistedSecret(KeyringError),
    InvalidStoredKey,
    EncodePrivateKey,
    EncodePublicKey,
    HostTlsIdentity(String),
}

impl fmt::Display for DesktopHostSigningIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenCredentialStore(error) => write!(
                formatter,
                "could not open the operating-system credential store: {error}"
            ),
            Self::ReadSecret(error) => write!(
                formatter,
                "could not read the host signing key from secure storage: {error}"
            ),
            Self::GenerateSecret(error) => {
                write!(formatter, "could not generate a host signing key: {error}")
            }
            Self::KeyGenerationExhausted => {
                formatter.write_str("could not generate a valid host signing key")
            }
            Self::PersistSecret(error) => write!(
                formatter,
                "could not persist the host signing key in secure storage: {error}"
            ),
            Self::VerifyPersistedSecret(error) => write!(
                formatter,
                "host signing key was written but could not be verified from secure storage: {error}"
            ),
            Self::InvalidStoredKey => {
                formatter.write_str("secure storage held a malformed host signing key")
            }
            Self::EncodePrivateKey => {
                formatter.write_str("could not encode the host signing key for storage")
            }
            Self::EncodePublicKey => {
                formatter.write_str("could not encode the host signing public key")
            }
            Self::HostTlsIdentity(reason) => {
                write!(formatter, "could not build the host TLS identity: {reason}")
            }
        }
    }
}

impl std::error::Error for DesktopHostSigningIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenCredentialStore(source)
            | Self::ReadSecret(source)
            | Self::PersistSecret(source)
            | Self::VerifyPersistedSecret(source) => Some(source),
            Self::GenerateSecret(_)
            | Self::KeyGenerationExhausted
            | Self::InvalidStoredKey
            | Self::EncodePrivateKey
            | Self::EncodePublicKey
            | Self::HostTlsIdentity(_) => None,
        }
    }
}

/// Test-only helpers for building a [`DesktopHostSigningIdentity`] without
/// touching a real OS credential store -- used by this module's own tests
/// and by `invitation.rs`'s tests, which need a real, working identity to
/// sign against but must not depend on `keyring` succeeding in a sandboxed
/// test environment.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::{DesktopHostSigningIdentity, SigningKey};

    #[must_use]
    pub(crate) fn fixed_identity_for_tests(seed: u8) -> DesktopHostSigningIdentity {
        let signing_key = SigningKey::from_bytes((&[seed; 32]).into()).expect("fixed test key");
        // A distinct TLS key, so tests can tell the two apart.
        let tls_key = SigningKey::from_bytes((&[seed.wrapping_add(100); 32]).into())
            .expect("fixed TLS test key");
        DesktopHostSigningIdentity::from_keys(signing_key, tls_key).expect("build test identity")
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopHostSigningIdentity, generate_signing_key};
    use p256::ecdsa::{VerifyingKey, signature::Verifier};
    use p256::pkcs8::DecodePublicKey;

    #[test]
    fn a_generated_key_signs_and_its_own_public_key_verifies() {
        let signing_key = generate_signing_key().expect("generate key");
        let identity = DesktopHostSigningIdentity::from_keys(
            signing_key,
            generate_signing_key().expect("tls key"),
        )
        .expect("build identity");
        let signature_base64url = identity.sign_base64url(b"hello invitation");
        let signature_der = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            signature_base64url,
        )
        .expect("valid base64url");
        let signature =
            p256::ecdsa::Signature::from_der(&signature_der).expect("valid DER signature");
        let verifying_key = VerifyingKey::from_public_key_der(identity.public_key_der())
            .expect("valid public key DER");
        verifying_key
            .verify(b"hello invitation", &signature)
            .expect("signature verifies against its own public key");
    }

    #[test]
    fn two_generated_keys_are_different() {
        let first = generate_signing_key().expect("first key");
        let second = generate_signing_key().expect("second key");
        assert_ne!(first.to_bytes(), second.to_bytes());
    }

    #[test]
    fn a_signature_does_not_verify_against_a_different_key() {
        let identity_a = DesktopHostSigningIdentity::from_keys(
            generate_signing_key().expect("key a"),
            generate_signing_key().expect("tls a"),
        )
        .expect("identity a");
        let identity_b = DesktopHostSigningIdentity::from_keys(
            generate_signing_key().expect("key b"),
            generate_signing_key().expect("tls b"),
        )
        .expect("identity b");
        let signature_base64url = identity_a.sign_base64url(b"message");
        let signature_der = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            signature_base64url,
        )
        .expect("valid base64url");
        let signature =
            p256::ecdsa::Signature::from_der(&signature_der).expect("valid DER signature");
        let wrong_key = VerifyingKey::from_public_key_der(identity_b.public_key_der())
            .expect("valid public key DER");
        assert!(wrong_key.verify(b"message", &signature).is_err());
    }
}

#[cfg(test)]
mod host_tls_identity_tests {
    use super::tests_support::fixed_identity_for_tests;

    #[test]
    fn host_tls_identity_pins_the_tls_key_the_invitation_carries() {
        let identity = fixed_identity_for_tests(7);
        let tls = identity.host_tls_identity().expect("tls identity");
        // A listener pins the invitation's TLS key; the handshake must prove it.
        assert_eq!(tls.spki, identity.tls_public_key_der());
    }

    #[test]
    fn the_tls_key_is_separate_from_the_invitation_signing_key() {
        let identity = fixed_identity_for_tests(7);
        assert_ne!(identity.tls_public_key_der(), identity.public_key_der());
    }

    #[test]
    fn host_tls_identity_is_stable_for_one_key() {
        let identity = fixed_identity_for_tests(9);
        let first = identity.host_tls_identity().expect("first");
        let second = identity.host_tls_identity().expect("second");
        assert_eq!(first.spki, second.spki);
    }
}
