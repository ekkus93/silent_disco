//! Accept-time routing on a real host: TLS admission, the join that reaches the
//! host actor, refusals, and the visible reason for each refused connection.

use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, JoinApproval, ProtocolFrame, encode_frame};
use p256::pkcs8::EncodePrivateKey;

use crate::transport::secure_admission::dial_and_prove;
use crate::transport::secure_control_tls::host_tls_identity_from_pkcs8;
use crate::transport::secure_listener_proof::{ListenerProofSigner, ProofExchangeError};
use crate::transport::test_support::join_request;
use crate::transport::{HostTransportNode, TransportChannel, TransportErrorKind, TransportEvent};

use super::SocketHostTransport;
use super::secure_test_support::{
    control_address, host_with_secure, is_join_frame, join_over_tls, key, listener_device, spki_of,
    wait_for_event,
};
use crate::transport::{HostTransportConfig, ManualTransportClock};
use std::sync::Arc;

struct ForgingSigner {
    claimed: Vec<u8>,
    actual: p256::ecdsa::SigningKey,
}

impl ListenerProofSigner for ForgingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Ok(self.claimed.clone())
    }
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.actual.sign_der(transcript)
    }
}

#[test]
fn an_admitted_tls_listener_joins_the_host_actor_and_receives_the_approval_on_the_same_channel() {
    let session = SessionId::new("dispatch-join").expect("session");
    let (mut host, host_spki) = host_with_secure(41, &session);
    let listener_id = listener_device(42);

    let listener = join_over_tls(&host, &host_spki, &session, &listener_id, 42);
    let joined = wait_for_event(&mut host, is_join_frame);
    let TransportEvent::FrameReceived { peer, .. } = joined else {
        panic!("join was not a frame event");
    };
    assert_eq!(peer.device_id.as_ref(), Some(&listener_id));

    host.send_pending_control(
        &listener_id,
        &ControlMessage::JoinApproval(JoinApproval {
            session_id: session.clone(),
            listener_id: listener_id.clone(),
            trusted_for_future: false,
        }),
    )
    .expect("approval sent");
    let mut listener = listener;
    let reply = listener
        .recv_frame(Duration::from_secs(10))
        .expect("approval received");
    assert!(
        matches!(
            &reply,
            ProtocolFrame::Control(ControlMessage::JoinApproval(approval))
                if approval.listener_id == listener_id
        ),
        "the reply was not the join approval"
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_listener_frame_for_another_session_is_refused_and_its_peer_is_disconnected() {
    let session = SessionId::new("dispatch-foreign").expect("session");
    let (mut host, host_spki) = host_with_secure(49, &session);
    let other = SessionId::new("some-other-session").expect("session");
    let channel = dial_and_prove(control_address(&host), &host_spki, &session, 1, &key(50))
        .expect("listener proved");
    let listener = crate::transport::secure_control_session::SecureControlSession::new(
        channel,
        session.clone(),
    );

    let foreign = ProtocolFrame::Control(join_request(
        &other,
        &DeviceId::new("foreign-listener").expect("device"),
        "Foreign",
    ));
    listener
        .raw_send_for_test(encode_frame(&foreign).expect("encode"))
        .expect("bytes sent");

    let rejected = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                ..
            }
        )
    });
    let TransportEvent::Rejected { error, .. } = rejected else {
        panic!("not a rejection");
    };
    assert_eq!(error.kind, TransportErrorKind::Protocol);
    wait_for_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { .. })
    });
    host.shutdown().expect("shutdown");
}

