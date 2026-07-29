use silent_disco_core::domain::{
    ApprovalMode, MAX_SCAN_WINDOW_MS, MIN_LATE_PACKET_THRESHOLD_MS, MIN_RESYNC_THRESHOLD_GAP_MS,
    TuningSettings,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, HostDraft, HostDraftPatch, InviteCodePatch,
    RuntimeRecordValidationError, TuningPatch,
};

#[test]
fn host_draft_validation_covers_required_fields_and_cross_field_rules() {
    let empty = HostDraft::default();
    assert_eq!(
        empty.validate_for_creation(),
        Err(RuntimeRecordValidationError::SessionName)
    );

    let named = empty
        .patched(&HostDraftPatch {
            session_name: Some("Oakland room".to_owned()),
            ..HostDraftPatch::default()
        })
        .expect("valid session name");
    assert_eq!(
        named.validate_for_creation(),
        Err(RuntimeRecordValidationError::AudioSourceRequired)
    );

    assert_eq!(
        named.patched(&HostDraftPatch {
            approval_mode: Some(ApprovalMode::InviteCode),
            ..HostDraftPatch::default()
        }),
        Err(RuntimeRecordValidationError::InviteCodeRequired)
    );

    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4_096), Some(2_000))
        .expect("valid staged source");
    let invite_draft = named
        .patched(&HostDraftPatch {
            approval_mode: Some(ApprovalMode::InviteCode),
            invite_code: InviteCodePatch::Set("2468".to_owned()),
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(true),
            ..HostDraftPatch::default()
        })
        .expect("valid invite-code draft");
    invite_draft
        .validate_for_creation()
        .expect("complete host draft");

    assert_eq!(
        invite_draft.patched(&HostDraftPatch {
            approval_mode: Some(ApprovalMode::Manual),
            ..HostDraftPatch::default()
        }),
        Err(RuntimeRecordValidationError::UnexpectedInviteCode)
    );
    assert_eq!(invite_draft.approval_mode, ApprovalMode::InviteCode);
    assert_eq!(invite_draft.invite_code.as_deref(), Some("2468"));
}

#[test]
fn host_draft_rejects_invalid_text_and_tokens_without_trimming() {
    let draft = HostDraft::default();
    for invalid_name in ["", " room", "room ", "room\nname"] {
        assert_eq!(
            draft.patched(&HostDraftPatch {
                session_name: Some(invalid_name.to_owned()),
                ..HostDraftPatch::default()
            }),
            Err(RuntimeRecordValidationError::SessionName)
        );
    }

    assert_eq!(
        draft.patched(&HostDraftPatch {
            approval_mode: Some(ApprovalMode::InviteCode),
            invite_code: InviteCodePatch::Set("bad code".to_owned()),
            ..HostDraftPatch::default()
        }),
        Err(RuntimeRecordValidationError::InviteCode)
    );
}

#[test]
fn tuning_patch_validates_exact_values_instead_of_silently_clamping() {
    let defaults = TuningSettings::default();
    let valid = TuningPatch {
        late_packet_threshold_ms: Some(MIN_LATE_PACKET_THRESHOLD_MS),
        hard_resync_threshold_ms: Some(MIN_LATE_PACKET_THRESHOLD_MS + MIN_RESYNC_THRESHOLD_GAP_MS),
        scan_window_ms: Some(MAX_SCAN_WINDOW_MS),
        ..TuningPatch::default()
    }
    .apply_to(&defaults)
    .expect("valid boundary tuning");
    assert_eq!(valid.late_packet_threshold_ms, MIN_LATE_PACKET_THRESHOLD_MS);
    assert_eq!(valid.scan_window_ms, MAX_SCAN_WINDOW_MS);

    let out_of_range = TuningPatch {
        scan_window_ms: Some(MAX_SCAN_WINDOW_MS + 1),
        ..TuningPatch::default()
    };
    assert_eq!(
        out_of_range.apply_to(&defaults),
        Err(RuntimeRecordValidationError::TuningSettings)
    );
    assert_eq!(defaults, TuningSettings::default());

    let invalid_relationship = TuningPatch {
        late_packet_threshold_ms: Some(100),
        hard_resync_threshold_ms: Some(119),
        ..TuningPatch::default()
    };
    assert_eq!(
        invalid_relationship.apply_to(&defaults),
        Err(RuntimeRecordValidationError::TuningSettings)
    );
}
