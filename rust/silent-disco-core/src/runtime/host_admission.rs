use crate::domain::{ApprovalMode, DeviceId, RequestId, TrustState};
use super::{DeliveryReport, HostDraft, JoinRequestSummary};

/// Stable machine-readable rejection reasons emitted by host admission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRejectionReason {
    IncorrectInviteCode,
    HostRejected,
    HostUnavailable,
}

impl JoinRejectionReason {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::IncorrectInviteCode => "incorrect_invite_code",
            Self::HostRejected => "host_rejected",
            Self::HostUnavailable => "host_unavailable",
        }
    }
}

/// Policy result produced when a join request first reaches the authoritative core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRequestDisposition {
    PendingManualDecision,
    AutoApprove { trusted_for_future: bool },
    AutoReject { reason: JoinRejectionReason },
}

/// Storage work required before an approval may truthfully advertise durable trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustPersistenceRequest {
    pub request_id: RequestId,
    pub device_id: DeviceId,
    pub display_name: String,
}

/// Approval message content that is safe to deliver after any required persistence phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDelivery {
    pub request_id: RequestId,
    pub device_id: DeviceId,
    pub trusted_for_future: bool,
    pub persistence_failed: bool,
}

/// First step required by an explicit host approval command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalPreparation {
    PersistTrustFirst(TrustPersistenceRequest),
    Deliver(ApprovalDelivery),
}

/// Result of the pre-delivery durable trust write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustPersistenceOutcome {
    Committed,
    Failed,
}

/// Whether a completed control-message send is sufficient to commit the domain decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryCommitDisposition {
    Delivered,
    DeliveredWithFailures,
    NoRecipients,
    Failed,
}

impl DeliveryCommitDisposition {
    #[must_use]
    pub const fn commits_domain_decision(self) -> bool {
        matches!(self, Self::Delivered | Self::DeliveredWithFailures)
    }
}

/// Classifies a newly received request using only authoritative Rust policy.
#[must_use]
pub const fn classify_join_request(
    draft: &HostDraft,
    request: &JoinRequestSummary,
) -> JoinRequestDisposition {
    if matches!(draft.approval_mode, ApprovalMode::InviteCode) && !request.invite_code_valid {
        return JoinRequestDisposition::AutoReject {
            reason: JoinRejectionReason::IncorrectInviteCode,
        };
    }

    if matches!(draft.approval_mode, ApprovalMode::TrustedDevices)
        && matches!(request.trust_state, TrustState::Trusted)
    {
        return JoinRequestDisposition::AutoApprove {
            trusted_for_future: true,
        };
    }

    JoinRequestDisposition::PendingManualDecision
}

/// Plans an explicit approval without mutating the pending request.
///
/// Durable trust is written before approval delivery whenever the host requested
/// "remember approved devices" and the listener is not already trusted.
#[must_use]
pub fn prepare_approval(
    draft: &HostDraft,
    request: &JoinRequestSummary,
) -> ApprovalPreparation {
    if draft.remember_approved_devices && request.trust_state != TrustState::Trusted {
        ApprovalPreparation::PersistTrustFirst(TrustPersistenceRequest {
            request_id: request.request_id.clone(),
            device_id: request.device_id.clone(),
            display_name: request.display_name.clone(),
        })
    } else {
        ApprovalPreparation::Deliver(ApprovalDelivery {
            request_id: request.request_id.clone(),
            device_id: request.device_id.clone(),
            trusted_for_future: request.trust_state == TrustState::Trusted,
            persistence_failed: false,
        })
    }
}

/// Converts the durable trust result into the approval that may now be delivered.
///
/// A failed write is never hidden: the approval becomes session-only and carries
/// `persistence_failed = true` for a visible error/diagnostic path.
#[must_use]
pub fn approval_after_persistence(
    persistence: &TrustPersistenceRequest,
    outcome: TrustPersistenceOutcome,
) -> ApprovalDelivery {
    ApprovalDelivery {
        request_id: persistence.request_id.clone(),
        device_id: persistence.device_id.clone(),
        trusted_for_future: matches!(outcome, TrustPersistenceOutcome::Committed),
        persistence_failed: matches!(outcome, TrustPersistenceOutcome::Failed),
    }
}