#[test]
fn an_honest_tls_listener_is_admitted_and_reported_as_a_peer() {
    let session = SessionId::new("dispatch-admit").expect("session");
    let (mut host, host_spki) = host_with_secure(53, &session);
    let listener = dial_and_prove(control_address(&host), &host_spki, &session, 1, &key(54))
        .expect("listener proved");
    wait_for_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerAccepted { .. })
    });
    // The admitted channel is a live peer, not a refusal: no Rejected event
    // arrives for it, and dropping the listener ends it.
    drop(listener);
    wait_for_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { .. })
    });
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_listener_claiming_another_key_is_refused_as_unauthorized() {
    let session = SessionId::new("dispatch-forged").expect("session");
    let (mut host, host_spki) = host_with_secure(43, &session);
    let forger = ForgingSigner {
        claimed: spki_of(44),
        actual: key(45),
    };
    let address = control_address(&host);
    let session_for_client = session.clone();
    let client = std::thread::spawn(move || {
        dial_and_prove(address, &host_spki, &session_for_client, 1, &forger).map(drop)
    });

    let rejected = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                ..
            }
        )
    });
    let TransportEvent::Rejected { error, .. } = rejected else {
        panic!("not a rejection");
    };
    assert_eq!(
        error.kind,
        TransportErrorKind::Unauthorized,
        "message: {}",
        error.message
    );
    let _ = client.join();
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_connection_is_refused_with_a_reason_when_no_secure_identity_is_configured() {
    let session = SessionId::new("dispatch-unconfigured").expect("session");
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        Arc::new(ManualTransportClock::new(0)),
    )
    .expect("bind");
    let address = control_address(&host);
    let pkcs8 = key(46).to_pkcs8_der().expect("pkcs8").as_bytes().to_vec();
    let host_spki = host_tls_identity_from_pkcs8(&pkcs8).expect("identity").spki;
    let session_for_client = session.clone();
    let client = std::thread::spawn(move || {
        dial_and_prove(address, &host_spki, &session_for_client, 1, &key(47)).map(drop)
    });

    let rejected = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                ..
            }
        )
    });
    let TransportEvent::Rejected { error, .. } = rejected else {
        panic!("not a rejection");
    };
    assert_eq!(error.kind, TransportErrorKind::Protocol);
    assert!(
        error.message.contains("not configured"),
        "unexpected message: {}",
        error.message
    );
    assert!(
        client.join().expect("client thread").is_err(),
        "the listener must not be admitted"
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn a_silent_or_malformed_connection_is_left_to_the_plaintext_path_not_refused_here() {
    let session = SessionId::new("dispatch-legacy").expect("session");
    let (mut host, _) = host_with_secure(48, &session);
    let mut garbage = TcpStream::connect(control_address(&host)).expect("connect");
    garbage.write_all(b"GET / HTTP/1.1\r\n\r\n").expect("send");
    let _silent = TcpStream::connect(control_address(&host)).expect("connect silent");

    // Neither connection gets a TLS refusal from dispatch: the plaintext path
    // owns them, and its own decoder and timers decide what happens next.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(TransportEvent::Rejected { error, .. }) =
            host.recv_event(Duration::from_millis(100))
        {
            assert!(
                !error.message.contains("TLS") && !error.message.contains("secure control"),
                "dispatch refused a non-TLS connection: {}",
                error.message
            );
        }
    }
    host.shutdown().expect("shutdown");
}

#[test]
fn a_verified_tls_listener_claiming_another_device_id_is_refused_before_the_actor() {
    let session = SessionId::new("dispatch-claim").expect("session");
    let (mut host, host_spki) = host_with_secure(47, &session);
    // The key proves as seed 48, but the join claims seed 49's device ID.
    let claimed = listener_device(49);
    let _listener = join_over_tls(&host, &host_spki, &session, &claimed, 48);

    let rejected = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                ..
            } | TransportEvent::PeerDisconnected { .. }
        )
    });
    if let TransportEvent::Rejected { error, .. } = &rejected {
        assert_eq!(
            error.kind,
            TransportErrorKind::Unauthorized,
            "{}",
            error.message
        );
    }
    // The claim must never reach the host actor as a join.
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if let Ok(event) = host.recv_event(Duration::from_millis(50)) {
            assert!(
                !is_join_frame(&event),
                "a claimed device ID must not reach the actor"
            );
        }
    }
}
