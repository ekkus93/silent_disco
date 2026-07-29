use silent_disco_core::domain::{
    ApprovalMode, DeviceId, MonotonicMillis, RequestId, TrustState, TuningSettings,
};
use silent_disco_core::runtime::{
    ApprovalPreparation, AudioSourceDescriptor, DeliveryCommitDisposition, DeliveryReport,
    HostDraft, HostDraftPatch, InviteCodePatch, JoinRejectionReason, JoinRequestDisposition,
    JoinRequestSummary, RuntimeRecordValidationError, TrustPersistenceOutcome, TuningPatch,
    approval_after_persistence, classify_delivery, classify_join_request, prepare_approval,
};

fn audio_source() -> AudioSourceDescriptor {
    AudioSourceDescriptor::new(
        "source-block12",
        "Block 12 fixture.wav",
        Some(4_096),
        Some(2_000),
    )
    .expect("valid test audio source")
}

fn join_request(trust_state: TrustState, invite_code_valid: bool) -> JoinRequestSummary {
    JoinRequestSummary::new(
        RequestId::new("request-block12").expect("valid request ID"),
        DeviceId::new("listener-block12").expect("valid listener ID"),
        "Block 12 listener",
        trust_state,
        invite_code_valid,
        MonotonicMillis::new(100),
    )
    .expect("valid join request")
}

#[test]
fn host_creation_validation_rejects_each_invalid_draft_shape() {
    assert_eq!(
        HostDraft::default().validate_for_creation(),
        Err(RuntimeRecordValidationError::SessionName)
    );

    let missing_source = HostDraft {
        session_name: "Block 12 session".to_owned(),
        ..HostDraft::default()
    };
    assert_eq!(
        missing_source.validate_for_creation(),
        Err(RuntimeRecordValidationError::AudioSourceRequired)
    );

    let missing_invite_code = HostDraft {
        session_name: "Block 12 session".to_owned(),
        approval_mode: ApprovalMode::InviteCode,
        invite_code: None,
        audio_source: Some(audio_source()),
        remember_approved_devices: false,
    };
    assert_eq!(
        missing_invite_code.validate_for_creation(),
        Err(RuntimeRecordValidationError::InviteCodeRequired)
    );

    let unexpected_invite_code = HostDraft {
        session_name: "Block 12 session".to_owned(),
        approval_mode: ApprovalMode::Manual,
        invite_code: Some("2468".to_owned()),
        audio_source: Some(audio_source()),
        remember_approved_devices: false,
    };
    assert_eq!(
        unexpected_invite_code.validate_for_creation(),
        Err(RuntimeRecordValidationError::UnexpectedInviteCode)
    );

    let valid = HostDraft {
        session_name: "Block 12 session".to_owned(),
        approval_mode: ApprovalMode::InviteCode,
        invite_code: Some("2468".to_owned()),
        audio_source: Some(audio_source()),
        remember_approved_devices: true,
    };
    valid.validate_for_creation().expect("valid host draft");
}

#[test]
fn host_draft_patch_rejects_cross_field_changes_atomically() {
    let valid = HostDraft::default()
        .patched(&HostDraftPatch {
            session_name: Some("Block 12 session".to_owned()),
            approval_mode: Some(ApprovalMode::InviteCode),
            invite_code: InviteCodePatch::Set("2468".to_owned()),
            audio_source: silent_disco_core::runtime::AudioSourcePatch::Set(audio_source()),
            remember_approved_devices: Some(true),
        })
        .expect("valid initial patch");

    let invalid = HostDraftPatch {
        approval_mode: Some(ApprovalMode::Manual),
        ..HostDraftPatch::default()
    };
    assert_eq!(
        valid.patched(&invalid),
        Err(RuntimeRecordValidationError::UnexpectedInviteCode)
    );
    assert_eq!(valid.approval_mode, ApprovalMode::InviteCode);
    assert_eq!(valid.invite_code.as_deref(), Some("2468"));
}

#[test]
fn tuning_patch_preserves_unmodified_values_and_rejects_invalid_relationships() {
    let current = TuningSettings::default();
    let changed = TuningPatch {
        sync_cadence_ms: Some(1_000),
        startup_buffer_ms: Some(500),
        ..TuningPatch::default()
    }
    .apply_to(&current)
    .expect("valid tuning patch");

    assert_eq!(changed.sync_cadence_ms, 1_000);
    assert_eq!(changed.startup_buffer_ms, 500);
    assert_eq!(changed.sync_sample_window, current.sync_sample_window);
    assert_eq!(
        changed.late_packet_threshold_ms,
        current.late_packet_threshold_ms
    );

    let invalid = TuningPatch {
        late_packet_threshold_ms: Some(100),
        hard_resync_threshold_ms: Some(110),
        ..TuningPatch::default()
    };
    assert_eq!(
        invalid.apply_to(&current),
        Err(RuntimeRecordValidationError::TuningSettings)
    );
}

#[test]
fn admission_policy_rejects_bad_invites_and_auto_approves_only_trusted_devices() {
    let invite_draft = HostDraft {
        approval_mode: ApprovalMode::InviteCode,
        invite_code: Some("2468".to_owned()),
        ..HostDraft::default()
    };
    assert_eq!(
        classify_join_request(
            &invite_draft,
            &join_request(TrustState::SessionOnly, false)
        ),
        JoinRequestDisposition::AutoReject {
            reason: JoinRejectionReason::IncorrectInviteCode,
        }
    );

    let trusted_draft = HostDraft {
        approval_mode: ApprovalMode::TrustedDevices,
        ..HostDraft::default()
    };
    assert_eq!(
        classify_join_request(&trusted_draft, &join_request(TrustState::Trusted, true)),
        JoinRequestDisposition::AutoApprove {
            trusted_for_future: true,
        }
    );
    assert_eq!(
        classify_join_request(
            &trusted_draft,
            &join_request(TrustState::SessionOnly, true)
        ),
        JoinRequestDisposition::PendingManualDecision
    );
}

#[test]
fn remembered_approval_requires_persistence_and_exposes_persistence_failure() {
    let draft = HostDraft {
        remember_approved_devices: true,
        ..HostDraft::default()
    };
    let request = join_request(TrustState::SessionOnly, true);
    let ApprovalPreparation::PersistTrustFirst(persistence) = prepare_approval(&draft, &request)
    else {
        panic!("remembered approval must persist trust before delivery");
    };

    let committed = approval_after_persistence(&persistence, TrustPersistenceOutcome::Committed);
    assert!(committed.trusted_for_future);
    assert!(!committed.persistence_failed);

    let failed = approval_after_persistence(&persistence, TrustPersistenceOutcome::Failed);
    assert!(!failed.trusted_for_future);
    assert!(failed.persistence_failed);
}

#[test]
fn delivery_accounting_never_treats_zero_or_failed_delivery_as_success() {
    let complete = DeliveryReport::new(2, 2, 0).expect("complete report");
    let partial = DeliveryReport::new(2, 1, 1).expect("partial report");
    let zero = DeliveryReport::new(0, 0, 0).expect("zero-recipient report");
    let failed = DeliveryReport::new(2, 0, 2).expect("failed report");

    assert_eq!(
        classify_delivery(complete),
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
