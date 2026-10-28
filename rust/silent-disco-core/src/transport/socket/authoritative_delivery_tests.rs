//! Bounded, shutdown-safe delivery of authoritative TCP events.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::shared::{CounterState, send_authoritative_event};

#[test]
fn full_queue_preserves_order_until_collector_recovers() {
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(CounterState::default());
    let (sender, receiver) = sync_channel(1);
    sender.send("StreamStart").expect("fill bounded queue");
    let pending_stop = stop.clone();
    let pending_counters = counters.clone();
    let worker = thread::spawn(move || {
        send_authoritative_event(&sender, &pending_stop, &pending_counters, "Stop")
    });
    let mut observed_pressure = false;
    for _ in 0..100 {
        if counters.snapshot().queue_overflows > 0 {
            observed_pressure = true;
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(observed_pressure, "must exercise queue saturation");
    assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), "StreamStart");
    assert!(worker.join().expect("delivery worker"), "Stop must reach app");
    assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), "Stop");
    assert_eq!(counters.snapshot().delivery_failures, 0);
}

#[test]
fn shutdown_interrupts_blocked_control_without_unbounded_wait() {
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(CounterState::default());
    let (sender, receiver) = sync_channel(1);
    sender.send("Start").expect("fill bounded queue");
    let pending_stop = stop.clone();
    let pending_counters = counters.clone();
    let worker = thread::spawn(move || {
        send_authoritative_event(&sender, &pending_stop, &pending_counters, "Stop")
    });
    for _ in 0..100 {
        if counters.snapshot().queue_overflows > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(counters.snapshot().queue_overflows, 1);
    stop.store(true, Ordering::Release);
    assert!(!worker.join().expect("shutdown interrupts writer"));
    assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), "Start");
    assert!(receiver.try_recv().is_err(), "retired Stop must not leak");
    assert_eq!(counters.snapshot().delivery_failures, 1);
}
