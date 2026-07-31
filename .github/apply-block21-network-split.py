from pathlib import Path

NETWORK_PATH = Path("desktop/src-tauri/src/platform/network.rs")
MOD_PATH = Path("desktop/src-tauri/src/platform/mod.rs")
ERROR_PATH = Path("desktop/src-tauri/src/platform/network_error.rs")

network = NETWORK_PATH.read_text()

network = network.replace(
    "use super::failure::DesktopPlatformFailure;\n",
    "use super::failure::DesktopPlatformFailure;\n"
    "pub(super) use super::network_error::{DesktopNetworkError, NetworkErrorKind};\n",
    1,
)
network = network.replace(
    "use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};\n",
    "use silent_disco_core::error::CoreError;\n",
    1,
)
network = network.replace(
    "    HostTransportConfig, HostTransportNode, SystemTransportClock, TransportError,\n"
    "    TransportErrorKind, TransportFactory, production_transport_factory,\n",
    "    HostTransportConfig, HostTransportNode, SystemTransportClock, TransportFactory,\n"
    "    production_transport_factory,\n",
    1,
)
network = network.replace("use std::fmt;\n", "", 1)

start_marker = "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub(super) enum NetworkErrorKind"
end_marker = "#[cfg(test)]\npub(super) use HostPorts as TestHostPorts;"
start = network.find(start_marker)
end = network.find(end_marker)
if start == -1 or end == -1 or end <= start:
    raise SystemExit("Block 21 network error extraction markers were not found")
network = network[:start] + network[end:]
NETWORK_PATH.write_text(network)

mod_source = MOD_PATH.read_text()
needle = "pub mod network;\npub mod network_dto;\n"
replacement = "pub mod network;\nmod network_error;\npub mod network_dto;\n"
if needle not in mod_source:
    raise SystemExit("Block 21 platform module insertion point was not found")
MOD_PATH.write_text(mod_source.replace(needle, replacement, 1))

ERROR_PATH.write_text(
    """use super::failure::DesktopPlatformFailure;
use crate::dto::DesktopErrorDto;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::transport::{TransportError, TransportErrorKind};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NetworkErrorKind {
    InvalidArgument,
    InvalidState,
    Unavailable,
    Ambiguous,
    ResourceLimit,
    Transport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DesktopNetworkError {
    pub(super) kind: NetworkErrorKind,
    pub(super) message: String,
    code: CoreErrorCode,
    retryable: bool,
}

impl DesktopNetworkError {
    pub(super) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::InvalidArgument,
            CoreErrorCode::InvalidArgument,
            message,
            false,
        )
    }

    pub(super) fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::InvalidState,
            CoreErrorCode::InvalidStateTransition,
            message,
            false,
        )
    }

    pub(super) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::Unavailable,
            CoreErrorCode::TransportUnavailable,
            message,
            true,
        )
    }

    pub(super) fn ambiguous(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::Ambiguous,
            CoreErrorCode::InvalidArgument,
            message,
            false,
        )
    }

    pub(super) fn resource_limit(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::ResourceLimit,
            CoreErrorCode::ResourceLimitExceeded,
            message,
            false,
        )
    }

    pub(super) fn poisoned() -> Self {
        Self::new(
            NetworkErrorKind::InvalidState,
            CoreErrorCode::WorkerStopped,
            "desktop host network state mutex was poisoned",
            false,
        )
    }

    pub(super) fn transport(error: &TransportError) -> Self {
        let code = match error.kind {
            TransportErrorKind::Bind | TransportErrorKind::Listen => {
                CoreErrorCode::TransportUnavailable
            }
            TransportErrorKind::Timeout => CoreErrorCode::TransportTimeout,
            TransportErrorKind::ShuttingDown | TransportErrorKind::WorkerPanicked => {
                CoreErrorCode::ShutdownFailed
            }
            _ => CoreErrorCode::TransportConnectionFailed,
        };
        Self::new(NetworkErrorKind::Transport, code, error.to_string(), true)
    }

    pub(super) fn endpoint_mismatch(cleanup: Option<&TransportError>) -> Self {
        let message = cleanup.map_or_else(
            || "shared transport returned an endpoint for a different bind address".to_owned(),
            |cleanup| format!(
                "shared transport returned an endpoint for a different bind address; cleanup also failed: {cleanup}"
            ),
        );
        Self::new(
            NetworkErrorKind::Transport,
            CoreErrorCode::TransportUnavailable,
            message,
            false,
        )
    }

    fn new(
        kind: NetworkErrorKind,
        code: CoreErrorCode,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            retryable,
        }
    }

    pub(super) fn platform_failure(&self) -> DesktopPlatformFailure {
        DesktopPlatformFailure::new(
            self.code,
            self.message.clone(),
            ErrorSeverity::Error,
            self.retryable,
        )
    }

    pub(super) fn dto(self) -> DesktopErrorDto {
        DesktopErrorDto::new(
            &format!("desktop.network.{}", self.code.stable_name()),
            "transport",
            "error",
            self.retryable,
            &self.message,
        )
    }

    pub(super) fn core_error(
        self,
        operation_id: Option<silent_disco_core::domain::OperationId>,
    ) -> CoreError {
        let message = bounded_error_message(&self.message);
        CoreError::new(
            self.code,
            message,
            ErrorSeverity::Error,
            self.retryable,
            operation_id,
        )
        .expect("bounded desktop network error")
    }
}

fn bounded_error_message(message: &str) -> String {
    let mut output = String::new();
    for character in message.chars() {
        let next = character.len_utf8();
        if output.len().saturating_add(next) > silent_disco_core::error::MAX_ERROR_MESSAGE_BYTES {
            break;
        }
        output.push(character);
    }
    if output.is_empty() {
        "desktop network operation failed".to_owned()
    } else {
        output
    }
}

impl fmt::Display for DesktopNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DesktopNetworkError {}
"""
)
