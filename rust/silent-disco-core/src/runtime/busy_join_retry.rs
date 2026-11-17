//! Retrying a join the host refused as busy (remediation F3). A host that
//! cannot take a join right now (its event queue, pending-join table or core
//! is full) refuses it visibly with a busy reason instead of dropping it. The
//! listener then retries a bounded number of times with growing delays
//! (owner decision 2026-11-09: at most 3 retries). Every other rejection is
//! final.

/// The reason a host sends when it refuses a join only because it is busy.
pub const HOST_BUSY_REJECTION_REASON: &str = "host_busy";

/// How many times a listener retries after a busy refusal.
pub const MAX_BUSY_JOIN_RETRIES: u8 = 3;

/// Delay before the first retry; each later retry doubles it (1 s, 2 s, 4 s).
const FIRST_BUSY_RETRY_DELAY_MS: u64 = 1_000;

/// One scheduled retry of a busy-refused join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusyJoinRetry {
    /// 1 for the first retry, up to [`MAX_BUSY_JOIN_RETRIES`].
    pub attempt: u8,
    /// How long the platform waits before connecting again.
    pub delay_ms: u64,
}

/// Whether a host's rejection reason means "busy, try again". Covers the
/// shared reason code and the socket transport's "Host busy: …" refusals.
#[must_use]
pub fn is_host_busy_rejection(reason: &str) -> bool {
    reason == HOST_BUSY_REJECTION_REASON || reason.starts_with("Host busy")
}

/// The retry after `retries_so_far` busy refusals, or `None` once the
/// retries are used up.
#[must_use]
pub fn next_busy_join_retry(retries_so_far: u8) -> Option<BusyJoinRetry> {
    if retries_so_far >= MAX_BUSY_JOIN_RETRIES {
        return None;
    }
    Some(BusyJoinRetry {
        attempt: retries_so_far + 1,
        delay_ms: FIRST_BUSY_RETRY_DELAY_MS << retries_so_far,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_busy_reasons_are_retried() {
        assert!(is_host_busy_rejection("host_busy"));
        assert!(is_host_busy_rejection(
            "Host busy: join queue full; reconnect and retry"
        ));
        assert!(!is_host_busy_rejection("device_not_approved"));
        assert!(!is_host_busy_rejection("not tonight"));
        assert!(!is_host_busy_rejection("host_busy_but_not_really"));
    }

    #[test]
    fn three_retries_with_doubling_delays_then_none() {
        assert_eq!(
            (0..4).map(next_busy_join_retry).collect::<Vec<_>>(),
            vec![
                Some(BusyJoinRetry {
                    attempt: 1,
                    delay_ms: 1_000
                }),
                Some(BusyJoinRetry {
                    attempt: 2,
                    delay_ms: 2_000
                }),
                Some(BusyJoinRetry {
                    attempt: 3,
                    delay_ms: 4_000
                }),
                None,
            ]
        );
    }
}
