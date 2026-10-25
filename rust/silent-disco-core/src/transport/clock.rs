use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::domain::MonotonicMillis;

pub trait TransportClock: Send + Sync + 'static {
    fn now(&self) -> MonotonicMillis;
}

#[derive(Debug)]
pub struct SystemTransportClock {
    origin: Instant,
    initial_millis: u64,
    // CLOCK_BOOTTIME includes device suspend, unlike Rust Instant's CLOCK_MONOTONIC.
    #[cfg(target_os = "android")]
    android_boot_relative: bool,
}

impl SystemTransportClock {
    /// Anchors the native monotonic clock to an existing platform clock
    /// domain. Android hosts supply elapsedRealtime at transport bind so
    /// cached late-join playback is compared against actual host timestamps.
    #[must_use]
    pub fn starting_at(initial_millis: u64) -> Self {
        Self {
            origin: Instant::now(),
            initial_millis,
            #[cfg(target_os = "android")]
            android_boot_relative: false,
        }
    }

    /// Uses Android's boot-relative monotonic clock directly, including time
    /// spent suspended. Matches SystemClock.elapsedRealtime without bind skew.
    #[cfg(target_os = "android")]
    #[must_use]
    pub fn android_boot_relative() -> Self {
        Self {
            origin: Instant::now(),
            initial_millis: 0,
            android_boot_relative: true,
        }
    }
}

impl Default for SystemTransportClock {
    fn default() -> Self {
        Self::starting_at(0)
    }
}

impl TransportClock for SystemTransportClock {
    fn now(&self) -> MonotonicMillis {
        #[cfg(target_os = "android")]
        if self.android_boot_relative {
            // Safe rustix wrapper around CLOCK_BOOTTIME: no JNI thread
            // attachment, wall-clock changes, or bind-time extrapolation.
            let now = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
            let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
            let nanoseconds = u64::try_from(now.tv_nsec).unwrap_or(0);
            return MonotonicMillis::new(
                seconds
                    .saturating_mul(1_000)
                    .saturating_add(nanoseconds / 1_000_000),
            );
        }
        let elapsed = self.origin.elapsed().as_millis();
        let millis = u64::try_from(elapsed).unwrap_or(u64::MAX);
        MonotonicMillis::new(self.initial_millis.saturating_add(millis))
    }
}

#[derive(Debug, Default)]
pub struct ManualTransportClock {
    now_ms: AtomicU64,
}

impl ManualTransportClock {
    #[must_use]
    pub const fn new(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    pub fn set(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, delta_ms: u64) {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl TransportClock for ManualTransportClock {
    fn now(&self) -> MonotonicMillis {
        MonotonicMillis::new(self.now_ms.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemTransportClock, TransportClock};

    #[test]
    fn anchored_clock_uses_platform_timestamp_without_resetting_to_zero() {
        let anchor = 87_654_321_u64;
        let clock = SystemTransportClock::starting_at(anchor);
        assert!(clock.now().get() >= anchor);
        assert!(clock.now().get() < anchor + 10_000);
        assert!(SystemTransportClock::default().now().get() < 10_000);
    }

    #[test]
    fn anchored_clock_saturates_instead_of_wrapping_near_u64_max() {
        let clock = SystemTransportClock::starting_at(u64::MAX);
        assert_eq!(clock.now().get(), u64::MAX);
    }

    #[cfg(target_os = "android")]
    #[test]
    fn android_host_clock_matches_suspend_aware_kernel_boottime() {
        use rustix::time::{ClockId, clock_gettime};

        let millis = || {
            let now = clock_gettime(ClockId::Boottime);
            u64::try_from(now.tv_sec).unwrap_or(0) * 1_000
                + u64::try_from(now.tv_nsec).unwrap_or(0) / 1_000_000
        };
        let clock = SystemTransportClock::android_boot_relative();
        let before = millis();
        let observed = clock.now().get();
        let after = millis();
        assert!(before <= observed && observed <= after);
    }
}
