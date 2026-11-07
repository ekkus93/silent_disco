//! Join-level frame exchange over an admitted TLS control channel.

use std::net::TcpListener;
use std::time::Duration;

use p256::{ecdsa::SigningKey, pkcs8::EncodePrivateKey};

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{
    ControlMessage, DeviceIdentity, JoinApproval, JoinRequest, ProtocolFrame, encode_frame,
};

use super::secure_admission::{SECURE_ADMISSION_GENERATION, admit_listener, dial_and_prove};
use super::secure_control_session::{SecureControlSession, SecureSessionError};
use super::secure_control_tls::host_tls_identity_from_pkcs8;

const WAIT: Duration = Duration::from_secs(10);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("key")
}

fn join_frame(session: &SessionId, device: &DeviceId) -> ProtocolFrame {
    ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
        session_id: session.clone(),
        device: DeviceIdentity {
            device_id: device.clone(),
            display_name: "Pinned listener".to_owned(),
        },
        invite_code: None,
        sync_port: 0,
        audio_port: 0,
    }))
}

/// Admits one listener over real TLS and returns both sessions.
fn admitted_pair(
    host_seed: u8,
    listener_seed: u8,
    session: &SessionId,
) -> (SecureControlSession, SecureControlSession) {
    let host_pkcs8 = key(host_seed)
        .to_pkcs8_der()
        .expect("pkcs8")
        .as_bytes()
        .to_vec();
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8).expect("identity");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");
    let listener_key = key(listener_seed);
    let host_spki = identity.spki.clone();
    let client_session = session.clone();
    let client = std::thread::spawn(move || {
        dial_and_prove(
            address,
            &host_spki,
            &client_session,
            SECURE_ADMISSION_GENERATION,
            &listener_key,
        )
    });
    let (tcp, _) = server.accept().expect("accept");
    let admitted =
        admit_listener(tcp, &identity, session, SECURE_ADMISSION_GENERATION).expect("admitted");
    let listener_channel = client.join().expect("client").expect("listener proved");
    (
        SecureControlSession::new(admitted.channel, session.clone()),
        SecureControlSession::new(listener_channel, session.clone()),
    )
}

#[test]
fn a_join_request_reaches_the_host_and_the_approval_returns_over_the_same_tls_channel() {
    let session = SessionId::new("join-session").expect("session");
    let device = DeviceId::new("listener-tls-1").expect("device");
    let (mut host, listener) = admitted_pair(51, 52, &session);

    listener
        .send_frame(&join_frame(&session, &device))
        .expect("listener sends join");
    match host.recv_frame(WAIT).expect("host receives join") {
        ProtocolFrame::Control(ControlMessage::JoinRequest(request)) => {
            assert_eq!(request.device.device_id, device);
            assert_eq!(request.session_id, session);
        }
        _ => panic!("expected a join request frame"),
    }

    host.send_frame(&ProtocolFrame::Control(ControlMessage::JoinApproval(
        JoinApproval {
            session_id: session.clone(),
            listener_id: device.clone(),
            trusted_for_future: false,
        },
    )))
    .expect("host sends approval");
    let mut listener = listener;
    match listener
        .recv_frame(WAIT)
        .expect("listener receives approval")
    {
        ProtocolFrame::Control(ControlMessage::JoinApproval(approval)) => {
            assert_eq!(approval.listener_id, device);
        }
        _ => panic!("expected an approval frame"),
    }
    listener.close();
    host.close();
}

#[test]
fn the_host_refuses_to_send_a_frame_for_another_session() {
    let session = SessionId::new("send-session").expect("session");
    let (host, listener) = admitted_pair(53, 54, &session);
    let other = SessionId::new("other-session").expect("other");
    let outcome = host.send_frame(&join_frame(&other, &DeviceId::new("x").expect("device")));
    assert!(matches!(outcome, Err(SecureSessionError::Protocol(_))));
    listener.close();
    host.close();
}

#[test]
fn queued_bytes_for_another_session_never_leave_the_host() {
    let session = SessionId::new("bytes-session").expect("session");
    let other = SessionId::new("bytes-other").expect("other");
    let (host, listener) = admitted_pair(57, 58, &session);
    // The control writer sends already-encoded bytes, so the byte path needs
    // the same session check as the frame path.
    let foreign =
        encode_frame(&join_frame(&other, &DeviceId::new("y").expect("device"))).expect("encode");
    assert!(matches!(
        host.send_encoded(foreign),
        Err(SecureSessionError::Protocol(_))
    ));
    // The same bytes for this session are accepted and delivered.
    let own =
        encode_frame(&join_frame(&session, &DeviceId::new("y").expect("device"))).expect("encode");
    host.send_encoded(own).expect("own-session bytes sent");
    let mut listener = listener;
    assert!(listener.recv_frame(WAIT).is_ok());
    listener.close();
    host.close();
}

#[test]
fn a_frame_for_another_session_arriving_over_tls_is_refused_and_the_session_recovers() {
    let session = SessionId::new("recv-session").expect("session");
    let other = SessionId::new("stale-session").expect("other");
    let device = DeviceId::new("listener-tls-2").expect("device");
    let (mut host, listener) = admitted_pair(55, 56, &session);

    // A peer that bypasses its own session check sends a foreign-session frame.
    let foreign = encode_frame(&join_frame(&other, &device)).expect("encode");
    listener.raw_send_for_test(foreign).expect("raw send");
    assert!(matches!(
        host.recv_frame(WAIT),
        Err(SecureSessionError::Protocol(_))
    ));

    // The next valid frame still decodes: the refusal did not poison the channel.
    listener
        .send_frame(&join_frame(&session, &device))
        .expect("valid join");
    assert!(matches!(
        host.recv_frame(WAIT).expect("valid frame after refusal"),
        ProtocolFrame::Control(ControlMessage::JoinRequest(_))
    ));
    listener.close();
    host.close();
}

#[test]
fn malformed_bytes_over_tls_are_refused_as_protocol_errors() {
    let session = SessionId::new("malformed-session").expect("session");
    let (mut host, listener) = admitted_pair(57, 58, &session);
    listener
        .raw_send_for_test(b"not a protocol frame".to_vec())
        .expect("raw send");
    assert!(matches!(
        host.recv_frame(WAIT),
        Err(SecureSessionError::Protocol(_))
    ));
    listener.close();
    host.close();
}

#[test]
fn a_silent_peer_times_out_instead_of_blocking_the_session() {
    let session = SessionId::new("silent-session").expect("session");
    let (mut host, listener) = admitted_pair(59, 60, &session);
    let started = std::time::Instant::now();
    assert!(matches!(
        host.recv_frame(Duration::from_millis(300)),
        Err(SecureSessionError::Channel(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    listener.close();
    host.close();
}
