//! Host TLS identity whose private key stays outside this crate.
//!
//! The Android host's control-port key lives in Android Keystore and is never
//! exported. This module lets such a key stand in for the in-process key used
//! by `secure_control_tls::host_tls_identity_from_pkcs8`. The key is reached
//! only through [`ListenerProofSigner`]: its SPKI is what the invitation pins,
//! and each signature is produced by the external signer. The certificate is
//! issued with the same signer, so the SPKI, the certificate and the handshake
//! signature cannot disagree.

use std::fmt;
use std::sync::Arc;

use p256::{ecdsa::VerifyingKey, pkcs8::DecodePublicKey};
use rcgen::{CertificateParams, PKCS_ECDSA_P256_SHA256, PublicKeyData, SignatureAlgorithm};
use rustls::{
    Error as TlsError, SignatureAlgorithm as TlsAlgorithm, SignatureScheme,
    sign::{Signer, SigningKey},
};

use super::secure_control_tls::{HOST_SERVER_NAME, HostTlsIdentity};
use super::secure_listener_proof::ListenerProofSigner;

/// Failure building an external-key host identity.
#[derive(Debug)]
pub struct ExternalKeyError(pub String);

impl fmt::Display for ExternalKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "external host key: {}", self.0)
    }
}

impl std::error::Error for ExternalKeyError {}

/// Builds the host TLS identity from a signer whose private key is held
/// elsewhere (for example Android Keystore).
///
/// # Errors
/// Returns [`ExternalKeyError`] if the signer cannot provide its SPKI, the SPKI
/// is not a P-256 key, or the certificate cannot be issued.
pub fn host_tls_identity_from_external_key(
    signer: Arc<dyn ListenerProofSigner + Send + Sync>,
) -> Result<HostTlsIdentity, ExternalKeyError> {
    let spki = signer
        .spki_der()
        .map_err(|error| ExternalKeyError(format!("SPKI unavailable: {error:?}")))?;
    let verifying = VerifyingKey::from_public_key_der(&spki)
        .map_err(|_| ExternalKeyError("SPKI is not a P-256 public key".to_owned()))?;
    let public_point = verifying.to_encoded_point(false).as_bytes().to_vec();
    let remote = ExternalCertificateKey {
        signer: Arc::clone(&signer),
        public_point,
    };
    let certificate = CertificateParams::new(vec![HOST_SERVER_NAME.to_owned()])
        .map_err(|_| ExternalKeyError("certificate parameters are invalid".to_owned()))?
        .self_signed(&remote)
        .map_err(|error| ExternalKeyError(format!("certificate not issued: {error}")))?
        .der()
        .clone();
    Ok(HostTlsIdentity {
        spki,
        certificate,
        signing_key: Arc::new(ExternalSigningKey { signer }),
    })
}

/// Public point and signing delegate for certificate issuance.
struct ExternalCertificateKey {
    signer: Arc<dyn ListenerProofSigner + Send + Sync>,
    public_point: Vec<u8>,
}

impl PublicKeyData for ExternalCertificateKey {
    fn der_bytes(&self) -> &[u8] {
        &self.public_point
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for ExternalCertificateKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        self.signer
            .sign_der(message)
            .map_err(|_| rcgen::Error::RemoteKeyError)
    }
}

/// rustls key that asks the external signer for each handshake signature.
struct ExternalSigningKey {
    signer: Arc<dyn ListenerProofSigner + Send + Sync>,
}

impl fmt::Debug for ExternalSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalSigningKey")
            .finish_non_exhaustive()
    }
}

impl SigningKey for ExternalSigningKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&SignatureScheme::ECDSA_NISTP256_SHA256)
            .then(|| {
                Box::new(ExternalSigner {
                    signer: Arc::clone(&self.signer),
                }) as Box<dyn Signer>
            })
    }

    fn algorithm(&self) -> TlsAlgorithm {
        TlsAlgorithm::ECDSA
    }
}

struct ExternalSigner {
    signer: Arc<dyn ListenerProofSigner + Send + Sync>,
}

impl fmt::Debug for ExternalSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalSigner")
            .finish_non_exhaustive()
    }
}

impl Signer for ExternalSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, TlsError> {
        // The signer hashes with SHA-256 and returns the DER signature rustls expects.
        self.signer
            .sign_der(message)
            .map_err(|error| TlsError::General(format!("external signer failed: {error:?}")))
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}
