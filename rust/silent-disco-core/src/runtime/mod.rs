mod actor_runtime;
mod effects;
mod host_admission;
#[path = "records_runtime.rs"]
mod records;
#[allow(
    clippy::struct_excessive_bools,
    reason = "CapabilitySnapshot is an explicit cross-platform availability record"
)]
mod types;

pub use actor_runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreObserver, DEFAULT_ACTOR_QUEUE_CAPACITY,
    DEFAULT_NOTIFICATION_QUEUE_CAPACITY, MAX_ACTOR_QUEUE_CAPACITY, MAX_NOTIFICATION_QUEUE_CAPACITY,
};
pub use effects::{
    EffectContractError, StorageEffect, StorageEffectRequest, TransportEffect,
    TransportEffectRequest,
};
pub use host_admission::{
    ApprovalDelivery, ApprovalPreparation, DeliveryCommitDisposition, JoinRejectionReason,
    JoinRequestDisposition, TrustPersistenceOutcome, TrustPersistenceRequest,
    approval_after_persistence, classify_delivery, classify_join_request, prepare_approval,
    prepare_explicit_approval,
};
pub use records::{
    AudioEvent, AudioOutputInfo, AudioOutputRequest, CommandReceipt, CoreActorInput, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, DiscoveryRequest, MAX_CAPABILITY_REQUESTS,
    MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS, MAX_EXPORT_ID_BYTES,
    MAX_PENDING_JOIN_REQUESTS, MAX_STORAGE_TRUSTED_DEVICES, NetworkEstablishmentRequest,
    PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, RecoverableAction, RuntimeContractError, StorageCompletion,
    StorageEvent, TransportEvent, current_protocol_version,
};
pub use types::{
    AudioSourceDescriptor, AudioSourcePatch, CapabilitySnapshot, CoreDiagnostic, DeliveryReport,
    DiagnosticField, HostDraft, HostDraftPatch, InviteCodePatch, JoinRequestSummary,
    ListenerSummary, MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES, MAX_AUDIO_SOURCE_ID_BYTES,
    MAX_DIAGNOSTIC_EVENT_NAME_BYTES, MAX_DIAGNOSTIC_FIELD_KEY_BYTES,
    MAX_DIAGNOSTIC_FIELD_VALUE_BYTES, MAX_DIAGNOSTIC_FIELDS, NetworkEndpoint,
    RuntimeRecordValidationError, SessionAdvertisement, SnapshotRevision, SynchronizationSummary,
    TuningPatch,
};
