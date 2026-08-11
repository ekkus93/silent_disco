//! Desktop mDNS publish/withdraw adapter (Block 30).
//!
//! Broadcasts the same semantic advertisement the manual connection payload
//! already carries (`host_session_dto.rs`'s `HostConnectionDto`), as a
//! convenience layer alongside that manual path -- never a replacement for
//! it, and never a hidden requirement for transport (Block 30's own
//! acceptance criterion). The Android app has no mDNS/NSD client today, so
//! publishing here does not yet close the discovery loop end to end; that
//! is separate, not-yet-scoped follow-up work. See
//! `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` Block 30 for the full
//! scope note and the `mdns-sd` crate selection record (30.1).
//!
//! # Endpoint-change policy (30.2 "update after endpoint/interface change")
//!
//! A host session's bound endpoint is fixed for the session's lifetime --
//! nothing in this codebase supports rebinding an active session to a new
//! address, and the manual connection payload has the identical limitation.
//! Consistent with that, this adapter does **not** attempt to detect or
//! re-publish on a live interface/address change. If the bound interface
//! disappears mid-session, the existing publication goes stale exactly as
//! the manual payload would; recovery is the same as today's: stop and
//! start a new host session, which withdraws the old publication and
//! creates a fresh one.

use silent_disco_core::domain::ApprovalMode;
use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

/// Service type this host publishes under. `_tcp` because the primary
/// advertised port is the control channel, a TCP listener.
const SERVICE_TYPE: &str = "_silentdisco._tcp.local.";
/// Fixed per-machine mDNS hostname. One desktop process hosts at most one
/// session at a time (`NetworkState.active` is a single `Option`), so
/// there is never more than one service instance to disambiguate by
/// instance name alone; the hostname does not need to vary per session.
const HOST_NAME: &str = "silent-disco-desktop-host.local.";
/// RFC 6763 §6.4: one TXT attribute's value must not exceed 255 bytes.
const MAX_TXT_VALUE_BYTES: usize = 255;
/// Conservative bound on the summed TXT payload, well under mDNS's
/// practical single-UDP-packet ceiling -- deliberately smaller than the
/// protocol's absolute limit so this fails long before anything
/// downstream would, matching this project's general "bounded and
/// explicitly validated" framing rule.
const MAX_TOTAL_TXT_BYTES: usize = 1_300;
/// How long `withdraw` waits for the daemon to confirm an unregister
/// actually completed before reporting it as failed/unconfirmed, rather
/// than assuming success the instant the request was queued.
const WITHDRAW_CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);

/// Stable, structured failure taxonomy for this adapter. Every variant is
/// something a caller can act on or at least report honestly -- never a
/// silently-swallowed publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MdnsPublishError {
    /// One TXT field on its own exceeds [`MAX_TXT_VALUE_BYTES`].
    FieldTooLarge { field: String, bytes: usize },
    /// The summed TXT payload exceeds [`MAX_TOTAL_TXT_BYTES`].
    PayloadTooLarge { bytes: usize },
    /// The underlying `mdns-sd` daemon could not be created or reached
    /// (e.g. the multicast socket could not be bound) -- corresponds to
    /// 30.3's "daemon/multicast unavailable" test.
    DaemonUnavailable { message: String },
    /// `ServiceInfo` construction or `register` was rejected.
    RegistrationFailed { message: String },
    /// `unregister` was rejected, timed out, or reported the service was
    /// already gone from the daemon's own registry (a genuine
    /// inconsistency worth surfacing, not silently treated as success).
    WithdrawFailed { message: String },
}

impl fmt::Display for MdnsPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldTooLarge { field, bytes } => write!(
                formatter,
                "mDNS TXT field {field} is {bytes} bytes, over the {MAX_TXT_VALUE_BYTES}-byte \
                 per-field limit"
            ),
            Self::PayloadTooLarge { bytes } => write!(
                formatter,
                "mDNS TXT payload is {bytes} bytes, over the {MAX_TOTAL_TXT_BYTES}-byte bound"
            ),
            Self::DaemonUnavailable { message } => {
                write!(formatter, "mDNS daemon unavailable: {message}")
            }
            Self::RegistrationFailed { message } => {
                write!(formatter, "mDNS registration failed: {message}")
            }
            Self::WithdrawFailed { message } => {
                write!(formatter, "mDNS withdrawal failed: {message}")
            }
        }
    }
}

