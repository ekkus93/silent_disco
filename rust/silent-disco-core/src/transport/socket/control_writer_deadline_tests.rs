//! A timed-out control send must not be transmitted from a stale writer backlog.
//! This uses an actual TCP socket and the production bounded writer queue.
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use super::shared::{ControlSender, CounterState, spawn_control_writer};
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
