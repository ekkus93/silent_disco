use core::fmt;
use std::error::Error;

use crate::domain::MonotonicMillis;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LocalMonotonicMillis(u64);

impl LocalMonotonicMillis {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_elapsed_since(self, earlier: Self) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

impl From<MonotonicMillis> for LocalMonotonicMillis {
    fn from(value: MonotonicMillis) -> Self {
        Self::new(value.get())
    }
}

impl From<LocalMonotonicMillis> for MonotonicMillis {
    fn from(value: LocalMonotonicMillis) -> Self {
        Self::new(value.get())
    }
}

impl fmt::Display for LocalMonotonicMillis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HostMonotonicMillis(u64);

impl HostMonotonicMillis {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_elapsed_since(self, earlier: Self) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

impl From<MonotonicMillis> for HostMonotonicMillis {
    fn from(value: MonotonicMillis) -> Self {
        Self::new(value.get())
    }
}

impl From<HostMonotonicMillis> for MonotonicMillis {
    fn from(value: HostMonotonicMillis) -> Self {
        Self::new(value.get())
    }
}

impl fmt::Display for HostMonotonicMillis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SyncCorrelationId(u64);

impl SyncCorrelationId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SyncCorrelationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncExchange {
    pub correlation_id: SyncCorrelationId,
    pub t1_local_send: LocalMonotonicMillis,
    pub t2_host_receive: HostMonotonicMillis,
    pub t3_host_send: HostMonotonicMillis,
    pub t4_local_receive: LocalMonotonicMillis,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncSample {
    pub correlation_id: SyncCorrelationId,
    pub local_receive_time: LocalMonotonicMillis,
    pub round_trip_time_ms: f64,
    pub offset_ms: f64,
}

impl SyncSample {
    /// Computes one four-timestamp synchronization sample with checked ordering.
    ///
    /// # Errors
    ///
    /// Returns [`SyncTimestampError`] when local or host timestamps move
    /// backwards or when host processing time exceeds the local round trip.
    pub fn from_exchange(exchange: SyncExchange) -> Result<Self, SyncTimestampError> {
        let local_round_trip = exchange
            .t4_local_receive
            .checked_elapsed_since(exchange.t1_local_send)
            .ok_or(SyncTimestampError::LocalClockMovedBackwards)?;
        let host_processing = exchange
            .t3_host_send
            .checked_elapsed_since(exchange.t2_host_receive)
            .ok_or(SyncTimestampError::HostClockMovedBackwards)?;
        let network_round_trip = local_round_trip
            .checked_sub(host_processing)
            .ok_or(SyncTimestampError::HostProcessingExceedsRoundTrip)?;

        let offset_twice = (i128::from(exchange.t2_host_receive.get())
            - i128::from(exchange.t1_local_send.get()))
            + (i128::from(exchange.t3_host_send.get())
                - i128::from(exchange.t4_local_receive.get()));

        Ok(Self {
            correlation_id: exchange.correlation_id,
            local_receive_time: exchange.t4_local_receive,
            round_trip_time_ms: network_round_trip as f64,
            offset_ms: offset_twice as f64 / 2.0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTimestampError {
    LocalClockMovedBackwards,
    HostClockMovedBackwards,
    HostProcessingExceedsRoundTrip,
}

impl fmt::Display for SyncTimestampError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LocalClockMovedBackwards => {
                "local monotonic receive time precedes local send time"
            }
            Self::HostClockMovedBackwards => {
                "host monotonic send time precedes host receive time"
            }
            Self::HostProcessingExceedsRoundTrip => {
                "host processing duration exceeds the measured local round trip"
            }
        })
    }
}

impl Error for SyncTimestampError {}

#[cfg(test)]
mod tests {
    use super::{
        HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId, SyncExchange, SyncSample,
        SyncTimestampError,
    };

    fn exchange(t1: u64, t2: u64, t3: u64, t4: u64) -> SyncExchange {
        SyncExchange {
            correlation_id: SyncCorrelationId::new(7),
            t1_local_send: LocalMonotonicMillis::new(t1),
            t2_host_receive: HostMonotonicMillis::new(t2),
            t3_host_send: HostMonotonicMillis::new(t3),
            t4_local_receive: LocalMonotonicMillis::new(t4),
        }
    }

    #[test]
    fn calculates_kotlin_compatible_four_timestamp_sample() {
        let sample = SyncSample::from_exchange(exchange(2_000, 2_015, 2_017, 2_022))
            .expect("valid four-timestamp exchange");
        assert_eq!(sample.round_trip_time_ms, 20.0);
        assert_eq!(sample.offset_ms, 5.0);
    }

    #[test]
    fn preserves_half_millisecond_offset_without_integer_truncation() {
        let sample = SyncSample::from_exchange(exchange(3_000, 3_100, 3_101, 3_400))
            .expect("valid high-RTT exchange");
        assert_eq!(sample.round_trip_time_ms, 399.0);
        assert_eq!(sample.offset_ms, -99.5);
    }

    #[test]
    fn uses_wide_signed_arithmetic_near_u64_limits() {
        let sample = SyncSample::from_exchange(exchange(
            u64::MAX - 100,
            u64::MAX - 90,
            u64::MAX - 89,
            u64::MAX - 80,
        ))
        .expect("ordered near-limit exchange");
        assert_eq!(sample.round_trip_time_ms, 19.0);
        assert_eq!(sample.offset_ms, 0.5);
    }

    #[test]
    fn rejects_impossible_timestamp_orderings() {
        assert_eq!(
            SyncSample::from_exchange(exchange(100, 110, 111, 99)),
            Err(SyncTimestampError::LocalClockMovedBackwards)
        );
        assert_eq!(
            SyncSample::from_exchange(exchange(100, 112, 111, 120)),
            Err(SyncTimestampError::HostClockMovedBackwards)
        );
        assert_eq!(
            SyncSample::from_exchange(exchange(100, 110, 131, 120)),
            Err(SyncTimestampError::HostProcessingExceedsRoundTrip)
        );
    }
}