impl std::error::Error for MdnsPublishError {}

/// Publishes and withdraws one host session's mDNS advertisement.
///
/// Implemented by [`MdnsSdPublisher`] in production and by a recording
/// fake in tests, mirroring this codebase's established sink/adapter
/// testing pattern (e.g. `DesktopHostTransportEventSink`).
pub(super) trait MdnsPublisher: Send + Sync + 'static {
    /// Publishes only after a real, already-bound host endpoint exists --
    /// callers must pass the endpoint the transport actually bound to,
    /// never a value computed ahead of a successful bind (30.2 "publish
    /// only after real host endpoints exist").
    ///
    /// # Errors
    ///
    /// Returns [`MdnsPublishError`] if the advertisement does not fit
    /// mDNS's bounds or the daemon rejects the registration. Never
    /// silently degrades to "publication active" on failure.
    fn publish(
        &self,
        advertisement: &SessionAdvertisement,
        endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError>;

    /// Shuts down the underlying daemon (if one was ever created) and
    /// waits for confirmation. Not yet wired into the desktop's overall
    /// shutdown sequence (Block 36's job); exposed and tested here so
    /// that block has a real, confirmed capability to call rather than
    /// needing to invent one later.
    ///
    /// # Errors
    ///
    /// Returns [`MdnsPublishError::DaemonUnavailable`] if the daemon
    /// cannot be reached or does not confirm shutdown.
    fn shutdown(&self) -> Result<(), MdnsPublishError>;
}

/// One active publication, ownable and explicitly withdrawable.
pub(super) trait MdnsRegistration: Send + Sync {
    /// Withdraws the publication and waits for the daemon to confirm it,
    /// rather than assuming success once the request is merely queued.
    ///
    /// # Errors
    ///
    /// Returns [`MdnsPublishError::WithdrawFailed`] if the daemon reports
    /// failure, reports the service was already gone, or does not confirm
    /// within [`WITHDRAW_CONFIRM_TIMEOUT`].
    fn withdraw(&self) -> Result<(), MdnsPublishError>;
}

/// Builds this session's TXT properties from the core-owned semantic
/// advertisement and its real bound endpoint (30.2 "use core-owned
/// semantic advertisement"). Every field here is already bounded well
/// under [`MAX_TXT_VALUE_BYTES`] by its own domain type
/// (`MAX_IDENTIFIER_BYTES`/`MAX_SESSION_NAME_BYTES` are both 128), so this
/// can only ever build a compliant payload -- [`validate_txt_properties`]
/// is still run over the result rather than assumed, since defending only
/// at the call site that happens to be safe today is exactly the kind of
/// assumption this project's diagnostics rules ask not to make.
fn build_txt_properties(
    advertisement: &SessionAdvertisement,
    endpoint: &NetworkEndpoint,
) -> HashMap<String, String> {
    let invite_code_required = advertisement.approval_mode == ApprovalMode::InviteCode;
    let mut properties = HashMap::new();
    properties.insert("sessionId".to_owned(), advertisement.session_id.to_string());
    properties.insert("sessionName".to_owned(), advertisement.session_name.clone());
    properties.insert(
        "protocolVersion".to_owned(),
        advertisement.protocol_version.to_string(),
    );
    properties.insert("syncPort".to_owned(), endpoint.sync_port.to_string());
    properties.insert("audioPort".to_owned(), endpoint.audio_port.to_string());
    properties.insert(
        "inviteCodeRequired".to_owned(),
        invite_code_required.to_string(),
    );
    properties
}

/// Validates a TXT payload against mDNS's real bounds (30.2 "validate
/// service and field lengths"). Deliberately a pure function over any
/// `HashMap`, not folded into [`build_txt_properties`] -- nothing this
/// adapter can build today is capable of exceeding these bounds (every
/// upstream field is already capped at 128 bytes), so this validator can
/// only be genuinely exercised, including its rejection paths, against a
/// directly-constructed payload. See the `oversized_metadata_is_rejected`
/// test.
fn validate_txt_properties(properties: &HashMap<String, String>) -> Result<(), MdnsPublishError> {
    let mut total_bytes = 0_usize;
    for (field, value) in properties {
        let field_bytes = value.len();
        if field_bytes > MAX_TXT_VALUE_BYTES {
            return Err(MdnsPublishError::FieldTooLarge {
                field: field.clone(),
                bytes: field_bytes,
            });
        }
        // key + '=' + value, matching the wire encoding of one TXT attribute.
        total_bytes = total_bytes
            .saturating_add(field.len())
            .saturating_add(1)
            .saturating_add(field_bytes);
    }
    if total_bytes > MAX_TOTAL_TXT_BYTES {
        return Err(MdnsPublishError::PayloadTooLarge { bytes: total_bytes });
    }
    Ok(())
}

