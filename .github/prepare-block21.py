from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"candidate repair anchor count {count}: {path}: {old[:80]!r}")
    target.write_text(source.replace(old, new))


replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "        pub(super) kind: NetworkErrorKind,",
    "        kind: NetworkErrorKind,",
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
apply_path.write_text(source)
