//! Bind-address selection policy: turning a classified interface snapshot
//! into either a concrete IPv4 address to bind the host transport to, or an
//! explicit, typed rejection reason.

use super::classification::{classify, is_active};
#[cfg(test)]
use super::interfaces::normalize_interfaces;
use super::{AddressRecord, DesktopNetworkError, InterfaceRecord};
use crate::platform::network_dto::{
    NetworkAddressCandidateDto, NetworkAddressClassDto, NetworkBindPreferenceDto,
    SetNetworkBindPreferenceRequest,
};
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BindPreference {
    Automatic,
    Explicit {
        interface_name: String,
        address: Ipv4Addr,
    },
}

impl BindPreference {
    pub(super) fn dto(&self) -> NetworkBindPreferenceDto {
        match self {
            Self::Automatic => NetworkBindPreferenceDto {
                mode: "automatic".to_owned(),
                interface_name: None,
                address: None,
            },
            Self::Explicit {
                interface_name,
                address,
            } => NetworkBindPreferenceDto {
                mode: "explicit".to_owned(),
                interface_name: Some(interface_name.clone()),
                address: Some(address.to_string()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SelectedAddress {
    pub(super) interface_name: String,
    pub(super) interface_index: u32,
    pub(super) address: Ipv4Addr,
    pub(super) default_route: bool,
    pub(super) physical: bool,
    pub(super) prefix_length: u8,
}

impl SelectedAddress {
    pub(super) fn dto(&self) -> NetworkAddressCandidateDto {
        NetworkAddressCandidateDto {
            interface_name: self.interface_name.clone(),
            interface_index: self.interface_index,
            address: self.address.to_string(),
            prefix_length: self.prefix_length,
            classification: NetworkAddressClassDto::PrivateLan,
            is_default_route: self.default_route,
            is_active: true,
            is_physical: self.physical,
            selectable: true,
            rejection_reason: None,
        }
    }
}

pub(super) fn address_candidates(
    interfaces: &[InterfaceRecord],
) -> Vec<NetworkAddressCandidateDto> {
    interfaces
        .iter()
        .flat_map(|interface| {
            interface
                .addresses
                .iter()
                .map(move |address: &AddressRecord| {
                    let class = classify(interface, address.address);
                    let active = is_active(interface);
                    let selectable = active
                        && class == NetworkAddressClassDto::PrivateLan
                        && address.address.is_ipv4();
                    let rejection_reason = if selectable {
                        None
                    } else if !active {
                        Some("interface is not active".to_owned())
                    } else if address.address.is_ipv6() {
                        Some(
                            "IPv6 host binding is not enabled in the initial desktop LAN baseline"
                                .to_owned(),
                        )
                    } else {
                        Some(
                            match class {
                                NetworkAddressClassDto::Loopback => {
                                    "loopback addresses are not advertised"
                                }
                                NetworkAddressClassDto::LinkLocal => {
                                    "link-local addresses are not advertised"
                                }
                                NetworkAddressClassDto::Vpn => {
                                    "VPN interfaces require a later explicit policy"
                                }
                                NetworkAddressClassDto::Container => {
                                    "container interfaces are not advertised"
                                }
                                NetworkAddressClassDto::Other => {
                                    "address is not a private LAN address"
                                }
                                NetworkAddressClassDto::PrivateLan => "address is not selectable",
                            }
                            .to_owned(),
                        )
                    };
                    NetworkAddressCandidateDto {
                        interface_name: interface.name.clone(),
                        interface_index: interface.index,
                        address: address.address.to_string(),
                        prefix_length: address.prefix_length,
                        classification: class,
                        is_default_route: interface.default_route,
                        is_active: active,
                        is_physical: interface.physical,
                        selectable,
                        rejection_reason,
                    }
                })
        })
        .collect()
}

pub(super) fn select_address(
    interfaces: &[InterfaceRecord],
    preference: &BindPreference,
) -> Result<SelectedAddress, DesktopNetworkError> {
    let mut selectable = interfaces
        .iter()
        .flat_map(|interface| {
            interface.addresses.iter().filter_map(move |address| {
                let IpAddr::V4(ipv4) = address.address else {
                    return None;
                };
                (is_active(interface)
                    && classify(interface, address.address) == NetworkAddressClassDto::PrivateLan)
                    .then(|| SelectedAddress {
                        interface_name: interface.name.clone(),
                        interface_index: interface.index,
                        address: ipv4,
                        default_route: interface.default_route,
                        physical: interface.physical,
                        prefix_length: address.prefix_length,
                    })
            })
        })
        .collect::<Vec<_>>();
    selectable.sort_by(|left, right| {
        right
            .default_route
            .cmp(&left.default_route)
            .then(left.interface_index.cmp(&right.interface_index))
            .then(left.interface_name.cmp(&right.interface_name))
            .then(left.address.octets().cmp(&right.address.octets()))
    });
    match preference {
        BindPreference::Explicit {
            interface_name,
            address,
        } => selectable
            .into_iter()
            .find(|candidate| {
                &candidate.interface_name == interface_name && &candidate.address == address
            })
            .ok_or_else(|| {
                DesktopNetworkError::unavailable(
                    "the requested private-LAN interface address is unavailable",
                )
            }),
        BindPreference::Automatic => match selectable.as_slice() {
            [] => Err(DesktopNetworkError::unavailable(
                "no active private-LAN IPv4 address is available for the desktop host",
            )),
            [single] => Ok(single.clone()),
            many => {
                let defaults = many
                    .iter()
                    .filter(|candidate| candidate.default_route)
                    .collect::<Vec<_>>();
                match defaults.as_slice() {
                    [single] => Ok((*single).clone()),
                    _ => Err(DesktopNetworkError::ambiguous(
                        "multiple private-LAN addresses are eligible; select one explicitly",
                    )),
                }
            }
        },
    }
}

pub(super) fn validate_selected(
    interfaces: &[InterfaceRecord],
    selected: &SelectedAddress,
) -> Result<(), DesktopNetworkError> {
    let preference = BindPreference::Explicit {
        interface_name: selected.interface_name.clone(),
        address: selected.address,
    };
    select_address(interfaces, &preference).map(|_| ())
}

pub(super) fn parse_preference(
    request: &SetNetworkBindPreferenceRequest,
) -> Result<BindPreference, DesktopNetworkError> {
    match request.mode.as_str() {
        "automatic" if request.interface_name.is_none() && request.address.is_none() => {
            Ok(BindPreference::Automatic)
        }
        "explicit" => {
            let interface_name = request.interface_name.as_deref().ok_or_else(|| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference requires an interface name",
                )
            })?;
            if interface_name.is_empty()
                || interface_name.len() > 128
                || interface_name.trim() != interface_name
            {
                return Err(DesktopNetworkError::invalid_argument(
                    "network interface name is invalid",
                ));
            }
            let address = request.address.as_deref().ok_or_else(|| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference requires an IPv4 address",
                )
            })?;
            let address = address.parse::<Ipv4Addr>().map_err(|_| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference address must be canonical IPv4",
                )
            })?;
            Ok(BindPreference::Explicit {
                interface_name: interface_name.to_owned(),
                address,
            })
        }
        _ => Err(DesktopNetworkError::invalid_argument(
            "network preference must be automatic or a complete explicit selection",
        )),
    }
}

/// The interface production would actually bind to, for tests that need a
/// real LAN address.
///
/// Tests used to hand-roll this filter -- "up, not loopback, not tun, not
/// point-to-point, has a private IPv4" -- which accepts strictly more than
/// production does: `classify` additionally excludes VPN and *container*
/// interfaces. On a host with Docker bridges (172.x, private, up, physical
/// by every one of those predicates) the two disagreed, and `netdev`'s
/// ordering decided which interface a test picked. That is what made
/// `port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport`
/// fail roughly three runs in four with "no active private-LAN IPv4 address
/// is available": the test had handed production a bridge, and production
/// correctly refused it.
///
/// Going through `normalize_interfaces` and `select_address` means the answer
/// is production's own, so the two cannot drift apart again.
#[cfg(test)]
pub(in crate::platform) fn first_bindable_private_lan_address() -> Option<(String, u32, Ipv4Addr)> {
    let records = normalize_interfaces(netdev::get_interfaces()).ok()?;
    let selected = select_address(&records, &BindPreference::Automatic).ok()?;
    Some((
        selected.interface_name,
        selected.interface_index,
        selected.address,
    ))
}