/// Production [`MdnsPublisher`] backed by a real `mdns-sd` daemon.
///
/// The daemon is created lazily on first use and reused across host
/// sessions within one desktop process lifetime (`ServiceDaemon` owns a
/// background thread and its own multicast socket; there is no benefit to
/// tearing it down between sessions, only at process shutdown).
pub(super) struct MdnsSdPublisher {
    daemon: Mutex<Option<mdns_sd::ServiceDaemon>>,
}

impl MdnsSdPublisher {
    pub(super) fn new() -> Self {
        Self {
            daemon: Mutex::new(None),
        }
    }

    fn daemon(&self) -> Result<mdns_sd::ServiceDaemon, MdnsPublishError> {
        let mut guard = self
            .daemon
            .lock()
            .map_err(|_| MdnsPublishError::DaemonUnavailable {
                message: "mDNS daemon handle is poisoned".to_owned(),
            })?;
        if let Some(daemon) = guard.as_ref() {
            return Ok(daemon.clone());
        }
        let daemon =
            mdns_sd::ServiceDaemon::new().map_err(|error| MdnsPublishError::DaemonUnavailable {
                message: error.to_string(),
            })?;
        *guard = Some(daemon.clone());
        Ok(daemon)
    }
}

impl MdnsPublisher for MdnsSdPublisher {
    fn publish(
        &self,
        advertisement: &SessionAdvertisement,
        endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        let properties = build_txt_properties(advertisement, &endpoint);
        validate_txt_properties(&properties)?;
        let daemon = self.daemon()?;
        let instance_name = advertisement.session_id.as_str();
        let service_info = mdns_sd::ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            HOST_NAME,
            endpoint.address,
            endpoint.control_port,
            properties,
        )
        .map_err(|error| MdnsPublishError::RegistrationFailed {
            message: error.to_string(),
        })?;
        let fullname = service_info.get_fullname().to_owned();
        daemon
            .register(service_info)
            .map_err(|error| MdnsPublishError::RegistrationFailed {
                message: error.to_string(),
            })?;
        Ok(Box::new(MdnsSdRegistration { daemon, fullname }))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        let mut guard = self
            .daemon
            .lock()
            .map_err(|_| MdnsPublishError::DaemonUnavailable {
                message: "mDNS daemon handle is poisoned".to_owned(),
            })?;
        let Some(daemon) = guard.take() else {
            // Never created (no session was ever published) -- nothing to
            // shut down, and that is a success, not an error.
            return Ok(());
        };
        let receiver = daemon
            .shutdown()
            .map_err(|error| MdnsPublishError::DaemonUnavailable {
                message: error.to_string(),
            })?;
        match receiver.recv_timeout(WITHDRAW_CONFIRM_TIMEOUT) {
            Ok(mdns_sd::DaemonStatus::Shutdown) => Ok(()),
            Ok(other) => Err(MdnsPublishError::DaemonUnavailable {
                message: format!("mDNS daemon reported unexpected status on shutdown: {other:?}"),
            }),
            Err(error) => Err(MdnsPublishError::DaemonUnavailable {
                message: format!("no shutdown confirmation from the mDNS daemon: {error}"),
            }),
        }
    }
}

