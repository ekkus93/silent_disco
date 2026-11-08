//! Desktop listener side of the TLS possession proof.
//!
//! Loads the listener's owner-only key file (see `listener_key_file`) and proves
//! it to a host over a TLS 1.3 connection pinned to the host key from the
//! invitation. The host verifies with `accept_and_verify` in the core.
//!
//! Not yet called from a production join: the desktop listener's join path
//! still runs the protocol v3 plaintext transport.

use std::net::SocketAddr;
use std::path::Path;

use silent_disco_core::domain::SessionId;
use silent_disco_core::transport::secure_listener_proof::{ProofExchangeError, connect_and_prove};

use super::listener_key_file::{ListenerKeyFileError, load_or_create_listener_key};

#[derive(Debug)]
pub enum DesktopListenerProofError {
    Key(ListenerKeyFileError),
    Exchange(ProofExchangeError),
}

impl std::fmt::Display for DesktopListenerProofError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(error) => write!(formatter, "listener key unavailable: {error}"),
            Self::Exchange(error) => write!(formatter, "listener proof failed: {error}"),
        }
    }
}

impl std::error::Error for DesktopListenerProofError {}

/// Proves this desktop's listener key to the host at `address`.
///
/// # Errors
/// Returns `Key` if the key file is missing, unsafe or damaged, and `Exchange`
/// if the TLS handshake (including the host-key pin) or the proof fails.
pub fn prove_to_host(
    key_directory: &Path,
    address: SocketAddr,
    host_spki: &[u8],
    session: &SessionId,
    generation: u64,
) -> Result<(), DesktopListenerProofError> {
    let key = load_or_create_listener_key(key_directory).map_err(DesktopListenerProofError::Key)?;
    connect_and_prove(address, host_spki, session, generation, &key)
        .map_err(DesktopListenerProofError::Exchange)
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::thread;

    use p256::{
        ecdsa::{SigningKey, VerifyingKey},
        pkcs8::{DecodePublicKey, EncodePublicKey},
    };
    use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use silent_disco_core::domain::SessionId;
    use silent_disco_core::transport::secure_control_tls::HOST_SERVER_NAME;
    use silent_disco_core::transport::secure_listener_proof::accept_and_verify;

    use super::{DesktopListenerProofError, prove_to_host};
    use crate::platform::listener_key_file::LISTENER_KEY_FILE_NAME;

    struct HostFixture {
        spki: Vec<u8>,
        certificate: CertificateDer<'static>,
        signing_key: Arc<dyn rustls::sign::SigningKey>,
    }

    fn host_fixture() -> HostFixture {
        let signing = SigningKey::from_slice(&[11_u8; 32]).expect("host key");
        let spki = signing
            .verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec();
        let pkcs8 = {
            use p256::pkcs8::EncodePrivateKey;
            signing.to_pkcs8_der().expect("pkcs8").as_bytes().to_vec()
        };
        let key_pair = KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(pkcs8.as_slice()),
            &PKCS_ECDSA_P256_SHA256,
        )
        .expect("rcgen key");
        let certificate = CertificateParams::new(vec![HOST_SERVER_NAME.to_owned()])
            .expect("params")
            .self_signed(&key_pair)
            .expect("certificate")
            .der()
            .clone();
        let signing_key = rustls::crypto::ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
            PrivatePkcs8KeyDer::from(pkcs8),
        ))
        .expect("signing key");
        HostFixture {
            spki,
            certificate,
            signing_key,
        }
    }

    fn owner_only_directory() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("tempdir");
        let directory = root.path().join("profile");
        fs::create_dir(&directory).expect("directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("mode");
        root
    }

    #[test]
    fn a_desktop_listener_proves_its_key_file_key_to_a_pinned_host() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        let host = host_fixture();
        let session = SessionId::new("desktop-proof-session").expect("session");
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("address");

        let client_directory = directory.clone();
        let client_session = session.clone();
        let host_spki = host.spki.clone();
        let client = thread::spawn(move || {
            prove_to_host(&client_directory, address, &host_spki, &client_session, 1)
        });
        let verified = accept_and_verify(
            &server,
            &host.certificate,
            &host.signing_key,
            &host.spki,
            &session,
            1,
        )
        .expect("the host verifies the desktop proof");
        client
            .join()
            .expect("client thread")
            .expect("desktop proof");

        // The verified key must be the one in the owner-only key file.
        let stored = fs::read(directory.join(LISTENER_KEY_FILE_NAME)).expect("key file");
        let stored_key = SigningKey::from_slice(&stored).expect("stored scalar");
        let stored_spki = stored_key
            .verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec();
        let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&stored_spki).into();
        assert_eq!(verified.fingerprint(), &digest);
        assert!(VerifyingKey::from_public_key_der(&stored_spki).is_ok());
    }

    #[test]
    fn a_desktop_listener_proves_to_a_host_presenting_the_invitation_identity() {
        // The host's TLS identity is built from its invitation key, so the key
        // a listener pins from the invitation is the key the host proves.
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        let signing =
            crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(21);
        let invitation_host_key = signing.public_key_der().to_vec();
        let identity = signing.host_tls_identity().expect("host tls identity");
        let session = SessionId::new("invitation-identity-session").expect("session");
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("address");

        let client_directory = directory.clone();
        let client_session = session.clone();
        let client = thread::spawn(move || {
            prove_to_host(
                &client_directory,
                address,
                &invitation_host_key,
                &client_session,
                1,
            )
        });
        accept_and_verify(
            &server,
            &identity.certificate,
            &identity.signing_key,
            &identity.spki,
            &session,
            1,
        )
        .expect("the host verifies the desktop proof against its invitation identity");
        client
            .join()
            .expect("client thread")
            .expect("desktop proof");
    }

    #[test]
    fn a_desktop_listener_refuses_to_prove_to_a_host_it_did_not_pin() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        let host = host_fixture();
        let other = {
            let signing = SigningKey::from_slice(&[12_u8; 32]).expect("other key");
            signing
                .verifying_key()
                .to_public_key_der()
                .expect("spki")
                .as_bytes()
                .to_vec()
        };
        let session = SessionId::new("unpinned-session").expect("session");
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("address");

        let client_session = session.clone();
        let client =
            thread::spawn(move || prove_to_host(&directory, address, &other, &client_session, 1));
        let host_outcome = accept_and_verify(
            &server,
            &host.certificate,
            &host.signing_key,
            &host.spki,
            &session,
            1,
        );
        let client_outcome = client.join().expect("client thread");
        assert!(
            matches!(client_outcome, Err(DesktopListenerProofError::Exchange(_))),
            "a listener must not prove to a host it did not pin: {client_outcome:?}"
        );
        assert!(host_outcome.is_err());
    }
}
