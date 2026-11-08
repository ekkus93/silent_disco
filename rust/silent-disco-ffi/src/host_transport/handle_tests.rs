//! Host transport handle behavior tests (moved out of `handle.rs` to keep it
//! below the source-size gate).

use super::handle::FfiHostTransportHandle;
use std::net::{TcpListener, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

fn free_tcp_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral tcp port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind ephemeral udp port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Block 24: races `shutdown` against a thread continuously issuing real
/// transport operations -- broadcasts over real loopback sockets and
/// event polling -- against a bound host with no connected peer. This is
/// exactly the moment the accept-loop, outbound, and event-queue worker
/// threads are all live and busy, which is what "shutdown during network
/// load" means for a host transport that owns its own worker threads
/// rather than borrowing a shared runtime. No operation may panic,
/// `shutdown` itself must complete in bounded time regardless of the
/// concurrent load, and a second shutdown afterward must be an explicit
/// no-op rather than reaching into torn-down worker state.
#[test]
fn shutdown_races_with_concurrent_broadcasts_and_never_panics_or_hangs() {
    for iteration in 0_u32..5 {
        let handle = FfiHostTransportHandle::bind(
            "127.0.0.1".to_owned(),
            free_tcp_port(),
            free_udp_port(),
            free_udp_port(),
            format!("session-shutdown-race-{iteration}"),
        )
        .expect("host transport binds on loopback");

        let running = Arc::new(AtomicBool::new(true));
        let load_flag = Arc::clone(&running);
        let load_handle = Arc::clone(&handle);
        let load_thread = thread::spawn(move || {
            let mut sequence = 0_u64;
            while load_flag.load(Ordering::SeqCst) {
                let _ = load_handle.broadcast_audio(
                    "stream-shutdown-race".to_owned(),
                    sequence,
                    48_000,
                    2,
                    960,
                    sequence * 960,
                    sequence * 20,
                    vec![0_u8; 64],
                );
                let _ = load_handle.poll_event(0);
                sequence += 1;
            }
        });

        // Let real broadcasts and polls actually run before racing the
        // shutdown.
        thread::sleep(Duration::from_millis(5));
        let started = Instant::now();
        handle
            .shutdown()
            .expect("shutdown succeeds under concurrent network load");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "shutdown took {elapsed:?} while racing concurrent broadcasts/polls"
        );

        running.store(false, Ordering::SeqCst);
        load_thread.join().expect("load thread must not panic");

        handle
            .shutdown()
            .expect("a repeated shutdown is a no-op, not an error");
    }
}
