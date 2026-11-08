//! Real TLS 1.3 handshakes against a host identity whose key is only reachable
//! through a `ListenerProofSigner`, as with a non-exportable Keystore key.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer as _},
    pkcs8::EncodePublicKey,
};
use rustls::{ClientConnection, ServerConnection, StreamOwned, pki_types::ServerName};

use super::secure_control_external_key::host_tls_identity_from_external_key;
use super::secure_control_tls::{
    HOST_SERVER_NAME, HostTlsIdentity, host_server_config, pinned_client_config,
};
use super::secure_listener_proof::{ListenerProofSigner, ProofExchangeError};

/// Stands in for Keystore: holds a key and answers only the trait's questions.
struct StandInKeystore {
    /// Key that signs the handshake.
    signing: SigningKey,
    /// Key whose SPKI is advertised. Equal to `signing` unless a test mismatches them.
    advertised: SigningKey,
}

impl ListenerProofSigner for StandInKeystore {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        self.advertised
            .verifying_key()
            .to_public_key_der()
            .map(|der| der.as_bytes().to_vec())
            .map_err(|_| ProofExchangeError::InvalidListenerKey)
    }

    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        let signature: Signature = self.signing.sign(transcript);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("fixture key")
}

fn identity_with(signing: SigningKey, advertised: SigningKey) -> HostTlsIdentity {
    let signer: Arc<dyn ListenerProofSigner + Send + Sync> = Arc::new(StandInKeystore {
        signing,
        advertised,
    });
    host_tls_identity_from_external_key(signer).expect("external-key identity")
}

/// One echo over TLS. A refused handshake returns an error on the client side.
fn echo_over_tls(identity: &HostTlsIdentity, pinned_spki: &[u8]) -> std::io::Result<[u8; 5]> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server_config = host_server_config(
        identity.certificate.clone(),
        Arc::clone(&identity.signing_key),
    )
    .expect("server config");
    let server_thread = thread::spawn(move || {
        let Ok((tcp, _)) = listener.accept() else {
            return;
        };
        tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let Ok(connection) = ServerConnection::new(server_config) else {
            return;
        };
        let mut stream = StreamOwned::new(connection, tcp);
        let mut received = [0_u8; 5];
        if stream.read_exact(&mut received).is_ok() {
            let _ = stream.write_all(&received);
            let _ = stream.flush();
        }
    });

    let client_config = pinned_client_config(pinned_spki).expect("client config");
    let tcp = TcpStream::connect(address)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let name = ServerName::try_from(HOST_SERVER_NAME).expect("name");
    let connection = ClientConnection::new(client_config, name)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut stream = StreamOwned::new(connection, tcp);
    let outcome = (|| -> std::io::Result<[u8; 5]> {
        stream.write_all(b"hello")?;
        stream.flush()?;
        let mut echoed = [0_u8; 5];
        stream.read_exact(&mut echoed)?;
        Ok(echoed)
    })();
    let _ = server_thread.join();
    outcome
}

#[test]
fn an_external_key_host_completes_a_pinned_tls13_handshake() {
    let identity = identity_with(key(71), key(71));
    // The pin is the SPKI the invitation carries, which the signer supplied.
    assert_eq!(
        echo_over_tls(&identity, &identity.spki).expect("handshake and echo"),
        *b"hello"
    );
    assert_eq!(identity.spki, {
        let k = key(71);
        k.verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec()
    });
}

#[test]
fn a_signer_whose_signature_does_not_match_its_advertised_key_is_refused() {
    // Advertises key 72 but signs with key 73: the handshake must not verify.
    let identity = identity_with(key(73), key(72));
    let pinned = identity.spki.clone();
    assert!(
        echo_over_tls(&identity, &pinned).is_err(),
        "a mismatched external signature must not complete the handshake"
    );
}

#[test]
fn a_client_pinned_to_another_key_refuses_an_external_key_host() {
    let identity = identity_with(key(74), key(74));
    let other = identity_with(key(75), key(75));
    assert!(echo_over_tls(&identity, &other.spki).is_err());
}
