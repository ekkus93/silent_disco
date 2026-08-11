//! Builds a signed P2 QR invitation for the desktop's active host session
//! (Block 31.1), reusing the shared Rust `p2` module's canonicalization,
//! validation, and signing-payload assembly -- this module supplies only the
//! desktop-specific inputs (the active session's data, the endpoint it is
//! actually bound to, and this profile's signing identity) and never
//! reimplements any of that logic.
//!
//! Unlike Android's peer-to-peer QR flow, a desktop invitation always
//! carries a real, already-bound network endpoint (`connection_payload_json`),
//! since a desktop host has no BLE/Wi-Fi Direct discovery path a listener
//! could fall back to -- the QR *is* the only way a listener learns where to
//! connect, short of the existing manual copy/paste.

use super::invitation_identity::DesktopHostSigningIdentity;
use crate::host_session_dto::HostInvitationDto;
use silent_disco_core::domain::ApprovalMode;
use silent_disco_core::p2::{P2Error, QrInvitationInput, finalize_qr, prepare_unsigned_qr};
use silent_disco_core::runtime::{HostDraft, NetworkEndpoint, SessionAdvertisement};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Matches this project's other short-lived-QR precedent
/// (`app/.../P2ViewModel.kt`'s `QR_INVITATION_LIFETIME_MS`) and stays well
/// under the shared core's own `p2::MAX_QR_LIFETIME_MS` (10 minutes).
const INVITATION_LIFETIME_MS: u64 = 5 * 60 * 1_000;
/// Host display name shown inside the invitation. There is no configurable
/// desktop host display-name feature yet, so this is a fixed label --
/// mirrors Android's own P2 host flow, which uses the equivalent fixed
/// "This Android Host" for the same reason.
const HOST_DISPLAY_NAME: &str = "This Desktop Host";

/// Builds and signs one invitation for the given session/endpoint/identity,
/// stamped at `issued_at_ms`.
///
/// # Errors
///
/// Returns [`InvitationError::Invitation`] when the session data does not fit
/// the shared core's invitation bounds (session/host name length, invite code
/// shape, embedded connection payload size/shape) -- never silently drops a
/// field or signs a payload the shared validator would reject. Returns
/// [`InvitationError::Nonce`] when the platform CSPRNG used for the QR nonce
/// is unavailable, matching how every other identity/session-key path in
/// this codebase (`invitation_identity.rs`, `identity.rs`) surfaces the same
/// underlying `getrandom` failure as a typed, propagated error rather than a
/// panic.
pub(crate) fn build_signed_invitation(
    advertisement: &SessionAdvertisement,
    endpoint: NetworkEndpoint,
    host_draft: &HostDraft,
    identity: &DesktopHostSigningIdentity,
    issued_at_ms: u64,
) -> Result<HostInvitationDto, InvitationError> {
    let expires_at_ms = issued_at_ms.saturating_add(INVITATION_LIFETIME_MS);
    let connection_payload_json = connection_payload_json(advertisement, endpoint);
    let input = QrInvitationInput {
        session_id: advertisement.session_id.as_str().to_owned(),
        session_name: advertisement.session_name.clone(),
        host_name: HOST_DISPLAY_NAME.to_owned(),
        host_public_key_der: identity.public_key_der().to_vec(),
        approval_mode: p2_wire_name(advertisement.approval_mode).to_owned(),
        invite_code: if advertisement.approval_mode == ApprovalMode::InviteCode {
            host_draft.invite_code.clone()
        } else {
            None
        },
        issued_at_ms,
        expires_at_ms,
        nonce: generate_nonce().map_err(InvitationError::Nonce)?,
        connection_payload_json: Some(connection_payload_json),
    };
    let unsigned = prepare_unsigned_qr(&input)?;
    let signature = identity.sign_base64url(unsigned.as_bytes());
    let payload = finalize_qr(&unsigned, &signature)?;
    Ok(HostInvitationDto {
        payload,
        expires_at_ms: expires_at_ms.to_string(),
    })
}

/// Failure while building a signed QR invitation.
#[derive(Debug)]
pub(crate) enum InvitationError {
    /// The platform CSPRNG used to generate the QR nonce was unavailable.
    Nonce(getrandom::Error),
    /// The shared `p2` validator rejected the assembled invitation.
    Invitation(P2Error),
}

impl fmt::Display for InvitationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nonce(error) => {
                write!(formatter, "could not generate a secure QR nonce: {error}")
            }
            Self::Invitation(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for InvitationError {}

impl From<P2Error> for InvitationError {
    fn from(error: P2Error) -> Self {
        Self::Invitation(error)
    }
}

/// Builds the exact same manual-endpoint JSON shape the "Manual connection
/// details" panel already offers for copy/paste
/// (`transport::manual_endpoint::ManualHostEndpoint`'s wire shape) -- one
/// connection-payload format, not two.
fn connection_payload_json(
    advertisement: &SessionAdvertisement,
    endpoint: NetworkEndpoint,
) -> String {
    format!(
        r#"{{"hostAddress":"{}","controlPort":{},"syncPort":{},"audioPort":{},"sessionId":"{}","protocolVersion":{},"inviteCodeRequired":{},"expiresAtMs":null}}"#,
        endpoint.address,
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        advertisement.session_id.as_str(),
        advertisement.protocol_version,
        advertisement.approval_mode == ApprovalMode::InviteCode,
    )
}

/// Maps this project's shared [`ApprovalMode`] to the P2 wire vocabulary,
/// which intentionally differs from [`ApprovalMode::wire_name`]'s own
/// (`"trusted_devices"` vs. P2's `"approved_devices"`) -- mirrors Android's
/// existing `ApprovalMode.qrWireName()` (`P2ViewModel.kt`), which the same
/// shared `p2` validator already expects.
fn p2_wire_name(mode: ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::Manual => "manual",
        ApprovalMode::InviteCode => "invite_code",
        ApprovalMode::TrustedDevices => "approved_devices",
    }
}

