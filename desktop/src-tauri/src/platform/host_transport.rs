//! Owned desktop host transport worker.

use super::host_transport_events::{DesktopHostTransportEventSink, HostTransportEventProcessor};
use super::network_error::DesktopNetworkError;
use silent_disco_core::domain::{DeliverySeverity, MonotonicMillis};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{ControlMessage, Disconnect, JoinApproval, JoinRejection};
use silent_disco_core::runtime::{
    DeliveryReport, SessionAdvertisement, TransportEffect, TransportEffectRequest,
    TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{HostTransportNode, TransportClock, TransportErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TRANSPORT_EFFECT_QUEUE_CAPACITY: usize = 32;
const MAX_EFFECTS_PER_TICK: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostTransportStatus {
    pub(crate) running: bool,
    pub(crate) last_error: Option<String>,
}

pub(crate) struct ActiveHostSessionSnapshot {
    pub(crate) advertisement: SessionAdvertisement,
    pub(crate) endpoint: silent_disco_core::runtime::NetworkEndpoint,
    pub(crate) worker_running: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) observed_at_ms: u64,
}

#[derive(Debug)]
struct SharedStatus {
    running: AtomicBool,
    last_error: Mutex<Option<String>>,
}

pub(crate) struct DesktopHostTransportRuntime {
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    stop: Arc<AtomicBool>,
    effect_sender: SyncSender<TransportEffect>,
    status: Arc<SharedStatus>,
    clock: Arc<dyn TransportClock>,
    worker: Option<JoinHandle<Result<(), DesktopNetworkError>>>,
}

impl DesktopHostTransportRuntime {
    pub(super) fn start(
        node: Box<dyn HostTransportNode>,
        advertisement: SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, DesktopNetworkError> {
        let endpoint = node.endpoint();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(SharedStatus {
            running: AtomicBool::new(true),
            last_error: Mutex::new(None),
        });
        let (effect_sender, effect_receiver) = sync_channel(TRANSPORT_EFFECT_QUEUE_CAPACITY);
        let worker_stop = Arc::clone(&stop);
        let worker_status = Arc::clone(&status);
        let worker = thread::Builder::new()
            .name("silent-disco-desktop-host-transport".to_owned())
            .spawn(move || {
                run_transport_worker(
                    node,
                    &advertisement,
                    &sink,
                    &effect_receiver,
                    &worker_stop,
                    &worker_status,
                )
            })
            .map_err(|error| {
                DesktopNetworkError::unavailable(format!(
                    "failed to start desktop host transport worker: {error}"
                ))
            })?;
        Ok(Self {
            endpoint,
            stop,
            effect_sender,
            status,
            clock,
            worker: Some(worker),
        })
    }

    #[must_use]
    pub(crate) const fn endpoint(&self) -> silent_disco_core::runtime::NetworkEndpoint {
        self.endpoint
    }

    #[must_use]
    pub(crate) fn observed_at(&self) -> MonotonicMillis {
        self.clock.now()
    }

    pub(crate) fn dispatch(&self, effect: TransportEffect) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        if self.stop.load(Ordering::Acquire) {
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            )
            .core_error(Some(operation_id)));
        }
        match self.effect_sender.try_send(effect) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(DesktopNetworkError::resource_limit(
                "desktop host transport effect queue is full",
            )
            .core_error(Some(operation_id))),
            Err(TrySendError::Disconnected(_)) => Err(DesktopNetworkError::unavailable(
                "desktop host transport effect worker is unavailable",
            )
            .core_error(Some(operation_id))),
        }
    }

    pub(super) fn status(&self) -> Result<HostTransportStatus, DesktopNetworkError> {
        let last_error = self
            .status
            .last_error
            .lock()
            .map_err(|_| {
                DesktopNetworkError::invalid_state(
                    "desktop host transport status mutex was poisoned",
                )
            })?
            .clone();
        Ok(HostTransportStatus {
            running: self.status.running.load(Ordering::Acquire),
            last_error,
        })
    }

    pub(super) fn shutdown(mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        match worker.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker panicked during shutdown",
            )),
        }
    }
}

fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: &Receiver<TransportEffect>,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    let mut processor = HostTransportEventProcessor::new();
    let mut primary_error = None;
    while !stop.load(Ordering::Acquire) {
        if let Err(error) = process_effects(&*node, &**sink, effect_receiver, status) {
            primary_error = Some(error);
            break;
        }
        match node.recv_event(EVENT_POLL_INTERVAL) {
            Ok(event) => match processor.process(event, &*node, advertisement, &**sink) {
                Ok(Some(message)) => set_last_error(status, message)?,
                Ok(None) => {}
                Err(error) => {
                    set_last_error(status, error.to_string())?;
                    primary_error = Some(error);
                    break;
                }
            },
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => {
                let error = DesktopNetworkError::transport(&error);
                set_last_error(status, error.to_string())?;
                primary_error = Some(error);
                break;
            }
        }
    }

    let drain_error = fail_queued_effects(effect_receiver, &**sink, status).err();
    let shutdown_error = node
        .shutdown()
        .map_err(|error| DesktopNetworkError::transport(&error))
        .err();
    status.running.store(false, Ordering::Release);
    primary_error
        .or(drain_error)
        .or(shutdown_error)
        .map_or(Ok(()), Err)
}

fn process_effects(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    receiver: &Receiver<TransportEffect>,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    for _ in 0..MAX_EFFECTS_PER_TICK {
        match receiver.try_recv() {
            Ok(effect) => process_effect(node, sink, effect, status)?,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                return Err(DesktopNetworkError::unavailable(
                    "desktop host transport effect queue disconnected",
                ));
            }
        }
    }
    Ok(())
}

fn process_effect(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    effect: TransportEffect,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    let operation_id = effect.operation_id;
    let delivery = match effect.request {
        TransportEffectRequest::DeliverJoinApproval {
            session_id,
            listener_id,
            trusted_for_future,
            ..
        } => node.send_pending_control(
            &listener_id.clone(),
            &ControlMessage::JoinApproval(JoinApproval {
                session_id,
                listener_id,
                trusted_for_future,
            }),
        ),
        TransportEffectRequest::DeliverJoinRejection {
            session_id,
            listener_id,
            reason_code,
            ..
        } => node.send_pending_control(
            &listener_id.clone(),
            &ControlMessage::JoinRejection(JoinRejection {
                session_id,
                listener_id,
                reason: reason_code,
            }),
        ),
        TransportEffectRequest::DisconnectListener {
            session_id,
            listener_id,
            reason_code,
        } => node.send_pending_control(
            &listener_id.clone(),
            &ControlMessage::Disconnect(Disconnect {
                session_id,
                listener_id,
                reason: reason_code,
            }),
        ),
    };

    let report = match delivery {
        Ok(delivery) => delivery.report,
        Err(error) => {
            set_last_error(status, error.to_string())?;
            failed_delivery_report()
        }
    };
    submit_delivery(sink, operation_id, report)
}

fn submit_delivery(
    sink: &dyn DesktopHostTransportEventSink,
    operation_id: silent_disco_core::domain::OperationId,
    report: DeliveryReport,
) -> Result<(), DesktopNetworkError> {
    sink.submit_transport_event(CoreTransportEvent::DeliveryCompleted {
        operation_id,
        report,
    })
    .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))
}

fn fail_queued_effects(
    receiver: &Receiver<TransportEffect>,
    sink: &dyn DesktopHostTransportEventSink,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    loop {
        match receiver.try_recv() {
            Ok(effect) => {
                set_last_error(
                    status,
                    "transport effect cancelled during host transport shutdown".to_owned(),
                )?;
                submit_delivery(sink, effect.operation_id, failed_delivery_report())?;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

const fn failed_delivery_report() -> DeliveryReport {
    DeliveryReport {
        intended_peers: 1,
        successful_peers: 0,
        failed_peers: 1,
        severity: DeliverySeverity::PartialFailure,
    }
}

fn set_last_error(status: &SharedStatus, message: String) -> Result<(), DesktopNetworkError> {
    *status.last_error.lock().map_err(|_| {
        DesktopNetworkError::invalid_state("desktop host transport status mutex was poisoned")
    })? = Some(message);
    Ok(())
}
