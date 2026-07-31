from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"candidate repair anchor count {count}: {path}: {old[:80]!r}")
    target.write_text(source.replace(old, new))


# Candidate-local source repairs.
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "        pub(super) kind: NetworkErrorKind,",
    "        kind: NetworkErrorKind,",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "#[derive(Debug, Clone, PartialEq, Eq)]\npub(super) struct InterfaceRecord {",
    "#[derive(Debug, Clone, PartialEq, Eq)]\n#[allow(clippy::struct_excessive_bools)]\npub(super) struct InterfaceRecord {",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """#[derive(Debug, Clone, Copy)]
pub(super) struct HostPorts {
    pub(super) control: u16,
    pub(super) sync: u16,
    pub(super) audio: u16,
}

impl Default for HostPorts {
    fn default() -> Self {
        Self {
            control: 0,
            sync: 0,
            audio: 0,
        }
    }
}
""",
    """#[derive(Debug, Clone, Copy, Default)]
pub(super) struct HostPorts {
    pub(super) control: u16,
    pub(super) sync: u16,
    pub(super) audio: u16,
}
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """            .map(|state| state.active.is_some())
            .unwrap_or(true);""",
    """            .map_or(true, |state| state.active.is_some());""",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    ".map_err(DesktopNetworkError::transport)?;",
    ".map_err(|error| DesktopNetworkError::transport(&error))?;",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "active.node.shutdown().map_err(DesktopNetworkError::transport)",
    "active\n            .node\n            .shutdown()\n            .map_err(|error| DesktopNetworkError::transport(&error))",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "fn transport(error: TransportError) -> Self {",
    "fn transport(error: &TransportError) -> Self {",
)
replace_once(
    "desktop/src-tauri/src/platform/network_dto.rs",
    "#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]\n#[serde(rename_all = \"camelCase\")]\n#[ts(rename_all = \"camelCase\")]\npub struct NetworkAddressCandidateDto {",
    "#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]\n#[allow(clippy::struct_excessive_bools)]\n#[serde(rename_all = \"camelCase\")]\n#[ts(rename_all = \"camelCase\")]\npub struct NetworkAddressCandidateDto {",
)
replace_once(
    "desktop/src-tauri/src/platform/network_tests.rs",
    """use super::network::{
    AddressRecord, DesktopHostNetworkControl, InterfaceRecord, NetworkAddressClassDto,
    NetworkErrorKind, NetworkInterfaceProvider, SetNetworkBindPreferenceRequest, TestHostPorts,
};
""",
    """use super::network::{
    AddressRecord, DesktopHostNetworkControl, InterfaceRecord, NetworkErrorKind,
    NetworkInterfaceProvider, TestHostPorts,
};
use super::network_dto::{NetworkAddressClassDto, SetNetworkBindPreferenceRequest};
""",
)
replace_once(
    "desktop/src-tauri/src/platform/diagnostics_export.rs",
    "const fn failure(message: &'static str) -> DesktopPlatformFailure {",
    "fn failure(message: &'static str) -> DesktopPlatformFailure {",
)

# Normalize the repository patch script to the current master layout.
apply_path = Path(".github/apply-block21.py")
source = apply_path.read_text()

start = source.index("# Register the platform modules and tests.")
end = source.index("# Platform errors must preserve dynamic bind/interface details.", start)
registration = r'''# Register the platform modules and tests.
insert_after(
    "desktop/src-tauri/src/platform/mod.rs",
    "pub mod identity;\n",
    "pub mod network;\npub mod network_dto;\n",
)
insert_after(
    "desktop/src-tauri/src/platform/mod.rs",
    '#[cfg(test)]\nmod file_picker_tests;\n',
    '#[cfg(test)]\nmod network_tests;\n',
)

'''
source = source[:start] + registration + source[end:]

start = source.index("# Register commands and module contract.")
end = source.index("# Include new DTOs in deterministic bindings.", start)
command_registration = r'''# Register commands and module contract.
lib = "desktop/src-tauri/src/lib.rs"
replace_once(
    lib,
    "            host_commands::create_host_session,\n            app_state::attach_notifications,",
    "            host_commands::create_host_session,\n            host_commands::get_host_network_state,\n            host_commands::set_host_network_preference,\n            app_state::attach_notifications,",
)

'''
source = source[:start] + command_registration + source[end:]

# Preserve owned failure messages while making the truncation helper borrowing.
source = source.replace(
    """    ) -> Self {
        Self {
            code,
            message: bounded_message(message.into()),
""",
    """    ) -> Self {
        let message = message.into();
        Self {
            code,
            message: bounded_message(&message),
""",
    1,
)
source = source.replace(
    """
) -> CoreError {
    CoreError::new(
        code,
        bounded_message(message.into()),
""",
    """
) -> CoreError {
    let message = message.into();
    CoreError::new(
        code,
        bounded_message(&message),
""",
    1,
)
source = source.replace(
    "fn bounded_message(message: String) -> String {",
    "fn bounded_message(message: &str) -> String {",
    1,
)

# The production path uses new_with_network; the convenience constructor is test-only.
source = source.replace(
    """    '''    pub(super) fn new(paths: DesktopProfilePaths) -> Self {
        Self::new_with_network(paths, Arc::new(DesktopHostNetworkControl::production()))
    }
""",
    """    '''    #[cfg(test)]
    pub(super) fn new(paths: DesktopProfilePaths) -> Self {
        Self::new_with_network(paths, Arc::new(DesktopHostNetworkControl::production()))
    }
""",
    1,
)

apply_path.write_text(source)
