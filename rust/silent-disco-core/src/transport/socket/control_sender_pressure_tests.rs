//! Bounded authoritative control delivery under queue saturation.
use super::{ControlSender, ControlWrite, CounterState};
use crate::transport::TransportErrorKind;
use std::sync::Arc;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

fn occupied_queue() -> (
    std::sync::mpsc::SyncSender<ControlWrite>,
    std::sync::mpsc::Receiver<ControlWrite>,
) {
    let (sender, receiver) = sync_channel(1);
    let (ack, _waiter) = sync_channel(1);
    assert!(
        sender
            .send(ControlWrite {
                bytes: vec![1],
                result: ack,
                deadline: None,
            })
            .is_ok()
    );
    (sender, receiver)
}

fn await_backpressure(counters: &CounterState) {
    let started = Instant::now();
    while counters.snapshot().queue_overflows == 0 && started.elapsed() < Duration::from_secs(1) {
        thread::yield_now();
    }
    assert_eq!(
        counters.snapshot().queue_overflows,
        1,
        "the bounded control queue must report congestion"
    );
}

#[test]
fn full_control_queue_retries_in_fifo_order_and_acknowledges_delivery() {
    let (sender, receiver) = occupied_queue();
    let counters = Arc::new(CounterState::default());
    let control = ControlSender::new(sender, Duration::from_secs(2), counters.clone());
    let producer = thread::spawn(move || control.send(vec![2]));
    await_backpressure(&counters);

    let first = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("queued first control");
    assert_eq!(first.bytes, vec![1]);
    let second = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("retried second control");
    assert_eq!(second.bytes, vec![2]);
    assert!(second.result.send(Ok(1)).is_ok());
    assert_eq!(producer.join().expect("control sender").expect("ack"), 1);
    assert_eq!(counters.snapshot().delivery_failures, 0);
    assert_eq!(counters.snapshot().queue_overflows, 1);
}

#[test]
fn permanently_full_control_queue_fails_within_one_deadline_without_late_send() {
    let (sender, receiver) = occupied_queue();
    let counters = Arc::new(CounterState::default());
    let control = ControlSender::new(sender, Duration::from_millis(30), counters.clone());
    let started = Instant::now();
    let error = control
        .send(vec![2])
        .expect_err("must refuse when queue stays full");
    assert_eq!(error.kind, TransportErrorKind::QueueFull);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        receiver.try_recv().expect("original control").bytes,
        vec![1]
    );
    assert!(
        receiver.try_recv().is_err(),
        "timed-out control must not appear later"
    );
    assert_eq!(counters.snapshot().queue_overflows, 1);
    assert_eq!(counters.snapshot().delivery_failures, 1);
}

#[test]
fn closing_control_writer_interrupts_congested_sender() {
    let (sender, receiver) = occupied_queue();
    let counters = Arc::new(CounterState::default());
    let control = ControlSender::new(sender, Duration::from_secs(2), counters.clone());
    let producer = thread::spawn(move || control.send(vec![2]));
    await_backpressure(&counters);
    drop(receiver);
    let error = producer
        .join()
        .expect("control sender")
        .expect_err("writer retired");
    assert_eq!(error.kind, TransportErrorKind::ShuttingDown);
    assert_eq!(counters.snapshot().delivery_failures, 1);
}

#[test]
fn queued_control_ack_uses_remaining_budget_not_a_fresh_timeout() {
    let (sender, receiver) = occupied_queue();
    let counters = Arc::new(CounterState::default());
    let control = ControlSender::new(sender, Duration::from_millis(200), counters.clone());
    let producer = thread::spawn(move || control.send(vec![2]));
    await_backpressure(&counters);
    thread::sleep(Duration::from_millis(90));
    let _first = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("first control");
    let second = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("retried control");
    assert_eq!(second.bytes, vec![2]);
    // Hold the acknowledgement past the original deadline. A fresh 200 ms
    // timeout after queue admission would incorrectly report success here.
    thread::sleep(Duration::from_millis(140));
    let error = producer
        .join()
        .expect("control sender")
        .expect_err("deadline expired");
    assert_eq!(error.kind, TransportErrorKind::Timeout);
    // This fixture deliberately has no writer. The writer counts an expired
    // queued frame once when it eventually observes the original deadline.
    assert_eq!(counters.snapshot().delivery_failures, 0);
}

#[test]
fn concurrent_authoritative_controls_cannot_overtake_a_waiting_sender() {
    let (sender, receiver) = occupied_queue();
    let counters = Arc::new(CounterState::default());
    let control = Arc::new(ControlSender::new(
        sender,
        Duration::from_secs(2),
        counters.clone(),
    ));
    let first_control = control.clone();
    let first = thread::spawn(move || first_control.send(vec![2]));
    await_backpressure(&counters);

    // The first caller is already waiting for a slot. A later concurrent
    // Stop must not race past it when the stalled writer finally drains.
    let (entered_sender, entered_receiver) = sync_channel(0);
    let second_control = control.clone();
    let second = thread::spawn(move || {
        entered_sender.send(()).expect("signal second sender");
        second_control.send(vec![3])
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("second sender started");
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("initial queued frame")
            .bytes,
        vec![1]
    );
    let first_write = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("first waiting control");
    assert_eq!(first_write.bytes, vec![2]);
    first_write.result.send(Ok(1)).expect("ack first control");
    let second_write = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("later control");
    assert_eq!(second_write.bytes, vec![3]);
    second_write.result.send(Ok(1)).expect("ack later control");
    assert_eq!(first.join().expect("first sender").expect("first ack"), 1);
    assert_eq!(second.join().expect("second sender").expect("second ack"), 1);
    assert_eq!(counters.snapshot().delivery_failures, 0);
}
