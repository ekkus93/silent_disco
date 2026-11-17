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

/// One retry of a busy-refused join, as the core schedules it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiBusyJoinRetry {
    /// 1 for the first retry, up to `max_attempts`.
    pub attempt: u8,
    pub max_attempts: u8,
    /// How long to wait before connecting again.
    pub delay_ms: u64,
}

/// The retry after `retries_so_far` busy refusals, or `None` once they are
/// used up (owner decision 2026-11-09: at most 3).
#[uniffi::export]
pub fn next_busy_join_retry(retries_so_far: u8) -> Option<FfiBusyJoinRetry> {
    silent_disco_core::runtime::next_busy_join_retry(retries_so_far).map(|retry| FfiBusyJoinRetry {
        attempt: retry.attempt,
        max_attempts: silent_disco_core::runtime::MAX_BUSY_JOIN_RETRIES,
        delay_ms: retry.delay_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::{is_host_busy_rejection, next_busy_join_retry};

    #[test]
    fn the_exported_policy_is_the_cores() {
        assert!(is_host_busy_rejection("host_busy".to_owned()));
        assert!(!is_host_busy_rejection("device_not_approved".to_owned()));
        let retries: Vec<_> = (0..4)
            .map(|done| next_busy_join_retry(done).map(|r| (r.attempt, r.max_attempts, r.delay_ms)))
            .collect();
        assert_eq!(
            retries,
            vec![
                Some((1, 3, 1_000)),
                Some((2, 3, 2_000)),
                Some((3, 3, 4_000)),
                None
            ]
        );
    }
}
