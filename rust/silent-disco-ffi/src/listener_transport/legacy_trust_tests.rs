use super::{FfiListenerTransportEvent, map_control_frame};
use silent_disco_core::domain::{DeviceId, SessionId};
use silent_disco_core::protocol::{ControlMessage, JoinApproval, ProtocolFrame};

fn mapped_approval(trusted_for_future: bool) -> FfiListenerTransportEvent {
    mapped_approval_over(trusted_for_future, false)
}

fn mapped_approval_over(trusted_for_future: bool, secure_v4: bool) -> FfiListenerTransportEvent {
    let approval = JoinApproval {
        session_id: SessionId::new("legacy-listener-test").expect("session"),
        listener_id: DeviceId::new("legacy-listener").expect("device"),
        trusted_for_future,
    };
    map_control_frame(
        ProtocolFrame::Control(ControlMessage::JoinApproval(approval)),
        secure_v4,
    )
    .expect("join result is surfaced to the listener")
}

#[test]
fn legacy_session_only_approval_remains_usable() {
    assert_eq!(
        mapped_approval(false),
        FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: false
        }
    );
}

#[test]
fn plaintext_host_cannot_claim_permanent_trust_at_listener_boundary() {
    let event = mapped_approval(true);
    assert!(
        matches!(
            event,
            FfiListenerTransportEvent::JoinRejected { reason }
                if reason.contains("unverified permanent trust")
        ),
        "a forged legacy trust flag must be a visible admission failure, not a trusted join"
    );
}

/// A secure-v4 host is the key pinned from the invitation and stores the
/// record itself, so its "Always allow" reaches the listener as remembered.
#[test]
fn a_secure_v4_host_can_tell_the_listener_it_was_remembered() {
    assert_eq!(
        mapped_approval_over(true, true),
        FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: true
        }
    );
    assert_eq!(
        mapped_approval_over(false, true),
        FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: false
        }
    );
}
