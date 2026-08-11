use std::sync::Arc;

use crate::domain::{DeliverySeverity, MonotonicMillis};
use crate::protocol::{ControlMessage, SyncRequest};

use super::test_support::{
    audio_frame, id_device, id_session, join_request, wait_for_authorized, wait_for_control_from,
    wait_for_frame, wait_for_frame_from, wait_for_host_event,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, SocketTransportFactory,
    TransportChannel, TransportClock, TransportEvent, TransportFactory,
};

/// Block A6 follow-up: confirmed on real hardware (2026-08-10) that the
/// host has no way to notice a listener that silently vanished -- it kept
/// reporting 100% `fully_delivered` for the rest of a run while the real
/// listener had received nothing since a Wi-Fi outage began, because a UDP
/// `send_to` to an unreachable peer on the same LAN segment still
/// "succeeds" at the OS level. This is the regression lock for the fix:
/// `SocketHostTransport::authorized_routes` now excludes (and evicts) any
/// peer that has sent nothing for `peer_inbound_silence_timeout`, so a
/// silent peer stops being counted as `successful` and its disconnection
/// becomes visible via the same `PeerDisconnected` event
/// `max_consecutive_failures` eviction already used. Uses
/// `ManualTransportClock` so the default 8s timeout is exercised exactly,
/// deterministically, with no real sleeping.
#[test]
fn a_silent_peer_is_evicted_and_stops_being_reported_as_delivered() {
    let session_id = id_session("inbound-silence-session");
    let factory = SocketTransportFactory;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host should bind");
    let endpoint = host.endpoint();
    let device_id = id_device("silent-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint),
            clock.clone(),
        )
        .expect("listener should connect");

    listener
        .send_control(&join_request(&session_id, &device_id, "Silent Listener"))
        .expect("join request should reach the host");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("peer routes should match authenticated control address");
    wait_for_authorized(&mut *host, &device_id);

    // One real sync request, proving inbound activity genuinely refreshes
    // the liveness marker (not just registration time), before the
    // listener goes silent for the rest of the test.
    listener
        .send_sync_request(&SyncRequest {
            session_id: session_id.clone(),
            correlation_id: 1,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(0),
        })
        .expect("authorized listener should send a sync request");
    wait_for_frame_from(
        &mut *host,
        TransportChannel::Synchronization,
        &device_id,
        |_| true,
    );

    // Confirmed alive right up to the silence timeout: still counted.
    clock.advance(7_999);
    let still_alive = host
        .broadcast_audio(&audio_frame(&session_id, 1))
        .expect("broadcast just under the silence timeout should still succeed");
    assert_eq!(still_alive.report.intended_peers, 1);
    assert_eq!(still_alive.report.successful_peers, 1);
    drop(wait_for_frame(
        &mut *listener,
        TransportChannel::Audio,
        |_| true,
    ));

    // Past the silence timeout with nothing further received: evicted.
    clock.advance(8_001);
    let after_silence = host
        .broadcast_audio(&audio_frame(&session_id, 2))
        .expect("broadcast past the silence timeout still returns explicit accounting");
    assert_eq!(
        after_silence.report.intended_peers, 0,
        "a silently-vanished listener must not still be counted as an intended recipient"
    );
    assert_eq!(after_silence.report.successful_peers, 0);
    assert_eq!(
        after_silence.report.severity,
        DeliverySeverity::ZeroPeers,
        "the host must report this honestly as zero recipients, not a clean success"
    );

    let disconnect = wait_for_host_event(&mut *host, |event| {
        matches!(
            event,
            TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&device_id)
        )
    });
    assert!(matches!(
        disconnect,
        TransportEvent::PeerDisconnected { .. }
    ));

    listener.shutdown().expect("listener should stop");
    host.shutdown().expect("host should stop");
}

/// Guards against the false-positive failure mode the sibling
/// `max_consecutive_failures` mechanism already warned about in its own
/// doc comment: a listener that keeps sending ordinary periodic sync
/// requests (steady-state cadence 2000ms, per
/// `SYNC_PROBE_CADENCE_MS` in `ManualListenerTransportController.kt`) must
/// never be evicted just because more than one cadence interval elapses
/// between probes.
#[test]
fn a_listener_that_keeps_probing_is_never_evicted_as_silent() {
    let session_id = id_session("inbound-liveness-session");
    let factory = SocketTransportFactory;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host should bind");
    let endpoint = host.endpoint();
    let device_id = id_device("live-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint),
            clock.clone(),
        )
        .expect("listener should connect");

    listener
        .send_control(&join_request(&session_id, &device_id, "Live Listener"))
        .expect("join request should reach the host");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("peer routes should match authenticated control address");
    wait_for_authorized(&mut *host, &device_id);

    // Several rounds spanning well past the silence timeout in total
    // elapsed time, each individual gap safely under it -- exactly ordinary
    // steady-state probing, not a burst.
    for round in 0..5_u64 {
        clock.advance(2_000);
        listener
            .send_sync_request(&SyncRequest {
                session_id: session_id.clone(),
                correlation_id: round,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(clock.now().get()),
            })
            .expect("authorized listener should send a sync request");
        wait_for_frame_from(
            &mut *host,
            TransportChannel::Synchronization,
            &device_id,
            |_| true,
        );
    }

    let delivery = host
        .broadcast_audio(&audio_frame(&session_id, 1))
        .expect("broadcast to a genuinely live listener should succeed");
    assert_eq!(
        delivery.report.intended_peers, 1,
        "a listener that never stopped probing must not be evicted as silent"
    );
    assert_eq!(delivery.report.successful_peers, 1);

    listener.shutdown().expect("listener should stop");
    host.shutdown().expect("host should stop");
}
