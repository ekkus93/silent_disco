//! Owns the bounded desktop host-transport receive worker and deterministic shutdown.

use super::host_transport_events::{DesktopHostTransportEventSink, HostTransportEventProcessor};
use super::network_error::DesktopNetworkError;
use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};
use silent_disco_core::transport::{HostTransportNode, TransportErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_VISIBLE_TRANSPORT_ERROR_CHARS: usize = 512;

#[derive(Default)]
struct WorkerStatus {
    running: bool,
    last_error: Option<String>,
}

pub(crate) struct ActiveHostSessionSnapshot {
    pub(crate) advertisement: SessionAdvertisement,
    pub(crate) endpoint: NetworkEndpoint,
    pub(crate) worker_running: bool,
    pub(crate) last_transport_error: Option<String>,
}

pub(super) struct DesktopHostTransportRuntime {
    endpoint: NetworkEndpoint,
    advertisement: SessionAdvertisement,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<WorkerStatus>>,
    join: Option<JoinHandle<Result<(), DesktopNetworkError>>>,
}

impl DesktopHostTransportRuntime {
    pub(super) fn start(
        node: Box<dyn HostTransportNode>,
        advertisement: SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
    ) -> Result<Self, DesktopNetworkError> {
        let endpoint = node.endpoint();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(WorkerStatus {
            running: true,
            last_error: None,
        }));
        let worker_stop = Arc::clone(&stop);
        let worker_status = Arc::clone(&status);
        let worker_advertisement = advertisement.clone();
        let join = thread::Builder::new()
            .name("silent-disco-desktop-host-transport".to_owned())
            .spawn(move || {
                run_transport_worker(
                    node,
                    &worker_advertisement,
                    sink.as_ref(),
                    &worker_stop,
                    &worker_status,
                )
            })
            .map_err(|error| {
                DesktopNetworkError::unavailable(format!(
                    "desktop host transport worker could not start: {error}"
                ))
            })?;
        Ok(Self {
            endpoint,
            advertisement,
            stop,
            status,
            join: Some(join),
        })
    }

    pub(super) const fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    pub(super) fn snapshot(&self) -> Result<ActiveHostSessionSnapshot, DesktopNetworkError> {
        let status = self
            .status
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned())?;
        Ok(ActiveHostSessionSnapshot {
            advertisement: self.advertisement.clone(),
            endpoint: self.endpoint,
            worker_running: status.running,
            last_transport_error: status.last_error.clone(),
        })
    }

    pub(super) fn shutdown(mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let join = self.join.take().ok_or_else(|| {
            DesktopNetworkError::invalid_state("desktop host transport worker was already joined")
        })?;
        match join.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::invalid_state(
                "desktop host transport worker panicked",
            )),
        }
    }
}

fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &dyn DesktopHostTransportEventSink,
    stop: &AtomicBool,
    status: &Mutex<WorkerStatus>,
) -> Result<(), DesktopNetworkError> {
    let mut processor = HostTransportEventProcessor::new();
    let mut primary = None;
    while !stop.load(Ordering::Acquire) {
        match node.recv_event(EVENT_POLL_INTERVAL) {
            Ok(event) => match processor.process(event, node.as_ref(), advertisement, sink) {
                Ok(Some(message)) => set_last_error(status, &message),
                Ok(None) => {}
                Err(error) => {
                    set_last_error(status, &error.to_string());
                    primary = Some(error);
                    break;
                }
            },
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => {
                let error = DesktopNetworkError::transport(&error);
                set_last_error(status, &error.to_string());
                primary = Some(error);
                break;
            }
        }
    }
    let cleanup = node
        .shutdown()
        .err()
        .map(|error| DesktopNetworkError::transport(&error));
    if let Ok(mut status) = status.lock() {
        status.running = false;
    }
    match (primary, cleanup) {
        (None, None) => Ok(()),
        (Some(error), None) | (None, Some(error)) => Err(error),
        (Some(primary), Some(cleanup)) => Err(DesktopNetworkError::invalid_state(format!(
            "{primary}; host transport shutdown also failed: {cleanup}"
        ))),
    }
}

fn set_last_error(status: &Mutex<WorkerStatus>, message: &str) {
    if let Ok(mut status) = status.lock() {
        status.last_error = Some(message.chars().take(MAX_VISIBLE_TRANSPORT_ERROR_CHARS).collect());
    }
}
