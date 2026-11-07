//! TLS 1.3 control-channel configuration pinned to the host's public key.
//!
//! The host's identity is the P-256 key shown in its signed invitation. There
//! is no certificate authority: the app works offline, so no public root can
//! vouch for a host. Instead the listener pins the host's `SubjectPublicKeyInfo`
//! (SPKI) from the invitation, and the handshake is accepted only if the server
//! proves possession of the matching private key.
//!
//! How the proof works: in TLS 1.3 the server signs the handshake transcript
//! (`CertificateVerify`) with its private key. The verifier checks that
//! signature against the pinned SPKI and rejects a signature from any other key.
//! The certificate the server presents is only a carrier for the key, so the
//! verifier does not validate its chain or names. Only a holder of the pinned
//! private key can complete the handshake, and the transcript binds the
//! session keys to that proof.
//!
//! Scope: this module configures TLS only. It does not decide which listener
//! is trusted (that is the host's durable trust policy, `secure_trust.rs`), and
//! it does not yet authenticate the listener to the host (client certificates
//! are disabled here). Protocol v3 plaintext is not replaced by this module
//! until the transport is wired in.

use std::sync::Arc;

use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, ServerConfig, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature_with_raw_key},
    pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime},
    server::{ClientHello, ResolvesServerCert},
    sign::{CertifiedKey, SigningKey},
    version::TLS13,
};

/// Name placed in SNI. The pinned key, not the name, establishes identity.
pub const HOST_SERVER_NAME: &str = "silent-disco-host";

/// Pins the host's SPKI and accepts the handshake only when the server signs
/// the transcript with the matching private key.
#[derive(Debug)]
struct PinnedHostKeyVerifier {
    host_spki: Vec<u8>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedHostKeyVerifier {
    /// The chain and names are intentionally not checked; the signature check
    /// below is what authenticates the server. See the module documentation.
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        _cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(self.host_spki.as_slice()),
            dss,
            &self.algorithms,
        )
    }

    /// TLS 1.2 is not configured on either side, so a TLS 1.2 signature is a
    /// protocol violation and is refused outright.
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General(
            "TLS 1.2 signatures are refused on the control channel".to_owned(),
        ))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Serves one fixed certificate and signing key. The key is any rustls
/// [`SigningKey`], so a platform-protected key can sign without being exported.
#[derive(Debug)]
struct FixedCertificate(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCertificate {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// Client configuration that accepts only the server holding `host_spki`.
///
/// # Errors
/// Returns a rustls error if the provider cannot offer TLS 1.3.
pub fn pinned_client_config(host_spki: &[u8]) -> Result<Arc<ClientConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinnedHostKeyVerifier {
        host_spki: host_spki.to_vec(),
        algorithms: provider.signature_verification_algorithms,
    };
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Host (server) configuration that presents `certificate` and signs with
/// `key`. Client certificates are not requested.
///
/// # Errors
/// Returns a rustls error if the provider cannot offer TLS 1.3 or the key does
/// not match the certificate.
pub fn host_server_config(
    certificate: CertificateDer<'static>,
    key: Arc<dyn SigningKey>,
) -> Result<Arc<ServerConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let certified = CertifiedKey::new(vec![certificate], key);
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&TLS13])?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FixedCertificate(Arc::new(certified))));
    Ok(Arc::new(config))
}
