//! The desktop host's opt-in secure transport switch (owner decision
//! 2026-11-12): off by default, chosen only by the switch, advertised as
//! version 4, and never changed under a live session.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};
use silent_disco_core::transport::{
    ListenerTransportConfig, QuicTransportFactory, SECURE_V4_ENDPOINT_VERSION, SecureHostTarget,
    SystemTransportClock, TransportFactory,
};

use super::mdns::{MdnsPublishError, MdnsPublisher, MdnsRegistration};
use super::network::{DesktopHostNetworkControl, NetworkErrorKind, TestHostPorts};
use super::network_tests::{FakeTransportFactory, SequenceProvider, advertisement, interface};

struct Fakes {
    control: DesktopHostNetworkControl,
    legacy: Arc<FakeTransportFactory>,
    secure: Arc<FakeTransportFactory>,
    published: Arc<Mutex<Vec<u16>>>,
}

/// Records the protocol version each mDNS publication carried.
struct RecordingMdns(Arc<Mutex<Vec<u16>>>);
struct Withdrawn;

impl MdnsRegistration for Withdrawn {
    fn withdraw(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

impl MdnsPublisher for RecordingMdns {
    fn publish(
        &self,
        advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        self.0
            .lock()
            .expect("published")
            .push(advertisement.protocol_version);
        Ok(Box::new(Withdrawn))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

fn fakes() -> Fakes {
    let records = vec![interface(
        "enp1s0",
        2,
        IpAddr::V4(Ipv4Addr::new(192, 168, 50, 20)),
    )];
    let provider = Arc::new(SequenceProvider::new([records.clone(), records]));
    let legacy = Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false))));
    let secure = Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false))));
    let published = Arc::new(Mutex::new(Vec::new()));
    let control = DesktopHostNetworkControl::with_components(
        provider,
        legacy.clone(),
        TestHostPorts::default(),
    )
    .with_secure_factory(secure.clone())
    .with_mdns_publisher(Arc::new(RecordingMdns(Arc::clone(&published))));
    Fakes {
        control,
        legacy,
        secure,
        published,
    }
}

fn with_identity(control: &DesktopHostNetworkControl) {
    let identity = crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(7)
        .host_tls_identity()
        .expect("tls identity");
    control.set_host_tls_identity(identity).expect("identity");
}

fn used(factory: &FakeTransportFactory) -> bool {
    factory.seen_config.lock().expect("seen").is_some()
}

#[test]
fn secure_transport_is_off_by_default_and_the_host_stays_v3_even_with_an_identity() {
    let f = fakes();
    with_identity(&f.control);
    assert!(!f.control.snapshot().expect("snapshot").secure_transport);
    f.control
        .start_host_inner(&advertisement())
        .expect("v3 host");
    assert!(used(&f.legacy) && !used(&f.secure));
    let active = f
        .control
        .active_host_session()
        .expect("session")
        .expect("active");
    assert_eq!(
        active.advertisement.protocol_version,
        advertisement().protocol_version
    );
    assert_ne!(
        active.advertisement.protocol_version,
        SECURE_V4_ENDPOINT_VERSION
    );
    f.control.stop_host_inner().expect("stop");
}

#[test]
fn with_the_switch_on_the_host_uses_quic_and_advertises_version_4_everywhere() {
    let f = fakes();
    with_identity(&f.control);
    f.control.set_secure_transport(true).expect("switch on");
    assert!(f.control.snapshot().expect("snapshot").secure_transport);
    f.control
        .start_host_inner(&advertisement())
        .expect("v4 host");
    assert!(used(&f.secure) && !used(&f.legacy));
    let active = f
        .control
        .active_host_session()
        .expect("session")
        .expect("active");
    // Manual details and invitations are built from this advertisement.
    assert_eq!(
        active.advertisement.protocol_version,
        SECURE_V4_ENDPOINT_VERSION
    );
    assert_eq!(
        *f.published.lock().expect("published"),
        vec![SECURE_V4_ENDPOINT_VERSION],
        "mDNS advertises version 4 too"
    );
    f.control.stop_host_inner().expect("stop");
}

#[test]
fn the_switch_without_a_tls_identity_refuses_to_host_instead_of_falling_back() {
    let f = fakes();
    f.control.set_secure_transport(true).expect("switch on");
    let error = f
        .control
        .start_host_inner(&advertisement())
        .expect_err("no identity, no v4 host");
    assert_eq!(error.kind, NetworkErrorKind::InvalidState);
    assert!(!used(&f.legacy) && !used(&f.secure), "nothing was bound");
    assert!(f.control.active_host_session().expect("session").is_none());
}

