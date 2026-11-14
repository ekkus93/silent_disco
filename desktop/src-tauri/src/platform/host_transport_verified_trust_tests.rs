//! Remediation F1 on the desktop host: over real loopback QUIC, a listener the
//! host remembered joins as Trusted, an unknown listener joins as session-only,
//! and "Always allow" reaches the listener as a remembered approval.

use super::host_transport::DesktopHostTransportRuntime;
use super::host_transport_admission_tests::{
    RecordingSink, TEST_TIMEOUT, advertisement, send_join, wait_for_control,
};
use super::host_transport_events::HostAdmission;
use super::verified_trust::DesktopVerifiedTrust;
use silent_disco_core::domain::{DeviceId, OperationId, SessionId, TransportState, TrustState};
use silent_disco_core::protocol::{ControlMessage, JoinApproval};
use silent_disco_core::runtime::{
    NetworkEndpoint, TransportEffect, TransportEffectRequest, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use silent_disco_core::transport::secure_identity::device_id_for_listener_key;
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportConfig, ListenerTransportNode, QuicTransportFactory,
    SecureHostTarget, SystemTransportClock, TransportFactory,
};
use std::sync::Arc;
use std::sync::mpsc;

struct Listener {
    device_id: DeviceId,
    node: Box<dyn ListenerTransportNode>,
}

fn connect(seed: u8, session: &SessionId, endpoint: NetworkEndpoint, host_spki: &[u8]) -> Listener {
    use p256::pkcs8::EncodePublicKey;
    let key = p256::ecdsa::SigningKey::from_slice(&[seed; 32]).expect("listener key");
    let spki = key
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let device_id = device_id_for_listener_key(&spki);
    let mut config =
        ListenerTransportConfig::loopback(session.clone(), device_id.clone(), endpoint);
    config.secure_host = Some(SecureHostTarget {
        host_spki: host_spki.to_vec(),
        signer: Arc::new(key),
    });
    let node = QuicTransportFactory
        .connect_listener(config, Arc::new(SystemTransportClock::default()))
        .expect("the listener proves its key over QUIC");
    Listener { device_id, node }
}

fn next_join(
    receiver: &mpsc::Receiver<CoreTransportEvent>,
) -> (DeviceId, TrustState, silent_disco_core::domain::RequestId) {
    loop {
        if let CoreTransportEvent::JoinRequested(request) = receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("actor received join")
        {
            return (request.device_id, request.trust_state, request.request_id);
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One acceptance scenario over real QUIC.
fn a_remembered_listener_joins_trusted_and_always_allow_reaches_the_listener() {
    let directory = tempfile::tempdir().expect("temp dir");
    let database = DatabaseWorker::start(
        DatabaseConfig::new(directory.path().join("domain.sqlite3")).expect("config"),
    )
    .expect("database");
    let identity = crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(8)
        .host_tls_identity()
        .expect("tls identity");
    let host_spki = identity.spki.clone();
    let trust =
        Arc::new(DesktopVerifiedTrust::load(database.client(), host_spki.clone()).expect("trust"));

    let session = SessionId::new("desktop-verified-trust-session").expect("session");
    let host_id = DeviceId::new("desktop-verified-trust-host").expect("host");
    let clock = Arc::new(SystemTransportClock::default());
    let mut config = HostTransportConfig::loopback(session.clone());
    config.secure_control = Some(Arc::new(identity));
    let node = QuicTransportFactory
        .bind_host(config, clock.clone())
        .expect("bind v4 host");
    let endpoint = node.endpoint();
    let (sender, receiver) = mpsc::channel();
    let runtime = DesktopHostTransportRuntime::start_with_admission(
        node,
        advertisement(&session, &host_id),
        RecordingSink::new(sender),
        clock,
        HostAdmission {
            secure_v4: true,
            trust: Some(Arc::clone(&trust)),
        },
    )
    .expect("runtime");

    // A listener the host never remembered joins as session-only.
    let mut stranger = connect(9, &session, endpoint, &host_spki);
    send_join(&*stranger.node, &session, &stranger.device_id);
    wait_for_control(&mut *stranger.node, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
    let (device, trust_state, request_id) = next_join(&receiver);
    assert_eq!(device, stranger.device_id);
    assert_eq!(trust_state, TrustState::SessionOnly);

    // "Always allow" is delivered as a remembered approval and the listener
    // connects as Trusted.
    runtime
        .dispatch(
            TransportEffect::new(
                OperationId::new("always-allow").expect("operation"),
                TransportEffectRequest::DeliverJoinApproval {
                    request_id,
                    session_id: session.clone(),
                    listener_id: stranger.device_id.clone(),
                    trusted_for_future: true,
                },
            )
            .expect("effect"),
        )
        .expect("dispatch");
    wait_for_control(&mut *stranger.node, |message| {
        matches!(
            message,
            ControlMessage::JoinApproval(JoinApproval {
                trusted_for_future: true,
                ..
            })
        )
    });
    let connected = loop {
        match receiver.recv_timeout(TEST_TIMEOUT).expect("connected") {
            CoreTransportEvent::ListenerConnected(listener) => break listener,
            CoreTransportEvent::DeliveryCompleted { report, .. } => {
                assert_eq!(report.successful_peers, 1, "approval delivered");
            }
            _ => {}
        }
    };
    assert_eq!(connected.device_id, stranger.device_id);
    assert_eq!(connected.trust_state, TrustState::Trusted);
    assert_eq!(connected.transport_state, TransportState::Connected);

    // A listener the host remembered joins as Trusted, so Approved-devices
    // policy can admit it without asking.
    let mut known = connect(10, &session, endpoint, &host_spki);
    trust
        .remember(&known.device_id, "Known listener", 1)
        .expect("remember");
    send_join(&*known.node, &session, &known.device_id);
    wait_for_control(&mut *known.node, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
    let (device, trust_state, _) = next_join(&receiver);
    assert_eq!(device, known.device_id);
    assert_eq!(trust_state, TrustState::Trusted);

    stranger.node.shutdown().expect("listener shutdown");
    known.node.shutdown().expect("listener shutdown");
    runtime.shutdown().expect("runtime shutdown");
    drop(trust);
    database.stop_and_join().expect("database shutdown");
}
