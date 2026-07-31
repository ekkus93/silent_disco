from pathlib import Path

ROOT = Path("rust/silent-disco-core/src/transport")


def replace_once(path: Path, old: str, new: str) -> None:
    source = path.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"repair anchor count {count} for {path}: {old[:80]!r}")
    path.write_text(source.replace(old, new))


def replace_count(path: Path, old: str, new: str, expected: int) -> None:
    source = path.read_text()
    count = source.count(old)
    if count != expected:
        raise SystemExit(f"repair anchor count {count}, expected {expected}, for {path}: {old[:80]!r}")
    path.write_text(source.replace(old, new))


boundary = ROOT / "boundary.rs"
replace_once(
    boundary,
    "pub trait HostTransportNode: Send {",
    "#[allow(clippy::missing_errors_doc)]\npub trait HostTransportNode: Send {",
)
replace_once(
    boundary,
    "pub trait ListenerTransportNode: Send {",
    "#[allow(clippy::missing_errors_doc)]\npub trait ListenerTransportNode: Send {",
)
replace_once(
    boundary,
    "pub trait TransportFactory: Send + Sync {",
    "#[allow(clippy::missing_errors_doc)]\npub trait TransportFactory: Send + Sync {",
)

host = ROOT / "socket/host.rs"
replace_once(
    host,
    "impl SocketHostTransport {\n    pub fn bind(\n",
    """impl SocketHostTransport {
    /// Binds the TCP control listener and UDP synchronization/audio sockets.
    ///
    /// # Errors
    ///
    /// Returns a typed transport error when configuration validation, socket
    /// binding, endpoint inspection, or worker startup fails.
    // Socket acquisition and rollback stay linear so ownership is auditable.
    #[allow(clippy::too_many_lines)]
    pub fn bind(
""",
)
replace_once(
    host,
    """            match result {
                Ok(written) => {
                    successful = successful.saturating_add(1);
                    sent_bytes = sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                }
                Err(_) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                }
            }
""",
    """            if let Ok(written) = result {
                successful = successful.saturating_add(1);
                sent_bytes = sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
            } else {
                failed = failed.saturating_add(1);
                self.counters.delivery_failure();
            }
""",
)

host_workers = ROOT / "socket/host_workers.rs"
replace_once(
    host_workers,
    "#[allow(clippy::too_many_arguments)]\nfn register_peer(\n",
    """// Peer registration is a linear ownership-transfer state machine.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn register_peer(
""",
)
replace_once(
    host_workers,
    "#[allow(clippy::too_many_arguments)]\npub(super) fn spawn_udp_receiver(\n",
    """// The receive loop keeps protocol, authorization, and failure transitions adjacent.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn spawn_udp_receiver(
""",
)

