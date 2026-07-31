from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"timeout anchor count {count} for {path}: {old[:140]!r}")
    target.write_text(source.replace(old, new, 1))


replace_once(
    "rust/silent-disco-core/src/transport/types.rs",
    """impl TransportError {
    pub(crate) fn new(
""",
    """impl TransportError {
    /// Creates a typed timeout result for an adapter whose wait interval elapsed.
    #[must_use]
    pub fn timeout(channel: TransportChannel, message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Timeout, channel, message)
    }

    pub(crate) fn new(
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network_tests.rs",
    """        Err(TransportError::new(
            silent_disco_core::transport::TransportErrorKind::Timeout,
            silent_disco_core::transport::TransportChannel::Runtime,
            "fake host receive timed out",
        ))
""",
    """        Err(TransportError::timeout(
            silent_disco_core::transport::TransportChannel::Runtime,
            "fake host receive timed out",
        ))
""",
)
