use super::{ReadControlOutcome, read_control_loop};
use crate::domain::{MonotonicMillis, SessionId, StreamId};
use crate::protocol::{ControlMessage, Hello, ProtocolFrame, Stop, encode_frame};
use crate::transport::TransportErrorKind;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

fn loopback() -> (TcpStream, TcpStream) {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = TcpStream::connect(server.local_addr().expect("address"))
        .expect("connect loopback");
    let (accepted, _) = server.accept().expect("accept loopback");
    (client, accepted)
}

#[test]
fn fragmented_control_and_coalesced_stop_remain_ordered() {
    let (mut client, server) = loopback();
    let session = SessionId::new("framing-session").expect("session");
    let hello = encode_frame(&ProtocolFrame::Control(ControlMessage::Hello(Hello {
        session_id: session.clone(),
        session_name: "session".to_owned(),
        host_name: "host".to_owned(),
        approval_required: true,
    })))
    .expect("encode hello");
    let stop = encode_frame(&ProtocolFrame::Control(ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: StreamId::new("framing-stream").expect("stream"),
        host_stop_time_ms: MonotonicMillis::new(100),
    })))
    .expect("encode stop");
    let worker = thread::spawn(move || {
        let mut received = Vec::new();
        read_control_loop(
            server,
            &AtomicBool::new(false),
            Duration::from_millis(10),
            Duration::from_millis(300),
            &session,
            None,
            |outcome| {
                received.push(match outcome {
                    ReadControlOutcome::Frame(
                        ProtocolFrame::Control(ControlMessage::Hello(_)),
                        _,
                    ) => "hello",
                    ReadControlOutcome::Frame(
                        ProtocolFrame::Control(ControlMessage::Stop(_)),
                        _,
                    ) => "stop",
                    ReadControlOutcome::Closed => "closed",
                    _ => "unexpected",
                });
                true
            },
        );
        received
    });
    client.write_all(&hello[..3]).expect("partial magic");
    thread::sleep(Duration::from_millis(15));
    client.write_all(&hello[3..9]).expect("partial header");
    thread::sleep(Duration::from_millis(15));
    let mut tail = hello[9..].to_vec();
    tail.extend_from_slice(&stop);
    client.write_all(&tail).expect("coalesced controls");
    client.shutdown(Shutdown::Write).expect("finish writes");
    assert_eq!(worker.join().expect("reader"), ["hello", "stop", "closed"]);
}

#[test]
fn joined_peer_cannot_trickle_partial_next_frame_indefinitely() {
    let (mut client, server) = loopback();
    let session = SessionId::new("framing-session").expect("session");
    let hello = encode_frame(&ProtocolFrame::Control(ControlMessage::Hello(Hello {
        session_id: session.clone(),
        session_name: "session".to_owned(),
        host_name: "host".to_owned(),
        approval_required: true,
    })))
    .expect("encode hello");
    let (sender, receiver) = sync_channel(1);
    let worker = thread::spawn(move || {
        let mut frames = 0;
        read_control_loop(
            server,
            &AtomicBool::new(false),
            Duration::from_millis(10),
            Duration::from_millis(80),
            &session,
            None,
            |outcome| match outcome {
                ReadControlOutcome::Frame(_, _) => {
                    frames += 1;
                    true
                }
                ReadControlOutcome::Rejected(error) => {
                    sender.send((frames, error.kind)).expect("report timeout");
                    false
                }
                ReadControlOutcome::Closed => false,
            },
        );
    });
    let mut data = hello;
    data.extend_from_slice(&[b'S', b'D']);
    client.write_all(&data).expect("valid frame then truncated header");
    let result = receiver.recv_timeout(Duration::from_secs(2));
    client.shutdown(Shutdown::Both).expect("close client");
    worker.join().expect("reader exits");
    let (frames, kind) = result.expect("absolute deadline must terminate slow peer");
    assert_eq!(frames, 1, "the complete frame must be delivered first");
    assert_eq!(kind, TransportErrorKind::Timeout);
}