#[test]
fn the_switch_cannot_change_while_a_host_is_active() {
    let f = fakes();
    with_identity(&f.control);
    f.control
        .start_host_inner(&advertisement())
        .expect("v3 host");
    f.control
        .set_secure_transport(true)
        .expect_err("protocol cannot change under a live session");
    f.control
        .set_secure_transport(false)
        .expect("restating the current value is allowed");
    f.control.stop_host_inner().expect("stop");
    f.control
        .set_secure_transport(true)
        .expect("allowed again once stopped");
}

#[test]
fn a_v4_listener_connects_to_a_real_desktop_v4_host() {
    let Some((name, index, address)) = super::network::first_bindable_private_lan_address() else {
        eprintln!("no private LAN interface on this host; real QUIC bind not exercised");
        return;
    };
    let records = vec![interface(&name, index, IpAddr::V4(address))];
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([records.clone(), records])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    let identity = crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(8)
        .host_tls_identity()
        .expect("tls identity");
    let host_spki = identity.spki.clone();
    control.set_host_tls_identity(identity).expect("identity");
    control.set_secure_transport(true).expect("switch on");
    let endpoint = control
        .start_host_inner(&advertisement())
        .expect("real v4 host binds");
    assert_eq!(endpoint.sync_port, endpoint.control_port, "one QUIC port");

    let key = p256::ecdsa::SigningKey::from_slice(&[9_u8; 32]).expect("listener key");
    let spki = {
        use p256::pkcs8::EncodePublicKey;
        key.verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec()
    };
    let device = silent_disco_core::transport::secure_identity::device_id_for_listener_key(&spki);
    let mut config =
        ListenerTransportConfig::loopback(advertisement().session_id.clone(), device, endpoint);
    config.local_address = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    config.secure_host = Some(SecureHostTarget {
        host_spki,
        signer: Arc::new(key),
    });
    let listener = QuicTransportFactory
        .connect_listener(config, Arc::new(SystemTransportClock::default()))
        .expect("the listener proves its key and is admitted over QUIC");
    drop(listener);
    control.stop_host_inner().expect("stop");
}

/// Phase 4 (owner decision 2026-11-13): a release build cannot host
/// plaintext v3. With v3 unavailable the host uses QUIC and advertises
/// version 4 even though the switch was never turned on, the snapshot shows
/// v4 as fixed, and turning v4 off is refused.
#[test]
fn a_release_build_always_hosts_v4_and_refuses_to_turn_it_off() {
    let Fakes {
        control,
        legacy,
        secure,
        published,
    } = fakes();
    let control = control.with_legacy_v3(false);
    with_identity(&control);

    let snapshot = control.snapshot().expect("snapshot");
    assert!(snapshot.secure_transport, "v4 without touching the switch");
    assert!(!snapshot.legacy_transport_available);
    let refused = control
        .set_secure_transport(false)
        .expect_err("v3 cannot be chosen");
    assert!(
        refused.message.contains("not available in this build"),
        "{}",
        refused.message
    );

    control.start_host_inner(&advertisement()).expect("v4 host");
    assert!(
        used(&secure) && !used(&legacy),
        "never binds the v3 transport"
    );
    let active = control
        .active_host_session()
        .expect("session")
        .expect("active");
    assert_eq!(
        active.advertisement.protocol_version,
        SECURE_V4_ENDPOINT_VERSION
    );
    assert_eq!(
        *published.lock().expect("published"),
        vec![SECURE_V4_ENDPOINT_VERSION]
    );
    control.stop_host_inner().expect("stop");
}

/// A release build with no host TLS identity refuses to host rather than
/// falling back to v3.
#[test]
fn a_release_build_without_a_tls_identity_refuses_to_host() {
    let f = fakes();
    let control = f.control.with_legacy_v3(false);
    let error = control
        .start_host_inner(&advertisement())
        .expect_err("no identity, no host");
    assert!(error.message.contains("TLS identity"), "{}", error.message);
    assert!(!used(&f.legacy) && !used(&f.secure));
}

/// Test builds keep v3 available, so the switch still reports it.
#[test]
fn test_and_lab_builds_still_offer_the_v3_switch() {
    let f = fakes();
    assert!(
        f.control
            .snapshot()
            .expect("snapshot")
            .legacy_transport_available
    );
}

/// The real production control offers v3 only in a Lab Mode build. Run
/// without `--all-features` (as `npm run check` and release builds do), this
/// proves a release build hosts v4 only.
#[test]
fn the_production_control_offers_v3_only_in_lab_mode_builds() {
    let snapshot = DesktopHostNetworkControl::production()
        .snapshot()
        .expect("snapshot");
    assert_eq!(
        snapshot.legacy_transport_available,
        cfg!(feature = "lab-mode")
    );
    assert_eq!(snapshot.secure_transport, !cfg!(feature = "lab-mode"));
}
