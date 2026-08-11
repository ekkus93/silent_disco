use super::{
    AudioOutputRequest, CommandReceipt, CoreActorInput, CoreCommand, CoreCommandRequest,
    CoreSnapshot, PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, RuntimeContractError,
};
use crate::domain::{OperationId, TransportState};
use crate::runtime::SnapshotRevision;

#[test]
fn command_receipt_represents_queue_admission_only() {
    let receipt = CommandReceipt {
        operation_id: OperationId::new("operation-1").expect("valid operation ID"),
        accepted_at_revision: SnapshotRevision::new(7),
    };
    assert_eq!(receipt.accepted_at_revision.get(), 7);
}

#[test]
fn command_shape_rejects_invalid_volume_and_invite_code() {
    assert_eq!(
        CoreCommandRequest::new(
            SnapshotRevision::new(0),
            CoreCommand::SetLocalVolume {
                linear_gain: f32::NAN,
            },
        ),
        Err(RuntimeContractError::LinearGain)
    );
    assert_eq!(
        CoreCommandRequest::new(
            SnapshotRevision::new(0),
            CoreCommand::SubmitJoin {
                invite_code: Some("bad code".to_owned()),
            },
        ),
        Err(RuntimeContractError::InviteCode)
    );
}

#[test]
fn effect_requires_valid_unique_capabilities_and_preserves_correlation() {
    let operation_id = OperationId::new("operation-2").expect("valid operation ID");
    let effect = PlatformEffect::new(
        operation_id.clone(),
        PlatformEffectRequest::RequestCapabilities(vec![
            PermissionCapability::LocalNetwork,
            PermissionCapability::AudioSourceSelection,
        ]),
    )
    .expect("valid effect");
    assert_eq!(effect.operation_id, operation_id);

    assert_eq!(
        PlatformEffect::new(
            OperationId::new("operation-3").expect("valid operation ID"),
            PlatformEffectRequest::RequestCapabilities(vec![
                PermissionCapability::LocalNetwork,
                PermissionCapability::LocalNetwork,
            ]),
        ),
        Err(RuntimeContractError::CapabilityList)
    );
}

#[test]
fn completion_and_actor_input_return_same_operation_id() {
    let operation_id = OperationId::new("operation-4").expect("valid operation ID");
    let event = PlatformEvent::OperationSucceeded {
        operation_id: operation_id.clone(),
        completion: PlatformOperationCompletion::DiscoveryStarted,
    };
    let input = CoreActorInput::Platform(event);
    assert_eq!(input.operation_id(), Some(&operation_id));
}

#[test]
fn snapshot_rejects_discovery_state_disagreement() {
    let snapshot = CoreSnapshot {
        discovery_active: true,
        transport_state: TransportState::Idle,
        ..CoreSnapshot::default()
    };
    assert_eq!(
        snapshot.validate(),
        Err(RuntimeContractError::DiscoveryStateMismatch)
    );
}

#[test]
fn audio_output_request_rejects_zero_format() {
    assert_eq!(
        AudioOutputRequest::new(0, 2),
        Err(RuntimeContractError::AudioOutputFormat)
    );
    assert!(AudioOutputRequest::new(48_000, 2).is_ok());
}