/// Classifies delivery accounting without treating queueing as delivery.
#[must_use]
pub const fn classify_delivery(report: DeliveryReport) -> DeliveryCommitDisposition {
    if report.intended_peers == 0 {
        return DeliveryCommitDisposition::NoRecipients;
    }
    if report.successful_peers == 0 {
        return DeliveryCommitDisposition::Failed;
    }
    if report.failed_peers == 0 {
        DeliveryCommitDisposition::Delivered
    } else {
        DeliveryCommitDisposition::DeliveredWithFailures
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalPreparation, DeliveryCommitDisposition, JoinRejectionReason,
        JoinRequestDisposition, TrustPersistenceOutcome, approval_after_persistence,
        classify_delivery, classify_join_request, prepare_approval,
    };
    use crate::domain::{ApprovalMode, DeviceId, MonotonicMillis, RequestId, TrustState};
    use crate::runtime::{DeliveryReport, HostDraft, JoinRequestSummary};

    fn request(trust_state: TrustState, invite_code_valid: bool) -> JoinRequestSummary {
        JoinRequestSummary::new(
            RequestId::new("request-1").expect("valid request ID"),
            DeviceId::new("listener-1").expect("valid device ID"),
            "Listener One",
            trust_state,
            invite_code_valid,
            MonotonicMillis::new(10),
        )
        .expect("valid request")
    }

    #[test]
    fn invite_mode_auto_rejects_invalid_code_before_pending_insertion() {
        let draft = HostDraft {
            approval_mode: ApprovalMode::InviteCode,
            invite_code: Some("correct-code".to_owned()),
            ..HostDraft::default()
        };

        assert_eq!(
            classify_join_request(&draft, &request(TrustState::SessionOnly, false)),
            JoinRequestDisposition::AutoReject {
                reason: JoinRejectionReason::IncorrectInviteCode,
            }
        );
    }

    #[test]
    fn trusted_device_mode_auto_approves_only_durably_trusted_listener() {
        let draft = HostDraft {
            approval_mode: ApprovalMode::TrustedDevices,
            ..HostDraft::default()
        };

        assert_eq!(
            classify_join_request(&draft, &request(TrustState::Trusted, true)),
            JoinRequestDisposition::AutoApprove {
                trusted_for_future: true,
            }
        );
        assert_eq!(
            classify_join_request(&draft, &request(TrustState::SessionOnly, true)),
            JoinRequestDisposition::PendingManualDecision
        );
    }

    #[test]
    fn remember_policy_requires_persistence_before_delivery() {
        let draft = HostDraft {
            remember_approved_devices: true,
            ..HostDraft::default()
        };
        let request = request(TrustState::SessionOnly, true);

        let ApprovalPreparation::PersistTrustFirst(persistence) = prepare_approval(&draft, &request)
        else {
            panic!("approval must persist trust before delivery");
        };
        assert_eq!(persistence.request_id, request.request_id);
        assert_eq!(persistence.device_id, request.device_id);

        let delivery =
            approval_after_persistence(&persistence, TrustPersistenceOutcome::Committed);
        assert!(delivery.trusted_for_future);
        assert!(!delivery.persistence_failed);
    }

    #[test]
    fn failed_persistence_falls_back_visibly_to_session_only_delivery() {
        let draft = HostDraft {
            remember_approved_devices: true,
            ..HostDraft::default()
        };
        let request = request(TrustState::SessionOnly, true);
        let ApprovalPreparation::PersistTrustFirst(persistence) = prepare_approval(&draft, &request)
        else {
            panic!("approval must persist trust before delivery");
        };

        let delivery = approval_after_persistence(&persistence, TrustPersistenceOutcome::Failed);
        assert!(!delivery.trusted_for_future);
        assert!(delivery.persistence_failed);
    }

    #[test]
    fn session_only_approval_does_not_request_persistence() {
        let draft = HostDraft::default();
        let request = request(TrustState::SessionOnly, true);

        let ApprovalPreparation::Deliver(delivery) = prepare_approval(&draft, &request) else {
            panic!("session-only approval must deliver directly");
        };
        assert!(!delivery.trusted_for_future);
        assert!(!delivery.persistence_failed);
    }

    #[test]
    fn delivery_commits_only_when_at_least_one_peer_received_control() {
        let delivered = DeliveryReport::new(1, 1, 0).expect("valid report");
        let partial = DeliveryReport::new(2, 1, 1).expect("valid report");
        let zero = DeliveryReport::new(0, 0, 0).expect("valid report");
        let failed = DeliveryReport::new(1, 0, 1).expect("valid report");

        assert_eq!(
            classify_delivery(delivered),
            DeliveryCommitDisposition::Delivered
        );
        assert_eq!(
            classify_delivery(partial),
            DeliveryCommitDisposition::DeliveredWithFailures
        );
        assert_eq!(
            classify_delivery(zero),
            DeliveryCommitDisposition::NoRecipients
        );
        assert_eq!(classify_delivery(failed), DeliveryCommitDisposition::Failed);
        assert!(classify_delivery(partial).commits_domain_decision());
        assert!(!classify_delivery(zero).commits_domain_decision());
        assert!(!classify_delivery(failed).commits_domain_decision());
    }
}
