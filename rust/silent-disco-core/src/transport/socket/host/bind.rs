use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::NetworkEndpoint;

use super::super::super::{
    HostTransportConfig, TransportChannel, TransportClock, TransportError, TransportErrorKind,
};
use super::super::host_workers::{spawn_accept_loop, spawn_udp_receiver, worker_registry_error};
use super::super::shared::CounterState;
use super::SocketHostTransport;

/// Bounds how long a single UDP `send_to` may block on `sync_socket`/
/// `audio_socket`. Neither socket is ever set non-blocking, because
/// `set_nonblocking` shares the same underlying OS file description as the
/// existing blocking-with-timeout `recv_from` reader loop (`io_timeout`,
/// default 100ms) -- flipping it would make reads non-blocking too, not
/// just sends. A write timeout is independent (`SO_SNDTIMEO`) and only
/// bounds sends.
///
/// Sized to the packetizer's own default cadence (`DEFAULT_PACKET_DURATION_MS`,
/// 5ms): confirmed against real hardware (a real, older Android phone over
/// real Wi-Fi, 2026-08-09) that a blocking send with no write timeout can
/// stall for 200-700ms when the OS send buffer backs up against a slow
/// receiver, which starves this worker's own broadcast queue (bounded at
/// 64 frames) far faster than it can be produced, and the resulting
/// silently-dropped/delayed audio was independently confirmed audible
/// ("choppy and staticy, breaking up a lot") on the same run. A send that
/// cannot complete within one packet period is not going to help keep pace
/// regardless of how much longer it is given, so failing fast here (already
/// handled as a counted, visible per-peer delivery failure, not a panic or
/// a silent drop) is strictly better than blocking the whole worker.
const DATAGRAM_SEND_TIMEOUT: Duration = Duration::from_millis(5);

impl SocketHostTransport {
    pub fn bind(
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let listener = TcpListener::bind(SocketAddr::new(config.bind_address, config.control_port))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Control,
                    "failed to bind TCP control listener",
                    &error,
                )
            })?;
        listener.set_nonblocking(true).map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to configure TCP control listener",
                &error,
            )
        })?;
        let sync_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.bind_address, config.sync_port)).map_err(
                |error| {
                    TransportError::io(
                        TransportErrorKind::Bind,
                        TransportChannel::Synchronization,
                        "failed to bind UDP synchronization endpoint",
                        &error,
                    )
                },
            )?,
        );
        let audio_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.bind_address, config.audio_port)).map_err(
                |error| {
                    TransportError::io(
                        TransportErrorKind::Bind,
                        TransportChannel::Audio,
                        "failed to bind UDP audio endpoint",
                        &error,
                    )
                },
            )?,
        );
        sync_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Synchronization,
                    "failed to configure synchronization read timeout",
                    &error,
                )
            })?;
        sync_socket
            .set_write_timeout(Some(DATAGRAM_SEND_TIMEOUT))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Synchronization,
                    "failed to configure synchronization write timeout",
                    &error,
                )
            })?;
        audio_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Audio,
                    "failed to configure audio read timeout",
                    &error,
                )
            })?;
        audio_socket
            .set_write_timeout(Some(DATAGRAM_SEND_TIMEOUT))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Audio,
                    "failed to configure audio write timeout",
                    &error,
                )
            })?;

        let control_address = listener.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to query bound control address",
                &error,
            )
        })?;
        let sync_address = sync_socket.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Synchronization,
                "failed to query bound synchronization address",
                &error,
            )
        })?;
        let audio_address = audio_socket.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Audio,
                "failed to query bound audio address",
                &error,
            )
        })?;
        let endpoint = NetworkEndpoint::new(
            control_address.ip(),
            control_address.port(),
            sync_address.port(),
            audio_address.port(),
        )
        .map_err(|error| {
            TransportError::new(
                TransportErrorKind::Bind,
                TransportChannel::Runtime,
                error.to_string(),
            )
        })?;

        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CounterState::default());
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let (event_sender, event_receiver) = sync_channel(config.event_queue_capacity);
        let next_peer = Arc::new(AtomicU64::new(1));

        let accept_handle = spawn_accept_loop(
            listener,
            config.clone(),
            stop.clone(),
            counters.clone(),
            peers.clone(),
            routes.clone(),
            workers.clone(),
            event_sender.clone(),
            clock.clone(),
            next_peer,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(accept_handle);

        let sync_handle = spawn_udp_receiver(
            sync_socket.clone(),
            TransportChannel::Synchronization,
            config.session_id.clone(),
            stop.clone(),
            counters.clone(),
            routes.clone(),
            event_sender.clone(),
            clock.clone(),
            config.io_timeout,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(sync_handle);

        let event_sender_for_runtime = event_sender.clone();
        let clock_for_runtime = clock.clone();
        let audio_handle = spawn_udp_receiver(
            audio_socket.clone(),
            TransportChannel::Audio,
            config.session_id.clone(),
            stop.clone(),
            counters.clone(),
            routes.clone(),
            event_sender,
            clock,
            config.io_timeout,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(audio_handle);

        Ok(Self {
            endpoint,
            session_id: config.session_id.clone(),
            config,
            stop,
            counters,
            peers,
            routes,
            sync_socket,
            audio_socket,
            event_sender: event_sender_for_runtime,
            event_receiver,
            clock: clock_for_runtime,
            workers,
            shutdown_complete: false,
        })
    }
}
