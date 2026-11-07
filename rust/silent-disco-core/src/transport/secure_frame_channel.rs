//! Owner-thread TLS frame channel.
//!
//! One worker thread owns the TCP socket and the rustls connection. Callers
//! never touch the socket: they send frames through a bounded queue and
//! receive frames from another. This avoids cloning a TLS stream, which the
//! existing `TcpStream` reader/writer split cannot do.
//!
//! Frames are length-prefixed (`u16`, big-endian) and capped at
//! [`MAX_SECURE_FRAME_BYTES`]. An oversized frame is refused at send time and
//! never reaches the wire.
//!
//! Scope: this is the connection layer only. It is not yet used by the host's
//! control path; `host_workers` still runs plaintext v3.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustls::Connection;

/// Largest application frame accepted in either direction.
pub const MAX_SECURE_FRAME_BYTES: usize = 65_535;

/// Read poll interval. The worker alternates between writing queued frames and
/// reading, so this bounds how long a queued frame can wait.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Frames waiting in each direction before a sender is refused.
const QUEUE_CAPACITY: usize = 64;

#[derive(Debug)]
pub enum SecureChannelError {
    /// The frame is empty or larger than [`MAX_SECURE_FRAME_BYTES`].
    FrameTooLarge {
        length: usize,
    },
    /// The worker has stopped; the connection is closed.
    Closed,
    /// The outbound queue is full. The caller must back off or fail.
    QueueFull,
    /// The TLS handshake or record layer failed.
    Tls(rustls::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for SecureChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FrameTooLarge { length } => write!(
                formatter,
                "frame of {length} bytes is outside 1..={MAX_SECURE_FRAME_BYTES}"
            ),
            Self::Closed => formatter.write_str("secure channel is closed"),
            Self::QueueFull => formatter.write_str("secure channel outbound queue is full"),
            Self::Tls(error) => write!(formatter, "TLS failed: {error}"),
            Self::Io(error) => write!(formatter, "socket failed: {error}"),
        }
    }
}

impl std::error::Error for SecureChannelError {}

