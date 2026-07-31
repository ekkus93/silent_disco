import re
from pathlib import Path

ROOT = Path("rust/silent-disco-core/src/transport")


def replace_once(path: Path, old: str, new: str) -> None:
    source = path.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"base repair anchor count {count} for {path}: {old[:80]!r}")
    path.write_text(source.replace(old, new))


def regex_replace_once(path: Path, pattern: str, replacement: str) -> None:
    source = path.read_text()
    repaired, count = re.subn(pattern, replacement, source, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"base regex repair count {count} for {path}")
    path.write_text(repaired)


shared = ROOT / "socket/shared.rs"
replace_once(
    shared,
    """                            drop(on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            ));
""",
    """                            drop(on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            )));
""",
)
replace_once(
    shared,
    """                drop(on_outcome(ReadControlOutcome::Rejected(TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Control,
                    "failed to read control connection",
                    &error,
                )));
""",
    """                drop(on_outcome(ReadControlOutcome::Rejected(TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Control,
                    "failed to read control connection",
                    &error,
                ))));
""",
)

host = ROOT / "socket/host.rs"
replace_once(
    host,
    "use super::host_workers::{spawn_accept_loop, spawn_udp_receiver, worker_registry_error};",
    """use super::host_workers::{
    shutting_down_error, spawn_accept_loop, spawn_udp_receiver, worker_registry_error,
};""",
)

datagram_pattern = (
    r"\(\s*TransportChannel::Synchronization,\s*ProtocolFrame::SyncRequest\(_\)\s*\)"
    r"\s*\|\s*"
    r"\(\s*TransportChannel::Synchronization,\s*ProtocolFrame::SyncResponse\(_\)\s*\)"
    r"\s*\|\s*"
    r"\(\s*TransportChannel::Audio,\s*ProtocolFrame::Audio\(_\)\s*\)\s*=>\s*\{\}"
)
datagram_replacement = """(
                TransportChannel::Synchronization,
                ProtocolFrame::SyncRequest(_) | ProtocolFrame::SyncResponse(_),
            )
            | (TransportChannel::Audio, ProtocolFrame::Audio(_)) => {}"""
regex_replace_once(host, datagram_pattern, datagram_replacement)
regex_replace_once(ROOT / "virtual_transport.rs", datagram_pattern, datagram_replacement)

host_workers = ROOT / "socket/host_workers.rs"
replace_once(
    host_workers,
    "fn shutting_down_error() -> TransportError {",
    "pub(super) fn shutting_down_error() -> TransportError {",
)
source = host_workers.read_text()
count = source.count("peer_for_reader")
if count != 12:
    raise SystemExit(f"base reader-peer rename count {count}")
host_workers.write_text(source.replace("peer_for_reader", "reader_peer"))

replace_once(
    ROOT / "socket/listener.rs",
    "use std::net::{Shutdown, SocketAddr, TcpStream, UdpSocket};",
    "use std::net::{SocketAddr, TcpStream, UdpSocket};",
)
replace_once(
    shared,
    "use std::net::{Shutdown, SocketAddr, TcpStream};",
    "use std::net::{Shutdown, TcpStream};",
)
replace_once(
    ROOT / "types.rs",
    "#[derive(Debug, Clone, PartialEq, Eq)]\npub enum TransportEvent {",
    "#[derive(Clone, PartialEq, Eq)]\npub enum TransportEvent {",
)
