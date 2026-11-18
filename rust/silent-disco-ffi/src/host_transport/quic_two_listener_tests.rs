//! Two v4 listeners on one v4 host, through the `UniFFI` handles Android and
//! the desktop use, over real loopback QUIC (remediation TODO §1: the
//! two-listener v4 test, in software; a two-phone run is still separate).

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::EncodePublicKey;

use super::quic_bind_tests::{host_spki, quic_host, v4_payload, wait_host, wait_listener};
use super::types::FfiHostTransportEvent;
use crate::listener_transport::{FfiListenerTransportEvent, FfiListenerTransportHandle};
use crate::{FfiBridgeError, FfiListenerProofSigner};

const FRAMES: u32 = 240;
const PACKETS: u64 = 40;

/// A listener's own platform key.
struct ListenerKey(SigningKey);

impl FfiListenerProofSigner for ListenerKey {
    fn spki_der(&self) -> Result<Vec<u8>, FfiBridgeError> {
        Ok(spki(&self.0))
    }

    fn sign_der(&self, transcript: Vec<u8>) -> Result<Vec<u8>, FfiBridgeError> {
        let signature: Signature = self.0.sign(&transcript);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

fn spki(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

struct Listener {
    handle: Arc<FfiListenerTransportHandle>,
    device_id: String,
}

fn listener(seed: u8, session: &str, port: u16) -> Listener {
    let key = SigningKey::from_slice(&[seed; 32]).expect("listener key");
    let device_id =
        silent_disco_core::transport::secure_identity::device_id_for_listener_key(&spki(&key))
            .as_str()
            .to_owned();
    let handle = FfiListenerTransportHandle::connect_quic(
        v4_payload(session, port),
        0,
        device_id.clone(),
        "127.0.0.1".to_owned(),
        host_spki(),
        Arc::new(ListenerKey(key)),
    )
    .expect("v4 listener connects");
    Listener { handle, device_id }
}

fn join_and_approve(host: &super::FfiHostTransportHandle, listener: &Listener) {
    listener
        .handle
        .send_join_request(format!("Listener {}", &listener.device_id[..8]), None)
        .expect("join");
    let wanted = listener.device_id.clone();
    let (sync_port, audio_port) = wait_host(host, |event| match event {
        FfiHostTransportEvent::JoinRequestReceived {
            device_id,
            sync_port,
            audio_port,
            ..
        } if device_id == wanted => Some((sync_port, audio_port)),
        _ => None,
    });
    host.send_join_approval(listener.device_id.clone(), false)
        .expect("approval");
    host.authorize_listener(listener.device_id.clone(), sync_port, audio_port)
        .expect("authorized");
    wait_listener(&listener.handle, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. }).then_some(())
    });
}

/// Audio sequences a listener received, up to `count` of them.
fn received_sequences(listener: &Listener, count: u64) -> Vec<u64> {
    (0..count)
        .map(|_| {
            wait_listener(&listener.handle, |event| match event {
                FfiListenerTransportEvent::AudioReceived { sequence, .. } => Some(sequence),
                _ => None,
            })
        })
        .collect()
}

fn broadcast(host: &super::FfiHostTransportHandle, sequences: std::ops::Range<u64>) -> Vec<u32> {
    sequences
        .map(|sequence| {
            let delivery = host
                .broadcast_audio(
                    "stream-two".to_owned(),
                    sequence,
                    48_000,
                    2,
                    FRAMES,
                    sequence * u64::from(FRAMES),
                    5_000 + sequence * 5,
                    vec![u8::try_from(sequence % 251).expect("byte"); FRAMES as usize * 4],
                )
                .expect("audio");
            // Paced like real 5 ms packets, so loopback datagrams are not
            // dropped by a burst.
            thread::sleep(Duration::from_millis(2));
            delivery.successful_peers
        })
        .collect()
}

#[test]
fn two_v4_listeners_each_receive_the_whole_stream_and_one_leaving_does_not_affect_the_other() {
    let session = "ffi-quic-two";
    let (host, port) = quic_host(session);
    let first = listener(71, session, port);
    let second = listener(72, session, port);
    assert_ne!(
        first.device_id, second.device_id,
        "each listener has its own key-bound identity"
    );

    // Each join reaches the host under its own verified ID and is approved alone.
    join_and_approve(&host, &first);
    join_and_approve(&host, &second);

    host.broadcast_stream_start(
        "stream-two".to_owned(),
        5_000,
        48_000,
        2,
        FRAMES,
        None,
        None,
    )
    .expect("start");
    for listener in [&first, &second] {
        wait_listener(&listener.handle, |event| match event {
            FfiListenerTransportEvent::StreamStarted { stream_id, .. } => {
                (stream_id == "stream-two").then_some(())
            }
            _ => None,
        });
    }

    let deliveries = broadcast(&host, 0..PACKETS);
    assert!(
        deliveries.iter().all(|&peers| peers == 2),
        "every packet went to both: {deliveries:?}"
    );
    let expected: Vec<u64> = (0..PACKETS).collect();
    assert_eq!(
        received_sequences(&first, PACKETS),
        expected,
        "first listener"
    );
    assert_eq!(
        received_sequences(&second, PACKETS),
        expected,
        "second listener"
    );

    // The host drops the first listener; the second keeps playing.
    host.disconnect_peer(first.device_id.clone(), "removed by host".to_owned())
        .expect("disconnect");
    wait_listener(&first.handle, |event| match event {
        FfiListenerTransportEvent::HostDisconnected { .. }
        | FfiListenerTransportEvent::ConnectionClosed { .. } => Some(()),
        _ => None,
    });
    let first_id = first.device_id.clone();
    wait_host(&host, |event| match event {
        FfiHostTransportEvent::PeerDisconnectedAt { listener_id, .. }
        | FfiHostTransportEvent::PeerDisconnected { listener_id, .. } => {
            (listener_id.as_deref() == Some(first_id.as_str())).then_some(())
        }
        _ => None,
    });

    let deliveries = broadcast(&host, PACKETS..PACKETS * 2);
    assert!(
        deliveries.iter().all(|&peers| peers == 1),
        "only the remaining listener: {deliveries:?}"
    );
    let expected: Vec<u64> = (PACKETS..PACKETS * 2).collect();
    assert_eq!(
        received_sequences(&second, PACKETS),
        expected,
        "second listener after the first left"
    );

    let stop = host
        .broadcast_stop("stream-two".to_owned(), 5_400)
        .expect("stop");
    assert_eq!(stop.successful_peers, 1);
    wait_listener(&second.handle, |event| match event {
        FfiListenerTransportEvent::Stopped { stream_id, .. } => {
            (stream_id == "stream-two").then_some(())
        }
        _ => None,
    });

    first.handle.shutdown().expect("first shutdown");
    second.handle.shutdown().expect("second shutdown");
    host.shutdown().expect("host shutdown");
}
