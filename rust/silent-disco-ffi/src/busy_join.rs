//! The core's busy-refusal rule, for the app's listener (remediation F3): the
//! app needs to know a rejection was "busy" before the core's snapshot says
//! so, to keep the refused connection's close from ending the retry.

/// Whether a host's join rejection reason means "busy, try again".
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
pub fn is_host_busy_rejection(reason: String) -> bool {
    silent_disco_core::runtime::is_host_busy_rejection(&reason)
}
