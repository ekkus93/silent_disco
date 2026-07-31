use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::domain::MonotonicMillis;

pub trait TransportClock: Send + Sync + 'static {
    fn now(&self) -> MonotonicMillis;
}

#[derive(Debug)]
pub struct SystemTransportClock {
    origin: Instant,
}

impl Default for SystemTransportClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl TransportClock for SystemTransportClock {
    fn now(&self) -> MonotonicMillis {
        let elapsed = self.origin.elapsed().as_millis();
        let millis = u64::try_from(elapsed).unwrap_or(u64::MAX);
        MonotonicMillis::new(millis)
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
