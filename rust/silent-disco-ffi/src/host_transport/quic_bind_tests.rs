//! v4 QUIC through the `UniFFI` host and listener handles: an explicit choice
//! on both ends, no fallback to v3 in either direction.

use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePublicKey;

use super::FfiHostTransportHandle;
use super::secure_bind_tests::{endpoint_payload, listener_id, signer};
use super::types::{FfiHostTransportError, FfiHostTransportEvent};
use crate::listener_transport::{
    FfiListenerTransportError, FfiListenerTransportEvent, FfiListenerTransportHandle,
};
use silent_disco_core::transport::SECURE_V4_ENDPOINT_VERSION;

/// 5 ms of 48 kHz stereo 16-bit PCM.
const FRAMES: u32 = 240;

fn host_spki() -> Vec<u8> {
    SigningKey::from_slice(&[61_u8; 32])
        .expect("fixture key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

/// What a v4 host advertises: one port, protocol version 4.
fn v4_payload(session: &str, port: u16) -> String {
    endpoint_payload(session, port, port, port).replace(
        &format!(
            "\"protocolVersion\":{}",
            silent_disco_core::runtime::current_protocol_version()
        ),
        &format!("\"protocolVersion\":{SECURE_V4_ENDPOINT_VERSION}"),
    )
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("probe")
        .local_addr()
        .expect("address")
        .port()
}

fn quic_host(session: &str) -> (std::sync::Arc<FfiHostTransportHandle>, u16) {
    let port = free_udp_port();
    let host = FfiHostTransportHandle::bind_quic_with_host_clock(
        "127.0.0.1".to_owned(),
        port,
        session.to_owned(),
        0,
        signer(false),
    )
    .expect("v4 host binds");
    (host, port)
}

fn wait_host<T>(
    host: &FfiHostTransportHandle,
    pick: impl Fn(FfiHostTransportEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(event) = host.poll_event(100).expect("host event")
            && let Some(found) = pick(event)
        {
            return found;
        }
    }
    panic!("expected host event did not arrive");
}

fn wait_listener<T>(
    listener: &FfiListenerTransportHandle,
    pick: impl Fn(FfiListenerTransportEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(event) = listener.poll_event(100).expect("listener event")
            && let Some(found) = pick(event)
        {
            return found;
        }
    }
    panic!("expected listener event did not arrive");
}

fn join_and_approve(host: &FfiHostTransportHandle, listener: &FfiListenerTransportHandle) {
    listener
        .send_join_request("QUIC listener".to_owned(), None)
        .expect("join");
    let (sync_port, audio_port) = wait_host(host, |event| match event {
        FfiHostTransportEvent::JoinRequestReceived {
            device_id,
            sync_port,
            audio_port,
            ..
        } if device_id == listener_id() => Some((sync_port, audio_port)),
        _ => None,
    });
    assert!(
        sync_port != 0 && audio_port != 0,
        "the join reports real ports"
    );
    host.send_join_approval(listener_id(), false)
        .expect("approval");
    host.authorize_listener(listener_id(), sync_port, audio_port)
        .expect("authorized");
    wait_listener(listener, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. }).then_some(())
    });
}