listener = ROOT / "socket/listener.rs"
replace_once(
    listener,
    "impl SocketListenerTransport {\n    pub fn connect(\n",
    """impl SocketListenerTransport {
    /// Connects the TCP control channel and binds listener UDP routes.
    ///
    /// # Errors
    ///
    /// Returns a typed transport error when configuration validation, socket
    /// connection/binding, endpoint inspection, or worker startup fails.
    // Socket acquisition and rollback stay linear so ownership is auditable.
    #[allow(clippy::too_many_lines)]
    pub fn connect(
""",
)
replace_once(
    listener,
    """        let mut workers = Vec::new();
        workers.push(spawn_control_writer(
            writer_stream,
            write_receiver,
            stop.clone(),
            counters.clone(),
            config.io_timeout,
            "silent-disco-listener-control-writer".to_owned(),
        )?);
        workers.push(spawn_control_reader(
            stream,
            config.clone(),
            stop.clone(),
            counters.clone(),
            event_sender.clone(),
            clock.clone(),
        )?);
        workers.push(spawn_listener_udp_receiver(
            sync_socket.clone(),
            TransportChannel::Synchronization,
            config.clone(),
            stop.clone(),
            counters.clone(),
            event_sender.clone(),
            clock.clone(),
        )?);
        workers.push(spawn_listener_udp_receiver(
            audio_socket.clone(),
            TransportChannel::Audio,
            config.clone(),
            stop.clone(),
            counters.clone(),
            event_sender,
            clock,
        )?);
""",
    """        let workers = vec![
            spawn_control_writer(
                writer_stream,
                write_receiver,
                stop.clone(),
                counters.clone(),
                config.io_timeout,
                "silent-disco-listener-control-writer".to_owned(),
            )?,
            spawn_control_reader(
                stream,
                config.clone(),
                stop.clone(),
                counters.clone(),
                event_sender.clone(),
                clock.clone(),
            )?,
            spawn_listener_udp_receiver(
                sync_socket.clone(),
                TransportChannel::Synchronization,
                config.clone(),
                stop.clone(),
                counters.clone(),
                event_sender.clone(),
                clock.clone(),
            )?,
            spawn_listener_udp_receiver(
                audio_socket.clone(),
                TransportChannel::Audio,
                config.clone(),
                stop.clone(),
                counters.clone(),
                event_sender,
                clock,
            )?,
        ];
""",
)
replace_once(
    listener,
    "fn spawn_listener_udp_receiver(\n",
    """// The receive loop keeps source validation, framing, and delivery accounting adjacent.
#[allow(clippy::too_many_lines)]
fn spawn_listener_udp_receiver(
""",
)

shared = ROOT / "socket/shared.rs"
replace_once(
    shared,
    "pub(super) fn read_control_loop<F>(\n",
    """// The framed TCP parser is intentionally linear so partial-read transitions stay auditable.
#[allow(clippy::too_many_lines)]
pub(super) fn read_control_loop<F>(
""",
)
replace_once(
    shared,
    "drop(on_outcome(ReadControlOutcome::Closed));",
    "let _ = on_outcome(ReadControlOutcome::Closed);",
)
replace_count(
    shared,
    "drop(on_outcome(ReadControlOutcome::Rejected(error)));",
    "let _ = on_outcome(ReadControlOutcome::Rejected(error));",
    4,
)
replace_once(
    shared,
    """        drop(on_outcome(ReadControlOutcome::Rejected(TransportError::io(
            TransportErrorKind::Io,
            TransportChannel::Control,
            "failed to set control read timeout",
            &error,
        ))));
""",
    """        let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::io(
            TransportErrorKind::Io,
            TransportChannel::Control,
            "failed to set control read timeout",
            &error,
        )));
""",
)
replace_once(
    shared,
    """                            drop(on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            )));
""",
    """                            let _ = on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            ));
""",
)
replace_once(
    shared,
    """                drop(on_outcome(ReadControlOutcome::Rejected(TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Control,
                    "failed to read control connection",
                    &error,
                ))));
""",
    """                let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Control,
                    "failed to read control connection",
                    &error,
                )));
""",
)
replace_once(
    shared,
    """                    let payload_length = match usize::try_from(header.payload_length) {
                        Ok(length) => length,
                        Err(_) => {
                            let error = TransportError::new(
                                TransportErrorKind::Protocol,
                                TransportChannel::Control,
                                "control payload length cannot be represented on this platform",
                            );
                            let _ = on_outcome(ReadControlOutcome::Rejected(error));
                            return;
                        }
                    };
""",
    """                    let Ok(payload_length) = usize::try_from(header.payload_length) else {
                        let error = TransportError::new(
                            TransportErrorKind::Protocol,
                            TransportChannel::Control,
                            "control payload length cannot be represented on this platform",
                        );
                        let _ = on_outcome(ReadControlOutcome::Rejected(error));
                        return;
                    };
""",
)

tests = ROOT / "tests.rs"
replace_once(
    tests,
    "#[test]\nfn socket_runtime_completes_multi_listener_join_sync_and_audio_exchange() {\n",
    "#[test]\n#[allow(clippy::too_many_lines)]\nfn socket_runtime_completes_multi_listener_join_sync_and_audio_exchange() {\n",
)
