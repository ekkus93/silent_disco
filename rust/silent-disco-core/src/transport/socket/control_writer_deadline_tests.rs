//! A timed-out control send must not be transmitted from a stale writer backlog.
//! This uses an actual TCP socket and the production bounded writer queue.
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

use super::shared::{ControlSender, ControlWrite, CounterState, spawn_control_writer};
use crate::transport::TransportErrorKind;

#[test]
fn timed_out_queued_control_is_not_sent_after_writer_recovers() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = TcpStream::connect(listener.local_addr().expect("local address"))
        .expect("connect loopback");
    let (mut server, _) = listener.accept().expect("accept loopback");
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let (sender, receiver) = sync_channel(2);
    let counters = Arc::new(CounterState::default());
    let short = ControlSender::new(sender.clone(), Duration::from_millis(30), counters.clone());
    let long = ControlSender::new(sender, Duration::from_secs(2), counters.clone());

    // No writer is running yet. The old send times out while still queued;
    // its bytes must never reach TCP when the writer is eventually started.
    assert_eq!(
        short
            .send(b"stale".to_vec())
            .expect_err("caller times out")
            .kind,
        TransportErrorKind::Timeout
    );
    let stop = Arc::new(AtomicBool::new(false));
    let worker = spawn_control_writer(
        client,
        receiver,
        stop.clone(),
        counters.clone(),
        Duration::from_millis(20),
        "test-control-writer-deadline".to_owned(),
    )
    .expect("spawn production writer");
    assert_eq!(
        long.send(b"fresh".to_vec()).expect("deliver fresh control"),
        5
    );
    let mut bytes = [0_u8; 5];
    server
        .read_exact(&mut bytes)
        .expect("read only fresh control");
    assert_eq!(
        &bytes, b"fresh",
        "expired control must not precede new control"
    );
    assert_eq!(counters.snapshot().control_frames_sent, 1);
    assert_eq!(counters.snapshot().delivery_failures, 1);
    stop.store(true, Ordering::Release);
    worker.join().expect("writer exits");
}

#[test]
fn transiently_full_writer_queue_delivers_ordered_authoritative_tcp_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = TcpStream::connect(listener.local_addr().expect("local address"))
        .expect("connect loopback");
    let (mut server, _) = listener.accept().expect("accept loopback");
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");

    // Keep the production writer stopped until a second authoritative
    // control attempts to enter a genuinely full bounded queue.
    let (sender, receiver) = sync_channel(1);
    let (first_ack, first_result) = sync_channel(1);
    assert!(
        sender
            .send(ControlWrite {
                bytes: b"first".to_vec(),
                result: first_ack,
                deadline: None,
            })
            .is_ok()
    );
    let counters = Arc::new(CounterState::default());
    let control = ControlSender::new(sender, Duration::from_secs(2), counters.clone());
    let producer = thread::spawn(move || control.send(b"second".to_vec()));
    let started = Instant::now();
    while counters.snapshot().queue_overflows == 0 && started.elapsed() < Duration::from_secs(1) {
        thread::yield_now();
    }
    assert_eq!(counters.snapshot().queue_overflows, 1);

    let stop = Arc::new(AtomicBool::new(false));
    let worker = spawn_control_writer(
        client,
        receiver,
        stop.clone(),
        counters.clone(),
        Duration::from_millis(20),
        "test-congested-control-writer".to_owned(),
    )
    .expect("spawn production writer");
    assert_eq!(
        first_result
            .recv_timeout(Duration::from_secs(1))
            .expect("first acknowledgement")
            .expect("first delivered"),
        5
    );
    assert_eq!(
        producer
            .join()
            .expect("control sender")
            .expect("second delivered"),
        6
    );
    let mut bytes = [0_u8; 11];
    server
        .read_exact(&mut bytes)
        .expect("both ordered controls");
    assert_eq!(&bytes, b"firstsecond");
    assert_eq!(counters.snapshot().control_frames_sent, 2);
    assert_eq!(counters.snapshot().delivery_failures, 0);
    stop.store(true, Ordering::Release);
    worker.join().expect("writer exits");
}

#[test]
fn concurrent_pause_and_stop_keep_wire_order_during_real_tcp_backpressure() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = TcpStream::connect(listener.local_addr().expect("local address"))
        .expect("connect loopback");
    let (mut server, _) = listener.accept().expect("accept loopback");
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");

    // Start is already queued, and no writer is running. The first caller
    // waits for queue capacity; a second concurrent caller must not overtake
    // it when the real TCP writer starts draining the queue.
    let (sender, receiver) = sync_channel(1);
    let (start_ack, start_result) = sync_channel(1);
    sender
        .send(ControlWrite {
            bytes: b"start".to_vec(),
            result: start_ack,
            deadline: None,
        })
        .expect("queue initial Start");
    let counters = Arc::new(CounterState::default());
    let control = Arc::new(ControlSender::new(
        sender,
        Duration::from_secs(3),
        counters.clone(),
    ));
    let pause_control = control.clone();
    let pause = thread::spawn(move || pause_control.send(b"pause".to_vec()));
    let started = Instant::now();
    while counters.snapshot().queue_overflows == 0 && started.elapsed() < Duration::from_secs(1) {
        thread::yield_now();
    }
    assert_eq!(counters.snapshot().queue_overflows, 1);
    let (stop_started, stop_ready) = sync_channel(0);
    let stop_control = control.clone();
    let stop = thread::spawn(move || {
        stop_started.send(()).expect("signal Stop sender");
        stop_control.send(b"stop".to_vec())
    });
    stop_ready
        .recv_timeout(Duration::from_secs(1))
        .expect("concurrent Stop sender started");

    let shutdown = Arc::new(AtomicBool::new(false));
    let writer = spawn_control_writer(
        client,
        receiver,
        shutdown.clone(),
        counters.clone(),
        Duration::from_millis(20),
        "test-concurrent-authoritative-writer".to_owned(),
    )
    .expect("spawn real TCP writer");
    assert_eq!(
        start_result
            .recv_timeout(Duration::from_secs(1))
            .expect("Start acknowledgement")
            .expect("Start delivered"),
        5
    );
    assert_eq!(
        pause
            .join()
            .expect("Pause sender")
            .expect("Pause delivered"),
        5
    );
    assert_eq!(
        stop.join().expect("Stop sender").expect("Stop delivered"),
        4
    );
    let mut bytes = [0_u8; 14];
    server
        .read_exact(&mut bytes)
        .expect("all three authoritative TCP controls");
    assert_eq!(&bytes, b"startpausestop");
    assert_eq!(counters.snapshot().control_frames_sent, 3);
    assert_eq!(counters.snapshot().delivery_failures, 0);
    shutdown.store(true, Ordering::Release);
    writer.join().expect("writer exits");
}
