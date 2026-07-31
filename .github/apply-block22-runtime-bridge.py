from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"integration anchor count {count} for {path}: {old[:140]!r}")
    target.write_text(source.replace(old, new, 1))


replace_once(
    "desktop/src-tauri/src/app_state.rs",
    "use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};\n",
    "use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};\nuse crate::host_session_dto::HostSessionSnapshotDto;\n",
)
app_state = Path("desktop/src-tauri/src/app_state.rs")
source = app_state.read_text()
anchor = """    pub(crate) fn host_network_snapshot(
        &self,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
"""
if source.count(anchor) != 1:
    raise SystemExit("app-state host network anchor not found")
method = """    pub(crate) fn host_session_snapshot(
        &self,
    ) -> Result<HostSessionSnapshotDto, DesktopErrorDto> {
        let (handle, network) = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => {
                    (ready.handle.clone(), Arc::clone(&ready.network))
                }
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        let snapshot = handle.current_snapshot().map_err(DesktopErrorDto::from)?;
        let active = network.active_host_session()?;
        Ok(HostSessionSnapshotDto::from_parts(&snapshot, active.as_ref()))
    }

"""
app_state.write_text(source.replace(anchor, method + anchor, 1))

replace_once(
    "desktop/src-tauri/src/host_commands.rs",
    "use crate::dto::DesktopErrorDto;\n",
    "use crate::dto::DesktopErrorDto;\nuse crate::host_session_dto::HostSessionSnapshotDto;\n",
)
host_commands = Path("desktop/src-tauri/src/host_commands.rs")
source = host_commands.read_text()
anchor = "/// Lists classified network addresses and the current bind policy for the ready profile.\n"
if source.count(anchor) != 1:
    raise SystemExit("host command insertion anchor not found")
commands = """/// Returns the authoritative active host workflow projection.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_host_session_state(
    state: State<'_, DesktopAppState>,
) -> Result<HostSessionSnapshotDto, DesktopErrorDto> {
    state.host_session_snapshot()
}

/// Requests a revision-aware host-session shutdown.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn end_host_session(
    state: State<'_, DesktopAppState>,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::EndHostSession,
    )
}

"""
host_commands.write_text(source.replace(anchor, commands + anchor, 1))
replace_once(
    "desktop/src-tauri/src/lib.rs",
    """            host_commands::create_host_session,
            host_commands::get_host_network_state,
""",
    """            host_commands::create_host_session,
            host_commands::get_host_session_state,
            host_commands::end_host_session,
            host_commands::get_host_network_state,
""",
)
replace_once(
    "desktop/src-tauri/src/bindings.rs",
    "use crate::dto::{\n",
    "use crate::host_session_dto::{\n    ConnectedListenerDto, HostConnectionDto, HostSessionSnapshotDto, PendingJoinRequestDto,\n};\nuse crate::dto::{\n",
)
replace_once(
    "desktop/src-tauri/src/bindings.rs",
    "        declaration::<SetNetworkBindPreferenceRequest>(&config),\n",
    """        declaration::<SetNetworkBindPreferenceRequest>(&config),
        declaration::<HostConnectionDto>(&config),
        declaration::<PendingJoinRequestDto>(&config),
        declaration::<ConnectedListenerDto>(&config),
        declaration::<HostSessionSnapshotDto>(&config),
""",
)