struct MdnsSdRegistration {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration for MdnsSdRegistration {
    fn withdraw(&self) -> Result<(), MdnsPublishError> {
        let receiver = self.daemon.unregister(&self.fullname).map_err(|error| {
            MdnsPublishError::WithdrawFailed {
                message: error.to_string(),
            }
        })?;
        match receiver.recv_timeout(WITHDRAW_CONFIRM_TIMEOUT) {
            Ok(mdns_sd::UnregisterStatus::OK) => Ok(()),
            Ok(mdns_sd::UnregisterStatus::NotFound) => Err(MdnsPublishError::WithdrawFailed {
                message: format!("daemon reports {} was already unregistered", self.fullname),
            }),
            Err(error) => Err(MdnsPublishError::WithdrawFailed {
                message: format!("no withdrawal confirmation from the mDNS daemon: {error}"),
            }),
        }
    }
}

/// A no-op [`MdnsPublisher`] that always "succeeds" without publishing
/// anything. The default for `DesktopHostNetworkControl::with_components`,
/// which many tests unrelated to mDNS construct directly -- giving them a
/// harmless, deterministic mDNS layer rather than either a hard
/// requirement to inject a fake or accidental real multicast traffic on
/// every unrelated host-session test. `production()` overrides this with
/// a real [`MdnsSdPublisher`].
pub(super) struct NullMdnsPublisher;

impl MdnsPublisher for NullMdnsPublisher {
    fn publish(
        &self,
        _advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        Ok(Box::new(NullMdnsRegistration))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

struct NullMdnsRegistration;

impl MdnsRegistration for NullMdnsRegistration {
    fn withdraw(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

/// Outcome of attempting to publish one host session's mDNS advertisement,
/// held alongside the rest of that session's active state. A publication
/// failure is recorded, not propagated as a `start_host` failure (30.2
/// "never claim discovery active after publication failure" implies the
/// inverse must also hold: a failure here must never fail the whole
/// session either -- the manual endpoint remains fully functional
/// regardless of mDNS's fate).
pub(super) enum MdnsPublicationState {
    Active(Box<dyn MdnsRegistration>),
    Failed(MdnsPublishError),
}

impl MdnsPublicationState {
    /// Withdraws an active publication; a previously-failed one has
    /// nothing to withdraw and this is a clean no-op for it.
    pub(super) fn withdraw(&self) -> Result<(), MdnsPublishError> {
        match self {
            Self::Active(registration) => registration.withdraw(),
            Self::Failed(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TOTAL_TXT_BYTES, MAX_TXT_VALUE_BYTES, MdnsPublishError, build_txt_properties,
        validate_txt_properties,
    };
    use silent_disco_core::domain::{ApprovalMode, DeviceId, SessionId};
    use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr};

    fn advertisement(session_name: &str, approval_mode: ApprovalMode) -> SessionAdvertisement {
        SessionAdvertisement::new(
            SessionId::new("session-mdns-test").expect("session id"),
            DeviceId::new("mdns-test-host").expect("device id"),
            session_name.to_owned(),
            approval_mode,
            2,
            Some(endpoint()),
        )
        .expect("valid advertisement")
    }

    fn endpoint() -> NetworkEndpoint {
        NetworkEndpoint::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            41_100,
            41_101,
            41_102,
        )
        .expect("valid endpoint")
    }

    #[test]
    fn builds_expected_properties_for_a_normal_advertisement() {
        let properties = build_txt_properties(
            &advertisement("Living Room", ApprovalMode::Manual),
            &endpoint(),
        );
        assert_eq!(
            properties.get("sessionId").map(String::as_str),
            Some("session-mdns-test")
        );
        assert_eq!(
            properties.get("sessionName").map(String::as_str),
            Some("Living Room")
        );
        assert_eq!(
            properties.get("protocolVersion").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            properties.get("syncPort").map(String::as_str),
            Some("41101")
        );
        assert_eq!(
            properties.get("audioPort").map(String::as_str),
            Some("41102")
        );
        assert_eq!(
            properties.get("inviteCodeRequired").map(String::as_str),
            Some("false")
        );
        validate_txt_properties(&properties).expect("a real advertisement always fits");
    }

    #[test]
    fn invite_code_required_reflects_the_approval_mode() {
        let properties = build_txt_properties(
            &advertisement("Kitchen", ApprovalMode::InviteCode),
            &endpoint(),
        );
        assert_eq!(
            properties.get("inviteCodeRequired").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn the_longest_session_name_this_project_allows_still_fits() {
        // MAX_IDENTIFIER_BYTES / MAX_SESSION_NAME_BYTES are both 128 bytes,
        // comfortably under MAX_TXT_VALUE_BYTES (255) -- confirms the
        // whole realistic input space stays valid, not just one example.
        let longest_name = "x".repeat(128);
        let properties = build_txt_properties(
            &advertisement(&longest_name, ApprovalMode::Manual),
            &endpoint(),
        );
        validate_txt_properties(&properties)
            .expect("the longest session name this project's own validation allows must still fit");
    }

    #[test]
    fn a_field_over_the_255_byte_limit_is_rejected() {
        // Nothing buildable from a real SessionAdvertisement can trigger
        // this today (every upstream field is capped at 128 bytes) -- this
        // directly exercises the guard itself with a synthetic payload, so
        // the rejection path is proven correct even though production
        // inputs can't reach it yet.
        let mut properties = HashMap::new();
        properties.insert(
            "sessionName".to_owned(),
            "x".repeat(MAX_TXT_VALUE_BYTES + 1),
        );
        let error =
            validate_txt_properties(&properties).expect_err("oversized field must be rejected");
        assert_eq!(
            error,
            MdnsPublishError::FieldTooLarge {
                field: "sessionName".to_owned(),
                bytes: MAX_TXT_VALUE_BYTES + 1,
            }
        );
    }

    #[test]
    fn a_payload_over_the_total_budget_is_rejected_even_with_no_single_field_over_limit() {
        let mut properties = HashMap::new();
        // Several fields each under the per-field cap, but summing well
        // past MAX_TOTAL_TXT_BYTES.
        for index in 0..10 {
            properties.insert(format!("field{index}"), "x".repeat(200));
        }
        let error =
            validate_txt_properties(&properties).expect_err("oversized payload must be rejected");
        assert!(
            matches!(error, MdnsPublishError::PayloadTooLarge { bytes } if bytes > MAX_TOTAL_TXT_BYTES)
        );
    }

    // Real end-to-end tests against a genuine `mdns-sd` daemon (30.3) --
    // this is exactly what 30.1 selected `mdns-sd` for: it can be its own
    // test client, so "discover from a test client" needs no external
    // `avahi-browse`/`dns-sd` tool or physical device. Each test uses a
    // distinct session_id (-> distinct mDNS instance name) so they stay
    // independent from each other and from other real-mDNS tests
    // elsewhere in this crate, even though real multicast is shared
    // machine-wide and `cargo test` runs them concurrently by default.

    use super::{MdnsPublisher, MdnsSdPublisher, SERVICE_TYPE};
    use std::time::{Duration, Instant};

    const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

    /// A real, currently-bindable LAN address, exactly like production
    /// always publishes -- `mdns-sd` silently fails to actually announce
    /// (no error, no multicast sent) when given an address that does not
    /// correspond to any real local interface, confirmed empirically while
    /// writing these tests. `None` on a machine with no such interface
    /// (e.g. some CI environments), matching this codebase's established
    /// `real_private_lan_address()` skip pattern in `start_playback_tests.rs`.
    fn real_endpoint() -> Option<NetworkEndpoint> {
        let (_name, _index, address) = super::super::network::first_bindable_private_lan_address()?;
        Some(
            NetworkEndpoint::new(IpAddr::V4(address), 41_100, 41_101, 41_102)
                .expect("valid endpoint"),
        )
    }

    fn advertisement_with_session(
        session_id: &str,
        endpoint: NetworkEndpoint,
    ) -> SessionAdvertisement {
        SessionAdvertisement::new(
            SessionId::new(session_id).expect("session id"),
            DeviceId::new("mdns-e2e-host").expect("device id"),
            "End To End Test Session".to_owned(),
            ApprovalMode::Manual,
            2,
            Some(endpoint),
        )
        .expect("valid advertisement")
    }

    /// Browses with a fresh, independent `ServiceDaemon` -- standing in for
    /// a genuinely separate client process, since `mdns-sd` daemons don't
    /// share any in-process state.
    fn resolve_from_a_fresh_client(
        instance_fullname: &str,
    ) -> Option<Box<mdns_sd::ResolvedService>> {
        let client = mdns_sd::ServiceDaemon::new().expect("test client daemon");
        let receiver = client.browse(SERVICE_TYPE).expect("browse");
        let deadline = Instant::now() + DISCOVERY_TIMEOUT;
        let mut found = None;
        while Instant::now() < deadline {
            let Ok(event) = receiver.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event
                && info.get_fullname() == instance_fullname
            {
                found = Some(info);
                break;
            }
        }
        drop(client.shutdown());
        found
    }

    #[test]
    fn a_real_publish_is_discoverable_by_a_separate_client_with_the_right_data() {
        let Some(endpoint) = real_endpoint() else {
            eprintln!("no private LAN interface on this host; skipping");
            return;
        };
        let publisher = MdnsSdPublisher::new();
        let advertisement = advertisement_with_session("session-mdns-e2e-publish", endpoint);
        let registration = publisher
            .publish(&advertisement, endpoint)
            .expect("publish should succeed");

        let fullname = format!("{}.{}", advertisement.session_id.as_str(), SERVICE_TYPE);
        let resolved = resolve_from_a_fresh_client(&fullname)
            .expect("a fresh client should discover the service");
        assert_eq!(resolved.get_port(), endpoint.control_port);
        assert!(
            resolved
                .get_addresses()
                .iter()
                .any(|scoped| scoped.to_ip_addr() == endpoint.address),
            "resolved addresses {:?} should contain the published endpoint {}",
            resolved.get_addresses(),
            endpoint.address
        );
        let properties = resolved.get_properties();
        assert_eq!(
            properties.get("sessionId").map(mdns_sd::TxtProperty::val_str),
            Some("session-mdns-e2e-publish")
        );
        assert_eq!(
            properties.get("syncPort").map(mdns_sd::TxtProperty::val_str),
            Some(endpoint.sync_port.to_string().as_str())
        );

        drop(registration.withdraw());
        drop(publisher.shutdown());
    }

    #[test]
    fn withdrawing_removes_it_from_a_fresh_clients_discovery() {
        let Some(endpoint) = real_endpoint() else {
            eprintln!("no private LAN interface on this host; skipping");
            return;
        };
        let publisher = MdnsSdPublisher::new();
        let advertisement = advertisement_with_session("session-mdns-e2e-withdraw", endpoint);
        let registration = publisher
            .publish(&advertisement, endpoint)
            .expect("publish should succeed");
        let fullname = format!("{}.{}", advertisement.session_id.as_str(), SERVICE_TYPE);
        assert!(
            resolve_from_a_fresh_client(&fullname).is_some(),
            "must be discoverable before withdrawal"
        );

        registration
            .withdraw()
            .expect("withdraw should succeed and be confirmed");

        assert!(
            resolve_from_a_fresh_client(&fullname).is_none(),
            "must no longer be discoverable once withdrawn"
        );
        drop(publisher.shutdown());
    }

    #[test]
    fn republishing_under_the_same_instance_name_is_not_an_error() {
        // The crate's own documented behaviour: "To re-announce a service
        // with an updated service_info, just call this register function
        // again. No need to call unregister first." This is the
        // "duplicate service name" case 30.3 asks for -- a second
        // publish under the identical session_id (identical instance
        // name) must not error, matching real usage where a listener
        // re-reads an unchanged advertisement.
        let Some(endpoint) = real_endpoint() else {
            eprintln!("no private LAN interface on this host; skipping");
            return;
        };
        let publisher = MdnsSdPublisher::new();
        let advertisement = advertisement_with_session("session-mdns-e2e-duplicate", endpoint);
        let first = publisher
            .publish(&advertisement, endpoint)
            .expect("first publish should succeed");
        let second = publisher
            .publish(&advertisement, endpoint)
            .expect("re-publishing the same session_id must not error");

        let fullname = format!("{}.{}", advertisement.session_id.as_str(), SERVICE_TYPE);
        assert!(
            resolve_from_a_fresh_client(&fullname).is_some(),
            "must still be discoverable after re-publishing"
        );

        drop(first.withdraw());
        drop(second.withdraw());
        drop(publisher.shutdown());
    }

    #[test]
    fn shutdown_is_a_clean_no_op_when_nothing_was_ever_published() {
        let publisher = MdnsSdPublisher::new();
        publisher
            .shutdown()
            .expect("shutting down before any publish must succeed, not error");
    }

    #[test]
    fn shutdown_confirms_after_a_real_publish_and_withdraw() {
        let Some(endpoint) = real_endpoint() else {
            eprintln!("no private LAN interface on this host; skipping");
            return;
        };
        let publisher = MdnsSdPublisher::new();
        let advertisement = advertisement_with_session("session-mdns-e2e-shutdown", endpoint);
        let registration = publisher
            .publish(&advertisement, endpoint)
            .expect("publish should succeed");
        registration.withdraw().expect("withdraw should succeed");
        publisher
            .shutdown()
            .expect("shutdown should be confirmed after a real publish/withdraw cycle");
    }
}
