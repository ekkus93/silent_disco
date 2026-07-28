pub use super::types::{
    AudioSourceDescriptor, CapabilitySnapshot, CoreDiagnostic, DeliveryReport, DiagnosticField,
    HostDraft, HostDraftPatch, JoinRequestSummary, ListenerSummary, NetworkEndpoint,
    SessionAdvertisement, SnapshotRevision, SynchronizationSummary, TuningPatch,
};

#[allow(
    clippy::large_enum_variant,
    reason = "CoreNotification is a stable semantic contract; queue storage boxes notifications"
)]
#[path = "records.rs"]
mod contract;

pub use contract::*;