#[test]
fn a_v4_listener_joins_a_v4_host_and_receives_the_stream_through_the_handles() {
    let session = "ffi-quic-join";
    let (host, port) = quic_host(session);
    let listener = FfiListenerTransportHandle::connect_quic(
        v4_payload(session, port),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki(),
        signer(false),
    )
    .expect("v4 listener connects");
    join_and_approve(&host, &listener);

    host.broadcast_stream_start("stream-q".to_owned(), 5_000, 48_000, 2, FRAMES, None, None)
        .expect("start");
    wait_listener(&listener, |event| match event {
        FfiListenerTransportEvent::StreamStarted { stream_id, .. } => {
            (stream_id == "stream-q").then_some(())
        }
        _ => None,
    });
    let delivery = host
        .broadcast_audio(
            "stream-q".to_owned(),
            0,
            48_000,
            2,
            FRAMES,
            0,
            5_000,
            vec![7_u8; FRAMES as usize * 4],
        )
        .expect("audio");
    assert_eq!(delivery.successful_peers, 1);
    let sequence = wait_listener(&listener, |event| match event {
        FfiListenerTransportEvent::AudioReceived { sequence, .. } => Some(sequence),
        _ => None,
    });
    assert_eq!(sequence, 0);
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn a_plain_v3_connect_refuses_a_v4_host_before_connecting() {
    let session = "ffi-quic-only";
    let (host, port) = quic_host(session);
    let started = Instant::now();
    let outcome = FfiListenerTransportHandle::connect(
        v4_payload(session, port),
        0,
        "plain-listener".to_owned(),
        "127.0.0.1".to_owned(),
    );
    assert!(
        matches!(outcome, Err(FfiListenerTransportError::InvalidEndpoint(ref m)) if m.contains("signed invitation")),
        "a v4 endpoint needs the pinned invitation path"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "refused, not timed out"
    );
    host.shutdown().expect("host shutdown");
}

#[test]
fn a_pinned_listener_with_a_v4_invitation_joins_over_quic() {
    // Android's invitation path: connect_secure follows the advertised version.
    let session = "ffi-quic-pinned";
    let (host, port) = quic_host(session);
    let listener = FfiListenerTransportHandle::connect_secure(
        v4_payload(session, port),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki(),
        signer(false),
    )
    .expect("a pinned listener joins a v4 host over QUIC");
    join_and_approve(&host, &listener);
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn a_quic_only_connect_refuses_an_endpoint_that_is_not_v4() {
    let outcome = FfiListenerTransportHandle::connect_quic(
        endpoint_payload("ffi-not-v4", 4000, 4001, 4002),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki(),
        signer(false),
    );
    assert!(matches!(
        outcome,
        Err(FfiListenerTransportError::InvalidEndpoint(ref m)) if m.contains("does not use secure transport")
    ));
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
#[test]
fn a_v4_listener_never_falls_back_to_a_v3_host() {
    let session = "ffi-quic-no-fallback";
    let port = free_udp_port();
    let control = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("probe")
        .local_addr()
        .expect("address")
        .port();
    let host = FfiHostTransportHandle::bind_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        port,
        free_udp_port(),
        session.to_owned(),
        0,
    )
    .expect("v3 host binds");
    // The endpoint claims v4 but only a v3 host is there: QUIC gets no
    // answer, and nothing retries over v3.
    let outcome = FfiListenerTransportHandle::connect_quic(
        v4_payload(session, port),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki(),
        signer(false),
    );
    assert!(
        outcome.is_err(),
        "no QUIC endpoint, no connection, no v3 retry"
    );
    host.shutdown().expect("host shutdown");
}

#[test]
fn a_v4_host_refuses_to_bind_without_its_platform_key() {
    let outcome = FfiHostTransportHandle::bind_quic_with_host_clock(
        "127.0.0.1".to_owned(),
        0,
        "ffi-quic-no-key".to_owned(),
        0,
        signer(true),
    );
    assert!(matches!(
        outcome,
        Err(FfiHostTransportError::InvalidConfiguration(_))
    ));
}

#[test]
fn the_parser_marks_only_a_v4_endpoint_as_secure_transport() {
    use crate::listener_transport::parse_manual_host_endpoint;
    let v4 = parse_manual_host_endpoint(v4_payload("ffi-parse-v4", 4000), 0).expect("v4 parses");
    assert!(v4.secure_transport);
    let v3 = parse_manual_host_endpoint(endpoint_payload("ffi-parse-v3", 4000, 4001, 4002), 0)
        .expect("v3 parses");
    assert!(!v3.secure_transport);
}
