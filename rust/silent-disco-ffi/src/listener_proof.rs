//! FFI surface for the listener TLS possession proof.
//!
//! Two sides:
//! - [`listener_prove_to_host`] runs the listener side. The signature is made
//!   by the platform through [`FfiListenerProofSigner`], because on Android the
//!   key lives in Keystore and cannot be exported.
//! - [`FfiProofHostListener`] runs the host side: it binds a TCP port, accepts
//!   one listener and verifies its proof. A host that does not hold the key
//!   cannot complete the handshake or the proof.
//!
//! Neither side is wired into a join flow yet. The host object is the verifier
//! a host will need once join admission uses verified identity.

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use silent_disco_core::domain::SessionId;
use silent_disco_core::transport::secure_control_external_key::host_tls_identity_from_external_key;
use silent_disco_core::transport::secure_control_tls::{
    HostTlsIdentity, host_tls_identity_from_pkcs8,
};
use silent_disco_core::transport::secure_listener_proof::{
    ListenerProofSigner, ProofExchangeError, accept_and_verify, connect_and_prove,
};

use crate::FfiBridgeError;

/// The device ID a listener proving `listener_spki_der` is bound to. A TLS
/// join must claim exactly this ID.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned Vec parameters at the foreign boundary"
)]
pub fn listener_device_id_for_key(listener_spki_der: Vec<u8>) -> String {
    silent_disco_core::transport::secure_identity::device_id_for_listener_key(&listener_spki_der)
        .as_str()
        .to_owned()
}

/// The platform's listener proof key. Implemented by the app over Keystore.
#[uniffi::export(with_foreign)]
pub trait FfiListenerProofSigner: Send + Sync {
    /// `SubjectPublicKeyInfo` DER of the key being proven.
    ///
    /// # Errors
    /// Returns an error if the key cannot be read.
    fn spki_der(&self) -> Result<Vec<u8>, FfiBridgeError>;

    /// DER ECDSA-SHA256 signature over the proof transcript.
    ///
    /// # Errors
    /// Returns an error if the key cannot sign.
    fn sign_der(&self, transcript: Vec<u8>) -> Result<Vec<u8>, FfiBridgeError>;
}

/// Adapts the platform signer to the core trait.
pub(crate) struct CallbackSigner(pub(crate) Arc<dyn FfiListenerProofSigner>);

impl ListenerProofSigner for CallbackSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        self.0
            .spki_der()
            .map_err(|error| ProofExchangeError::SignerFailed(error.to_string()))
    }

    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.0
            .sign_der(transcript.to_vec())
            .map_err(|error| ProofExchangeError::SignerFailed(error.to_string()))
    }
}

/// Runs the listener side of the proof against `address`, pinning `host_spki`.
///
/// # Errors
/// Returns an error if the address or session is malformed, the TLS handshake
/// fails (including a pin mismatch), the platform signer fails, or the exchange
/// is interrupted.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
pub fn listener_prove_to_host(
    address: String,
    host_spki: Vec<u8>,
    session_id: String,
    generation: u64,
    signer: Arc<dyn FfiListenerProofSigner>,
) -> Result<(), FfiBridgeError> {
    let address: SocketAddr = address
        .parse()
        .map_err(|_| FfiBridgeError::Core("host address is malformed".to_owned()))?;
    let session =
        SessionId::new(session_id).map_err(|error| FfiBridgeError::Core(error.to_string()))?;
    connect_and_prove(
        address,
        &host_spki,
        &session,
        generation,
        &CallbackSigner(signer),
    )
    .map_err(|error| FfiBridgeError::Core(error.to_string()))
}

/// The listener identity the host verified, as public fingerprints only.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiVerifiedProofListener {
    /// SHA-256 of the listener's SPKI.
    pub listener_fingerprint: Vec<u8>,
    /// SHA-256 of the host SPKI the listener proved to.
    pub host_fingerprint: Vec<u8>,
}

/// Host side of the proof: one bound port, one accepted listener per call.
#[derive(uniffi::Object)]
pub struct FfiProofHostListener {
    listener: TcpListener,
    identity: HostTlsIdentity,
}

#[uniffi::export]
impl FfiProofHostListener {
    /// Binds `local_address` and prepares the host identity from a PKCS#8
    /// P-256 private key. The SPKI pinned by listeners is derived from that key.
    ///
    /// # Errors
    /// Returns an error if the address cannot be bound or the key is invalid.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn bind(
        local_address: String,
        host_pkcs8_key: Vec<u8>,
    ) -> Result<Arc<Self>, FfiBridgeError> {
        let identity = host_tls_identity_from_pkcs8(&host_pkcs8_key)
            .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        let listener = TcpListener::bind(&local_address)
            .map_err(|error| FfiBridgeError::Core(format!("bind failed: {error}")))?;
        Ok(Arc::new(Self { listener, identity }))
    }

    /// Like [`Self::bind`], but the host key stays on the platform (Android
    /// Keystore): the pinned SPKI and every handshake signature come from `tls_signer`.
    ///
    /// # Errors
    /// Returns an error if the address cannot be bound or the platform key cannot
    /// supply its SPKI or certificate.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn bind_with_platform_key(
        local_address: String,
        tls_signer: Arc<dyn FfiListenerProofSigner>,
    ) -> Result<Arc<Self>, FfiBridgeError> {
        let identity = host_tls_identity_from_external_key(Arc::new(CallbackSigner(tls_signer)))
            .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        let listener = TcpListener::bind(&local_address)
            .map_err(|error| FfiBridgeError::Core(format!("bind failed: {error}")))?;
        Ok(Arc::new(Self { listener, identity }))
    }

    /// The port this host is listening on.
    ///
    /// # Errors
    /// Returns an error if the socket has no local address.
    pub fn local_port(&self) -> Result<u16, FfiBridgeError> {
        self.listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| FfiBridgeError::Core(error.to_string()))
    }

    /// The SPKI a listener must pin to prove to this host.
    #[must_use]
    pub fn host_spki(&self) -> Vec<u8> {
        self.identity.spki.clone()
    }

    /// Accepts one listener, completes TLS 1.3 and verifies its proof for
    /// `session_id` and `generation`. Blocks until a listener connects.
    ///
    /// # Errors
    /// Returns an error if the handshake or the proof fails.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    pub fn accept_and_verify(
        &self,
        session_id: String,
        generation: u64,
    ) -> Result<FfiVerifiedProofListener, FfiBridgeError> {
        let session =
            SessionId::new(session_id).map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        let verified = accept_and_verify(
            &self.listener,
            &self.identity.certificate,
            &self.identity.signing_key,
            &self.identity.spki,
            &session,
            generation,
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        Ok(FfiVerifiedProofListener {
            listener_fingerprint: verified.fingerprint().to_vec(),
            host_fingerprint: verified.host_fingerprint().to_vec(),
        })
    }
}
