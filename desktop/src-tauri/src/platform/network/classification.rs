//! Address classification predicates.
//!
//! [`classify`]'s precedence order -- loopback, then link-local, then VPN,
//! then container, then private-LAN, then "other" -- is load-bearing: a
//! Docker bridge address (private, up, physical by every naive predicate)
//! must classify as `Container`, not `PrivateLan`, or
//! [`super::bind_selection::select_address`] can pick a bridge interface
//! that production never would. See `bind_selection`'s
//! `first_bindable_private_lan_address` doc comment for the historical
//! test flake this fixed.

use super::InterfaceRecord;
use crate::platform::network_dto::NetworkAddressClassDto;
use std::net::{IpAddr, Ipv6Addr};

pub(super) fn is_active(interface: &InterfaceRecord) -> bool {
    interface.up && (interface.running || interface.oper_up)
}

pub(super) fn classify(interface: &InterfaceRecord, address: IpAddr) -> NetworkAddressClassDto {
    if interface.loopback || address.is_loopback() {
        return NetworkAddressClassDto::Loopback;
    }
    if is_link_local(address) {
        return NetworkAddressClassDto::LinkLocal;
    }
    if is_vpn(interface) {
        return NetworkAddressClassDto::Vpn;
    }
    if is_container(interface) {
        return NetworkAddressClassDto::Container;
    }
    if is_private_lan(address) {
        return NetworkAddressClassDto::PrivateLan;
    }
    NetworkAddressClassDto::Other
}

fn is_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_link_local(),
        IpAddr::V6(address) => address.is_unicast_link_local(),
    }
}

fn is_private_lan(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_broadcast()
        }
        IpAddr::V6(address) => is_unique_local(address),
    }
}

fn is_unique_local(address: Ipv6Addr) -> bool {
    address.octets()[0] & 0xfe == 0xfc
}

fn is_vpn(interface: &InterfaceRecord) -> bool {
    if interface.tun || interface.point_to_point {
        return true;
    }
    let name = interface.name.to_ascii_lowercase();
    [
        "tun",
        "tap",
        "wg",
        "tailscale",
        "utun",
        "ppp",
        "ipsec",
        "zerotier",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn is_container(interface: &InterfaceRecord) -> bool {
    let name = interface.name.to_ascii_lowercase();
    [
        "docker", "br-", "veth", "podman", "cni", "virbr", "lxc", "lxd", "flannel",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}