/// Handle to a running secure frame channel.
pub struct SecureFrameChannel {
    outbound: SyncSender<Vec<u8>>,
    inbound: Receiver<Result<Vec<u8>, SecureChannelError>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SecureFrameChannel {
    /// Queues one frame for the peer.
    ///
    /// # Errors
    /// Returns `FrameTooLarge` for empty or oversized frames, `QueueFull` when
    /// the bounded queue is full, and `Closed` once the worker has stopped.
    pub fn send(&self, frame: Vec<u8>) -> Result<(), SecureChannelError> {
        if frame.is_empty() || frame.len() > MAX_SECURE_FRAME_BYTES {
            return Err(SecureChannelError::FrameTooLarge {
                length: frame.len(),
            });
        }
        self.outbound.try_send(frame).map_err(|error| match error {
            TrySendError::Full(_) => SecureChannelError::QueueFull,
            TrySendError::Disconnected(_) => SecureChannelError::Closed,
        })
    }

    /// Waits up to `timeout` for the next frame from the peer.
    ///
    /// # Errors
    /// Returns the worker's failure once, or `Closed` after a clean close.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Vec<u8>, SecureChannelError> {
        match self.inbound.recv_timeout(timeout) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(SecureChannelError::Io(
                std::io::Error::new(ErrorKind::TimedOut, "no frame before the deadline"),
            )),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(SecureChannelError::Closed),
        }
    }

    /// Stops the worker. It sends `close_notify` and exits within one poll
    /// interval.
    pub fn close(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for SecureFrameChannel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Starts the owner thread for `tcp` with an established-or-handshaking
/// `connection`. The handshake runs on the worker, so this returns at once.
///
/// # Errors
/// Returns an I/O error if the socket cannot take the poll timeout or the
/// thread cannot be spawned.
pub fn spawn_secure_frame_channel(
    mut tcp: TcpStream,
    mut connection: Connection,
) -> Result<SecureFrameChannel, SecureChannelError> {
    tcp.set_read_timeout(Some(POLL_INTERVAL))
        .map_err(SecureChannelError::Io)?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(SecureChannelError::Io)?;
    tcp.set_nodelay(true).map_err(SecureChannelError::Io)?;
    let (outbound, outbound_rx) = sync_channel::<Vec<u8>>(QUEUE_CAPACITY);
    let (inbound_tx, inbound) = sync_channel::<Result<Vec<u8>, SecureChannelError>>(QUEUE_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = thread::Builder::new()
        .name("silent-disco-secure-frame-channel".to_owned())
        .spawn(move || {
            run_worker(
                &mut tcp,
                &mut connection,
                &outbound_rx,
                &inbound_tx,
                &worker_stop,
            );
        })
        .map_err(SecureChannelError::Io)?;
    Ok(SecureFrameChannel {
        outbound,
        inbound,
        stop,
        worker: Some(worker),
    })
}

fn run_worker(
    tcp: &mut TcpStream,
    connection: &mut Connection,
    outbound: &Receiver<Vec<u8>>,
    inbound: &SyncSender<Result<Vec<u8>, SecureChannelError>>,
    stop: &AtomicBool,
) {
    let mut assembly: VecDeque<u8> = VecDeque::new();
    let mut staged: VecDeque<u8> = VecDeque::new();
    let failure = loop {
        if stop.load(Ordering::Acquire) {
            connection.send_close_notify();
            let _ = flush_tls(tcp, connection);
            break None;
        }
        if let Err(error) = write_pending_frames(connection, outbound, &mut staged) {
            break Some(error);
        }
        if let Err(error) = flush_tls(tcp, connection) {
            break Some(error);
        }
        match connection.read_tls(tcp) {
            Ok(0) => break None,
            Ok(_) => {
                if let Err(error) = connection.process_new_packets() {
                    break Some(SecureChannelError::Tls(error));
                }
            }
            Err(error) if is_poll_timeout(&error) => {}
            Err(error) => break Some(SecureChannelError::Io(error)),
        }
        if let Err(error) = read_plaintext(connection, &mut assembly) {
            break Some(error);
        }
        if let Err(error) = deliver_frames(&mut assembly, inbound) {
            break Some(error);
        }
    };
    if let Some(error) = failure {
        let _ = inbound.send(Err(error));
    }
}

fn is_poll_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Bytes staged for the TLS writer before the caller's queue is drained further.
const MAX_STAGED_OUTBOUND_BYTES: usize = 256 * 1024;

/// Moves queued frames into the TLS writer. Bytes the TLS buffer does not accept
/// stay staged and are retried after the next flush, so a large frame is never
/// dropped or half-written.
fn write_pending_frames(
    connection: &mut Connection,
    outbound: &Receiver<Vec<u8>>,
    staged: &mut VecDeque<u8>,
) -> Result<(), SecureChannelError> {
    while staged.len() < MAX_STAGED_OUTBOUND_BYTES {
        let Ok(frame) = outbound.try_recv() else {
            break;
        };
        let length = u16::try_from(frame.len()).map_err(|_| SecureChannelError::FrameTooLarge {
            length: frame.len(),
        })?;
        staged.extend(length.to_be_bytes());
        staged.extend(frame);
    }
    while !staged.is_empty() {
        let (front, _) = staged.as_slices();
        match connection.writer().write(front) {
            Ok(0) => break,
            Ok(accepted) => {
                staged.drain(..accepted);
            }
            Err(error) if is_poll_timeout(&error) => break,
            Err(error) => return Err(SecureChannelError::Io(error)),
        }
    }
    Ok(())
}

fn flush_tls(tcp: &mut TcpStream, connection: &mut Connection) -> Result<(), SecureChannelError> {
    while connection.wants_write() {
        match connection.write_tls(tcp) {
            Ok(_) => {}
            Err(error) if is_poll_timeout(&error) => break,
            Err(error) => return Err(SecureChannelError::Io(error)),
        }
    }
    Ok(())
}

fn read_plaintext(
    connection: &mut Connection,
    assembly: &mut VecDeque<u8>,
) -> Result<(), SecureChannelError> {
    let mut buffer = [0_u8; 4096];
    loop {
        match connection.reader().read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => assembly.extend(&buffer[..count]),
            Err(error) if is_poll_timeout(&error) => return Ok(()),
            Err(error) => return Err(SecureChannelError::Io(error)),
        }
    }
}

fn deliver_frames(
    assembly: &mut VecDeque<u8>,
    inbound: &SyncSender<Result<Vec<u8>, SecureChannelError>>,
) -> Result<(), SecureChannelError> {
    loop {
        if assembly.len() < 2 {
            return Ok(());
        }
        let length = usize::from(u16::from_be_bytes([assembly[0], assembly[1]]));
        if length == 0 {
            return Err(SecureChannelError::FrameTooLarge { length: 0 });
        }
        if assembly.len() < 2 + length {
            return Ok(());
        }
        assembly.drain(..2);
        let frame: Vec<u8> = assembly.drain(..length).collect();
        if inbound.send(Ok(frame)).is_err() {
            return Err(SecureChannelError::Closed);
        }
    }
}
