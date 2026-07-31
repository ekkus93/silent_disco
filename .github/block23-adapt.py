#!/usr/bin/env python3
from __future__ import annotations

import ast
import base64
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PAYLOAD = ROOT / ".github/apply-block23.py"


def replace_exact(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one replacement target, found {count}")
    return source.replace(old, new, 1)


def remove_replace_calls(source: str, target_path: str) -> tuple[str, int]:
    tree = ast.parse(source)
    ranges: list[tuple[int, int]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Expr) or not isinstance(node.value, ast.Call):
            continue
        call = node.value
        if not isinstance(call.func, ast.Name) or call.func.id != "replace_once" or not call.args:
            continue
        first = call.args[0]
        if isinstance(first, ast.Constant) and first.value == target_path:
            if node.end_lineno is None:
                raise SystemExit(f"{target_path}: AST replacement call has no end line")
            ranges.append((node.lineno, node.end_lineno))
    lines = source.splitlines(keepends=True)
    for start, end in sorted(ranges, reverse=True):
        del lines[start - 1 : end]
    return "".join(lines), len(ranges)


def emit_replace(path: str, old: str, new: str) -> str:
    return f"replace_once({path!r}, {old!r}, {new!r})\n"


source = PAYLOAD.read_text(encoding="utf-8")

bindings_old = base64.b64decode(
    "cmVwbGFjZV9vbmNlKAogICAgImRlc2t0b3Avc3JjLXRhdXJpL3NyYy9iaW5kaW5ncy5ycyIsCiAgICAnJycgICAgUGxhdGZvcm1FZmZlY3REdG8sIFJldmlzaW9uQ29tbWFuZFJlcXVlc3QsIFVwZGF0ZUhvc3REcmFmdFJlcXVlc3QsCn07JycnLAogICAgJycnICAgIEFwcHJvdmVKb2luUmVxdWVzdCwgSm9pblJlcXVlc3RDb21tYW5kUmVxdWVzdCwgTGlzdGVuZXJDb21tYW5kUmVxdWVzdCwKICAgIFBsYXRmb3JtRWZmZWN0RHRvLCBSZXZpc2lvbkNvbW1hbmRSZXF1ZXN0LCBVcGRhdGVIb3N0RHJhZnRSZXF1ZXN0LAp9OycnJywKKQ=="
).decode("utf-8")
bindings_new = base64.b64decode(
    "cmVwbGFjZV9vbmNlKAogICAgImRlc2t0b3Avc3JjLXRhdXJpL3NyYy9iaW5kaW5ncy5ycyIsCiAgICAnJycgICAgSG9zdERyYWZ0VmFsaWRhdGlvbkR0bywgT3BlblByb2ZpbGVSZXF1ZXN0LCBPcGVuUHJvZmlsZVJlc3BvbnNlLCBQbGF0Zm9ybUVmZmVjdER0bywKICAgIFJldmlzaW9uQ29tbWFuZFJlcXVlc3QsIFVwZGF0ZUhvc3REcmFmdFJlcXVlc3QsCn07JycnLAogICAgJycnICAgIEFwcHJvdmVKb2luUmVxdWVzdCwgSG9zdERyYWZ0VmFsaWRhdGlvbkR0bywgSm9pblJlcXVlc3RDb21tYW5kUmVxdWVzdCwKICAgIExpc3RlbmVyQ29tbWFuZFJlcXVlc3QsIE9wZW5Qcm9maWxlUmVxdWVzdCwgT3BlblByb2ZpbGVSZXNwb25zZSwgUGxhdGZvcm1FZmZlY3REdG8sCiAgICBSZXZpc2lvbkNvbW1hbmRSZXF1ZXN0LCBVcGRhdGVIb3N0RHJhZnRSZXF1ZXN0LAp9OycnJywKKQ=="
).decode("utf-8")
source = replace_exact(source, bindings_old, bindings_new, "bindings compatibility")

network_start = "# Patch network ownership and clock propagation."
network_end = "# Route transport and storage notifications to owned workers."
if source.count(network_start) != 1 or source.count(network_end) != 1:
    raise SystemExit("network compatibility markers are not unique")
before, tail = source.split(network_start, 1)
_, after = tail.split(network_end, 1)
source = before + "# Current-layout network compatibility is appended below.\n" + network_end + after

source, effect_call_count = remove_replace_calls(
    source, "desktop/src-tauri/src/platform/effect_runner.rs"
)
if effect_call_count == 0:
    raise SystemExit("effect runner compatibility: no stale replacement calls found")

patches: list[str] = []

patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/host_transport.rs",
        """pub(crate) struct HostTransportStatus {
    pub(crate) running: bool,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug)]
""",
        """pub(crate) struct HostTransportStatus {
    pub(crate) running: bool,
    pub(crate) last_error: Option<String>,
}

pub(crate) struct ActiveHostSessionSnapshot {
    pub(crate) advertisement: SessionAdvertisement,
    pub(crate) endpoint: silent_disco_core::runtime::NetworkEndpoint,
    pub(crate) worker_running: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) observed_at_ms: u64,
}

#[derive(Debug)]
""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        "use silent_disco_core::runtime::{CoreActorHandle, NetworkEndpoint, SessionAdvertisement};",
        """use silent_disco_core::runtime::{
    CoreActorHandle, NetworkEndpoint, SessionAdvertisement, TransportEffect,
};""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """    HostTransportConfig, SystemTransportClock, TransportFactory, production_transport_factory,
};""",
        """    HostTransportConfig, SystemTransportClock, TransportClock, TransportFactory,
    production_transport_factory,
};""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """struct ActiveBinding {
    selected: SelectedAddress,
    runtime: DesktopHostTransportRuntime,
}""",
        """struct ActiveBinding {
    selected: SelectedAddress,
    advertisement: SessionAdvertisement,
    runtime: DesktopHostTransportRuntime,
}""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """        let node = self
            .transport_factory
            .bind_host(config, Arc::new(SystemTransportClock::default()))
            .map_err(|error| DesktopNetworkError::transport(&error))?;""",
        """        let clock: Arc<dyn TransportClock> = Arc::new(SystemTransportClock::default());
        let node = self
            .transport_factory
            .bind_host(config, Arc::clone(&clock))
            .map_err(|error| DesktopNetworkError::transport(&error))?;""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """        let runtime = DesktopHostTransportRuntime::start(node, advertisement.clone(), sink)?;
        state.active = Some(ActiveBinding { selected, runtime });""",
        """        let runtime = DesktopHostTransportRuntime::start(
            node,
            advertisement.clone(),
            sink,
            clock,
        )?;
        state.active = Some(ActiveBinding {
            selected,
            advertisement: advertisement.clone(),
            runtime,
        });""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """        state
            .active
            .as_ref()
            .map(|active| active.runtime.snapshot().map_err(DesktopNetworkError::dto))
            .transpose()""",
        """        let Some(active) = state.active.as_ref() else {
            return Ok(None);
        };
        let status = active
            .runtime
            .status()
            .map_err(DesktopNetworkError::dto)?;
        Ok(Some(ActiveHostSessionSnapshot {
            advertisement: active.advertisement.clone(),
            endpoint: active.runtime.endpoint(),
            worker_running: status.running,
            last_error: status.last_error,
            observed_at_ms: active.runtime.observed_at().get(),
        }))""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/network.rs",
        """    pub(crate) fn shutdown(&self) -> Result<(), CoreError> {""",
        """    pub(crate) fn dispatch_transport_effect(
        &self,
        effect: TransportEffect,
    ) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        let state = self.state.lock().map_err(|_| {
            DesktopNetworkError::poisoned().core_error(Some(operation_id.clone()))
        })?;
        let Some(active) = state.active.as_ref() else {
            return Err(
                DesktopNetworkError::unavailable(
                    "transport effect requires an active desktop host session",
                )
                .core_error(Some(operation_id)),
            );
        };
        active.runtime.dispatch(effect)
    }

    pub(crate) fn shutdown(&self) -> Result<(), CoreError> {""",
    )
)

patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/effect_runner.rs",
        "use super::network::DesktopHostNetworkControl;",
        """use super::network::DesktopHostNetworkControl;
use super::storage_effect_runner::DesktopStorageEffectDispatcher;""",
    )
)
patches.append(
    emit_replace(
        "desktop/src-tauri/src/platform/effect_runner.rs",
        """pub(crate) struct DesktopCoreObserver {
    notifications: Arc<DesktopNotificationBuffer>,
    platform_effects: DesktopPlatformEffectDispatcher,
}

impl DesktopCoreObserver {
    #[must_use]
    pub(crate) fn new(
        notifications: Arc<DesktopNotificationBuffer>,
        platform_effects: DesktopPlatformEffectDispatcher,
    ) -> Self {
        Self {
            notifications,
            platform_effects,
        }
    }
}

impl CoreObserver for DesktopCoreObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        match notification {
            CoreNotification::Effect(effect) => self.platform_effects.dispatch(effect),
            other => self.notifications.on_notification(other),
        }
    }
}""",
        """pub(crate) struct DesktopCoreObserver {
    notifications: Arc<DesktopNotificationBuffer>,
    platform_effects: DesktopPlatformEffectDispatcher,
    transport_effects: Arc<DesktopHostNetworkControl>,
    storage_effects: DesktopStorageEffectDispatcher,
}

impl DesktopCoreObserver {
    #[must_use]
    pub(crate) fn new(
        notifications: Arc<DesktopNotificationBuffer>,
        platform_effects: DesktopPlatformEffectDispatcher,
        transport_effects: Arc<DesktopHostNetworkControl>,
        storage_effects: DesktopStorageEffectDispatcher,
    ) -> Self {
        Self {
            notifications,
            platform_effects,
            transport_effects,
            storage_effects,
        }
    }
}

impl CoreObserver for DesktopCoreObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        match notification {
            CoreNotification::Effect(effect) => self.platform_effects.dispatch(effect),
            CoreNotification::TransportEffect(effect) => {
                self.transport_effects.dispatch_transport_effect(effect)
            }
            CoreNotification::StorageEffect(effect) => self.storage_effects.dispatch(effect),
            other => self.notifications.on_notification(other),
        }
    }
}""",
    )
)

source += "\n# Current repository layout compatibility patches.\n" + "".join(patches)
PAYLOAD.write_text(source, encoding="utf-8")
print(
    f"adapted Block 23 payload: removed {effect_call_count} stale effect-runner calls and appended {len(patches)} current-layout patches"
)
