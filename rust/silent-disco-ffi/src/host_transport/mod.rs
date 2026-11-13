mod handle;
mod protocol_policy;
// Every test here drives a plaintext v3 host handle.
#[cfg(all(test, feature = "legacy-v3"))]
mod handle_tests;
#[cfg(test)]
mod quic_bind_tests;
#[cfg(test)]
mod secure_bind_tests;
mod types;

pub use handle::FfiHostTransportHandle;
pub use types::{
    FfiHostTransportCounters, FfiHostTransportDelivery, FfiHostTransportError,
    FfiHostTransportEvent,
};
