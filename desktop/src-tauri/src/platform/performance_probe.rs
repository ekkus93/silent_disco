//! Block 45 desktop-only performance probes.
//!
//! Compiled only with the `performance-probe` Cargo feature. Each child
//! module drives a real bounded desktop/shared-core mechanism and reports raw
//! measurements while enforcing correctness/resource bounds only; no timing
//! threshold is invented here.

use serde::Serialize;
use std::error::Error;

mod monitor;
mod notification;
mod sync_scheduler;
mod transport;

use monitor::{MonitorCallbackMetric, measure_monitor_callback};
use notification::{NotificationBridgeMetric, measure_notification_bridge};
use sync_scheduler::{
    SchedulerMetric, SynchronizationMetric, measure_scheduler_concealment, measure_synchronization,
};
use transport::{DesktopTransportQueueMetric, measure_transport_queue};

pub type ProbeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopRuntimeMetric {
    transport_queue: DesktopTransportQueueMetric,
    notification_bridge: NotificationBridgeMetric,
    monitor_callback: MonitorCallbackMetric,
    synchronization: SynchronizationMetric,
    scheduler: SchedulerMetric,
}

/// Runs the desktop-only Block 45 probes.
///
/// # Errors
///
/// Returns immediately if a bounded queue overflows, a notification is lost,
/// monitor accounting is inconsistent, sync samples are rejected, scheduler
/// concealment diverges from the expected single-gap case, or shutdown fails.
pub fn measure_desktop_runtime() -> ProbeResult<DesktopRuntimeMetric> {
    Ok(DesktopRuntimeMetric {
        transport_queue: measure_transport_queue()?,
        notification_bridge: measure_notification_bridge()?,
        monitor_callback: measure_monitor_callback()?,
        synchronization: measure_synchronization()?,
        scheduler: measure_scheduler_concealment()?,
    })
}

pub(super) fn duration_micros(duration: std::time::Duration) -> ProbeResult<u64> {
    Ok(u64::try_from(duration.as_micros())?)
}
