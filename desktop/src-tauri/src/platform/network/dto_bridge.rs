//! Translates internal network/mDNS/monitor state into the Tauri-facing
//! DTOs in [`crate::platform::network_dto`].

use super::bind_selection::{
    BindPreference, SelectedAddress, address_candidates, select_address, validate_selected,
};
use super::host_control::ActiveBinding;
use super::{InterfaceRecord, NetworkErrorKind};
use crate::platform::mdns::MdnsPublicationState;
use crate::platform::monitor::MonitorStatus;
use crate::platform::network_dto::{
    MdnsStatusDto, MonitorStatusDto, NetworkBindingDto, NetworkInterfaceSnapshotDto,
};

/// Returns a bounded, classified interface snapshot and detects changes to
/// an active bind. Called by [`super::host_control::DesktopHostNetworkControl::snapshot`]
/// and `::set_preference`.
pub(super) fn snapshot_from(
    interfaces: &[InterfaceRecord],
    preference: &BindPreference,
    active: Option<&ActiveBinding>,
) -> NetworkInterfaceSnapshotDto {
    let mut candidates = address_candidates(interfaces);
    candidates.sort_by(|left, right| {
        left.interface_name
            .cmp(&right.interface_name)
            .then(left.interface_index.cmp(&right.interface_index))
            .then(left.address.cmp(&right.address))
    });
    let automatic = select_address(interfaces, &BindPreference::Automatic);
    let (automatic_selection, requires_explicit_selection) = match &automatic {
        Ok(selected) => (Some(selected.dto()), false),
        Err(error) if error.kind == NetworkErrorKind::Ambiguous => (None, true),
        Err(_) => (None, false),
    };
    let resolved = select_address(interfaces, preference);
    let resolved_selection = resolved.as_ref().ok().map(SelectedAddress::dto);
    let selection_error = resolved.err().map(|error| error.message);
    let active_binding = active.map(|binding| NetworkBindingDto {
        interface_name: binding.selected.interface_name.clone(),
        address: binding.runtime.endpoint().address.to_string(),
        control_port: binding.runtime.endpoint().control_port,
        sync_port: binding.runtime.endpoint().sync_port,
        audio_port: binding.runtime.endpoint().audio_port,
        mdns: mdns_status_dto(&binding.mdns),
    });
    let active_binding_valid =
        active.is_none_or(|binding| validate_selected(interfaces, &binding.selected).is_ok());
    let interface_change = if active.is_some() && !active_binding_valid {
        Some("the active network interface or address is no longer available".to_owned())
    } else {
        None
    };
    NetworkInterfaceSnapshotDto {
        preference: preference.dto(),
        candidates,
        automatic_selection,
        resolved_selection,
        requires_explicit_selection,
        selection_error,
        active_binding,
        active_binding_valid,
        interface_change,
    }
}

fn mdns_status_dto(mdns: &MdnsPublicationState) -> MdnsStatusDto {
    match mdns {
        MdnsPublicationState::Active(_) => MdnsStatusDto {
            active: true,
            failure_reason: None,
        },
        MdnsPublicationState::Failed(error) => MdnsStatusDto {
            active: false,
            failure_reason: Some(error.to_string()),
        },
    }
}

/// Called by [`super::host_control::DesktopHostNetworkControl::monitor_status`].
pub(super) fn monitor_status_dto(status: &MonitorStatus) -> MonitorStatusDto {
    MonitorStatusDto {
        enabled: status.enabled,
        active: status.active,
        failure_reason: status.failure_reason.clone(),
    }
}
