//! A bounded, short-lived refusal path for TCP joins after the host is full.
//! These sockets never become registered or authorized peers. Only a valid
//! first `JoinRequest` supplies the listener ID needed for a v3 `JoinRejection`.
//! Silent, invalid, or excess requests are closed without blocking accept.
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::protocol::{ControlMessage, JoinRejection, ProtocolFrame, encode_frame};
use crate::transport::HostTransportConfig;

use super::super::shared::{CounterState, ReadControlOutcome, read_control_loop};

const MAX_CONCURRENT_REFUSALS: usize = 2;
const REFUSAL_DEADLINE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(super) struct CapacityRefusalGate {
    in_flight: Arc<AtomicUsize>,
    handles: Mutex<Vec<thread::JoinHandle<()>>>,
}

struct RefusalPermit(Arc<AtomicUsize>);

impl Drop for RefusalPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CapacityRefusalGate {
    fn reserve(&self) -> Option<RefusalPermit> {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONCURRENT_REFUSALS).then_some(active + 1)
            })
            .ok()?;
        Some(RefusalPermit(self.in_flight.clone()))
    }

    /// Never read a join frame on the accept thread or allocate an unbounded
    /// number of threads under a hostile connection flood.
    pub(super) fn spawn(
        &self,
        stream: TcpStream,
        config: &HostTransportConfig,
        stop: &Arc<AtomicBool>,
        counters: &Arc<CounterState>,
    ) -> bool {
        let Some(permit) = self.reserve() else {
            return false; // Dropping stream immediately closes excess probes.
        };
        let deadline = config.operation_timeout.min(REFUSAL_DEADLINE);
        let io_timeout = config.io_timeout.min(deadline);
        let session = config.session_id.clone();
        let stop = stop.clone();
        let counters = counters.clone();
        let Ok(mut handles) = self.handles.lock() else {
            return false;
        };
        // Reap completed refusal workers on every capacity attempt. Without
        // this, repeated short-lived refusals accumulate JoinHandles for the
        // entire host lifetime despite the concurrency permit limit.
        let (finished, active): (Vec<_>, Vec<_>) =
            core::mem::take(&mut *handles)
                .into_iter()
                .partition(thread::JoinHandle::is_finished);
        *handles = active;
        // A detached JoinHandle hides panics. A refusal is never authorized,
        // but a panicking worker must still be visible in host diagnostics.
        for handle in finished {
            if handle.join().is_err() {
                counters.mark_worker_panicked();
            }
        }
        let worker = thread::Builder::new()
            .name("silent-disco-capacity-refusal".to_owned())
            .spawn(move || {
                let _permit = permit;
                if stop.load(Ordering::Acquire) {
                    return;
                }
                let Ok(mut writer) = stream.try_clone() else {
                    return;
                };
                if writer.set_write_timeout(Some(deadline)).is_err() {
                    return;
                }
                let joined = AtomicBool::new(false);
                read_control_loop(
                    stream,
                    &stop,
                    io_timeout,
                    deadline,
                    &session,
                    Some((&joined, deadline)),
                    |outcome| {
                        if let ReadControlOutcome::Frame(
                            ProtocolFrame::Control(ControlMessage::JoinRequest(request)),
                            received,
                        ) = outcome
                        {
                            counters.frame_received(received);
                            let refusal = ProtocolFrame::Control(ControlMessage::JoinRejection(
                                JoinRejection {
                                    session_id: session.clone(),
                                    listener_id: request.device.device_id,
                                    reason: "Host busy: capacity reached; reconnect and retry"
                                        .to_owned(),
                                },
                            ));
                            if let Ok(bytes) = encode_frame(&refusal) {
                                if writer
                                    .write_all(&bytes)
                                    .and_then(|()| writer.flush())
                                    .is_ok()
                                {
                                    counters.frame_sent(bytes.len());
                                } else {
                                    counters.delivery_failure();
                                }
                            } else {
                                counters.delivery_failure();
                            }
                        }
                        false // One first frame, one bounded refusal, then close.
                    },
                );
            });
        if let Ok(worker) = worker {
            handles.push(worker);
            true
        } else {
            false
        }
    }
}

impl Drop for CapacityRefusalGate {
    fn drop(&mut self) {
        // The accept loop has already observed shutdown. Refusal workers
        // have bounded reads/writes and must not outlive the host transport.
        // Joining also makes any worker panic observable before teardown.
        if let Ok(handles) = self.handles.get_mut() {
            for handle in handles.drain(..) {
                if handle.join().is_err() {
                    // No counters are owned by the gate on shutdown. A panic
                    // is a lifecycle violation rather than a detached thread.
                    if !thread::panicking() {
                        eprintln!("capacity refusal worker panicked during shutdown");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CapacityRefusalGate, MAX_CONCURRENT_REFUSALS};
    use std::sync::atomic::Ordering;

    #[test]
    fn refusal_permits_are_strictly_bounded_and_reusable() {
        let gate = CapacityRefusalGate::default();
        let first = gate.reserve().expect("first refusal slot");
        let second = gate.reserve().expect("second refusal slot");
        assert_eq!(
            gate.in_flight.load(Ordering::Acquire),
            MAX_CONCURRENT_REFUSALS
        );
        assert!(
            gate.reserve().is_none(),
            "capacity flood cannot add workers"
        );
        drop(first);
        let replacement = gate.reserve().expect("released slot is reusable");
        assert_eq!(
            gate.in_flight.load(Ordering::Acquire),
            MAX_CONCURRENT_REFUSALS
        );
        drop(second);
        drop(replacement);
        assert_eq!(gate.in_flight.load(Ordering::Acquire), 0);
        assert!(gate.handles.lock().expect("refusal handles").is_empty());
    }
}
