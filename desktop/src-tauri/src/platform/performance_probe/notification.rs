use super::{ProbeResult, duration_micros};
use crate::notification_buffer::{
    DESKTOP_PENDING_NOTIFICATION_CAPACITY, DesktopNotificationBuffer, DesktopNotificationSendError,
    DesktopNotificationSink,
};
use serde::Serialize;
use silent_disco_core::runtime::{CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const NOTIFICATION_BURST: u64 = 48;
const SINK_DELAY: Duration = Duration::from_millis(1);
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NotificationBridgeMetric {
    notifications_submitted: u64,
    notifications_delivered: u64,
    queue_capacity: usize,
    queue_peak_depth: usize,
    queue_depth_at_end: usize,
    delivery_elapsed_micros: u64,
}

pub(super) fn measure_notification_bridge() -> ProbeResult<NotificationBridgeMetric> {
    let buffer = DesktopNotificationBuffer::new();
    CoreObserver::on_notification(&buffer, CoreNotification::Snapshot(CoreSnapshot::default()))?;
    let delivered = Arc::new(AtomicU64::new(0));
    buffer.attach_sink(Arc::new(SlowSink {
        delivered: Arc::clone(&delivered),
    }))?;

    let started = Instant::now();
    let mut maximum_backlog = 0_usize;
    for index in 0..NOTIFICATION_BURST {
        let diagnostic = CoreDiagnostic::new(format!("block45-{index}"), Vec::new())?;
        CoreObserver::on_notification(&buffer, CoreNotification::Diagnostic(diagnostic))?;
        let submitted_total = index.saturating_add(2); // initial snapshot + diagnostics so far
        maximum_backlog = maximum_backlog.max(usize::try_from(
            submitted_total.saturating_sub(delivered.load(Ordering::Acquire)),
        )?);
    }
    let target_deliveries = NOTIFICATION_BURST.saturating_add(1);
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while delivered.load(Ordering::Acquire) < target_deliveries {
        if Instant::now() >= deadline {
            return Err(format!(
                "desktop notification bridge delivered {} of {target_deliveries}",
                delivered.load(Ordering::Acquire)
            )
            .into());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let elapsed = started.elapsed();
    let final_delivered = delivered.load(Ordering::Acquire);
    buffer.shutdown()?;

    Ok(NotificationBridgeMetric {
        notifications_submitted: NOTIFICATION_BURST,
        notifications_delivered: final_delivered,
        queue_capacity: DESKTOP_PENDING_NOTIFICATION_CAPACITY,
        queue_peak_depth: maximum_backlog,
        queue_depth_at_end: usize::try_from(target_deliveries.saturating_sub(final_delivered))?,
        delivery_elapsed_micros: duration_micros(elapsed)?,
    })
}

struct SlowSink {
    delivered: Arc<AtomicU64>,
}

impl DesktopNotificationSink for SlowSink {
    fn send(&self, _notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        std::thread::sleep(SINK_DELAY);
        self.delivered.fetch_add(1, Ordering::Release);
        Ok(())
    }
}