/// 32 random bytes, hex-encoded to 64 characters -- comfortably inside the
/// shared core's 16-80 character nonce bound.
///
/// # Errors
///
/// Returns the underlying `getrandom` error when the platform CSPRNG is
/// unavailable, so the caller can surface a typed, visible failure instead
/// of silently falling back to a weaker source or crashing the process.
fn generate_nonce() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    let mut nonce = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(&mut nonce, "{byte:02x}");
    }
    Ok(nonce)
}

/// Current wall-clock time in milliseconds, used only to stamp when an
/// invitation was issued -- never for sync or playback scheduling, which
/// remain monotonic-only per this project's rules.
#[must_use]
pub(crate) fn current_wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::{InvitationError, build_signed_invitation, p2_wire_name};
    use crate::platform::invitation_identity::DesktopHostSigningIdentity;
    use silent_disco_core::domain::{ApprovalMode, DeviceId};
    use silent_disco_core::runtime::{HostDraft, NetworkEndpoint, SessionAdvertisement};
    use std::net::{IpAddr, Ipv4Addr};

    fn advertisement(approval_mode: ApprovalMode) -> SessionAdvertisement {
        SessionAdvertisement::new(
            silent_disco_core::domain::SessionId::new("session-block31").expect("session id"),
            DeviceId::new("desktop-block31").expect("device id"),
            "Block 31 Session",
            approval_mode,
            silent_disco_core::runtime::current_protocol_version(),
            None,
        )
        .expect("advertisement")
    }

    fn endpoint() -> NetworkEndpoint {
        NetworkEndpoint::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            41_000,
            41_001,
            41_002,
        )
        .expect("endpoint")
    }

    fn identity() -> DesktopHostSigningIdentity {
        // Constructed the same way production does, just without touching a
        // real OS keyring -- signing correctness is what these tests cover.
        super::super::invitation_identity::tests_support::fixed_identity_for_tests(3)
    }

    #[test]
    fn approval_mode_maps_to_the_shared_p2_wire_vocabulary() {
        assert_eq!(p2_wire_name(ApprovalMode::Manual), "manual");
        assert_eq!(p2_wire_name(ApprovalMode::InviteCode), "invite_code");
        // Deliberately NOT the domain wire_name() ("trusted_devices") --
        // P2's own validator only accepts "approved_devices" for this mode.
        assert_eq!(
            p2_wire_name(ApprovalMode::TrustedDevices),
            "approved_devices"
        );
    }

    #[test]
    fn a_valid_session_produces_a_signed_invitation_with_a_bounded_expiry() {
        let draft = HostDraft::default();
        let invitation = build_signed_invitation(
            &advertisement(ApprovalMode::Manual),
            endpoint(),
            &draft,
            &identity(),
            1_000_000_u64,
        )
        .expect("build invitation");
        assert!(invitation.payload.contains("\"v\":1"));
        assert!(invitation.payload.contains("192.168.1.50"));
        assert_eq!(invitation.expires_at_ms, "1300000");
    }

    #[test]
    fn invite_code_is_embedded_only_when_the_approval_mode_requires_it() {
        let draft = HostDraft {
            invite_code: Some("4826".to_owned()),
            ..HostDraft::default()
        };
        let invitation = build_signed_invitation(
            &advertisement(ApprovalMode::InviteCode),
            endpoint(),
            &draft,
            &identity(),
            1_000_000_u64,
        )
        .expect("build invitation");
        assert!(invitation.payload.contains("\"code\":\"4826\""));

        let manual_invitation = build_signed_invitation(
            &advertisement(ApprovalMode::Manual),
            endpoint(),
            &draft,
            &identity(),
            1_000_000_u64,
        )
        .expect("build invitation");
        assert!(manual_invitation.payload.contains("\"code\":null"));
    }

    /// Every field this function feeds into the shared `p2` validator (host
    /// name, session data, embedded connection JSON) is already exercised
    /// end to end by `p2::tests`' own oversized/tampered/structurally-invalid
    /// cases (Block 31.1's commit) -- this test's job is only to confirm
    /// this module's embedded connection-payload *shape* is one the shared
    /// validator (which parses it via `ManualHostEndpoint::parse`) actually
    /// accepts, not to re-test the shared validator itself.
    #[test]
    fn a_structurally_sound_connection_payload_survives_shared_core_validation() {
        let draft = HostDraft::default();
        let result: Result<_, InvitationError> = build_signed_invitation(
            &advertisement(ApprovalMode::Manual),
            endpoint(),
            &draft,
            &identity(),
            1_000_000_u64,
        );
        assert!(result.is_ok());
    }
}
