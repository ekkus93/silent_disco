mod actor_runtime;
#[allow(
    clippy::large_enum_variant,
    reason = "CoreNotification is a stable semantic contract; queue storage boxes notifications"
)]
mod records;
#[allow(
    clippy::struct_excessive_bools,
    reason = "CapabilitySnapshot is an explicit cross-platform availability record"
)]
mod types;

pub use actor_runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreObserver,
    DEFAULT_ACTOR_QUEUE_CAPACITY, DEFAULT_NOTIFICATION_QUEUE_CAPACITY,
    MAX_ACTOR_QUEUE_CAPACITY, MAX_NOTIFICATION_QUEUE_CAPACITY,
};
pub use records::{
    AudioEvent, AudioOutputInfo, AudioOutputRequest, CommandReceipt, CoreActorInput, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, DiscoveryRequest,
    NetworkEstablishmentRequest, PermissionCapability, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, RecoverableAction, RuntimeContractError,
    StorageCompletion, StorageEvent, TransportEvent, current_protocol_version,
    MAX_CAPABILITY_REQUESTS, MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS,
    MAX_EXPORT_ID_BYTES, MAX_PENDING_JOIN_REQUESTS, MAX_STORAGE_TRUSTED_DEVICES,
};
pub use types::{
    AudioSourceDescriptor, AudioSourcePatch, CapabilitySnapshot, CoreDiagnostic, DeliveryReport,
    DiagnosticField, HostDraft, HostDraftPatch, InviteCodePatch, JoinRequestSummary,
    ListenerSummary, NetworkEndpoint, RuntimeRecordValidationError, SessionAdvertisement,
    SnapshotRevision, SynchronizationSummary, TuningPatch, MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES,
    MAX_AUDIO_SOURCE_ID_BYTES, MAX_DIAGNOSTIC_EVENT_NAME_BYTES, MAX_DIAGNOSTIC_FIELDS,
    MAX_DIAGNOSTIC_FIELD_KEY_BYTES, MAX_DIAGNOSTIC_FIELD_VALUE_BYTES,
};
