//! Address classification predicates.
//!
//! [`classify`]'s precedence order -- loopback, then link-local, then VPN,
//! then container, then private-LAN, then shared address space, then "other"
//! -- is load-bearing: a
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
    if is_shared_address_space(address) {
        return NetworkAddressClassDto::SharedAddressSpace;
    }
    NetworkAddressClassDto::Other
}

/// RFC 6598 shared address space, `100.64.0.0/10`.
///
/// Ordered after `is_private_lan` so it can never shadow a genuine RFC 1918
/// address, and after the VPN/container checks so a tunnel or bridge that
/// happens to carry a carrier-NAT address still classifies by interface.
fn is_shared_address_space(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, ..] = address.octets();
            first == 100 && (64..128).contains(&second)
        }
        IpAddr::V6(_) => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::network::AddressRecord;
    use std::net::Ipv4Addr;

    fn interface(name: &str, address: Ipv4Addr) -> InterfaceRecord {
        InterfaceRecord {
            name: name.to_owned(),
            index: 2,
            up: true,
            running: true,
            oper_up: true,
            loopback: false,
            point_to_point: false,
            tun: false,
            physical: true,
            default_route: true,
            addresses: vec![AddressRecord {
                address: IpAddr::V4(address),
                prefix_length: 24,
            }],
        }
    }

    fn class_of(name: &str, address: Ipv4Addr) -> NetworkAddressClassDto {
        let record = interface(name, address);
        classify(&record, IpAddr::V4(address))
    }

    #[test]
    fn rfc1918_ranges_classify_as_private_lan() {
        for address in [
            Ipv4Addr::new(192, 168, 1, 20),
            Ipv4Addr::new(10, 0, 0, 5),
            Ipv4Addr::new(172, 16, 0, 9),
            Ipv4Addr::new(172, 31, 255, 254),
        ] {
            assert_eq!(
                class_of("wlo1", address),
                NetworkAddressClassDto::PrivateLan,
                "{address} must classify as a private LAN address"
            );
        }
    }

    // RFC 6598 is 100.64.0.0/10, i.e. 100.64.0.0 through 100.127.255.255.
    // The boundaries matter: 100.63 and 100.128 are ordinary public space and
    // must not be mistaken for carrier-grade NAT.
    #[test]
    fn shared_address_space_covers_exactly_rfc6598() {
        for address in [
            Ipv4Addr::new(100, 64, 0, 0),
            Ipv4Addr::new(100, 64, 64, 132),
            Ipv4Addr::new(100, 100, 1, 1),
            Ipv4Addr::new(100, 127, 255, 255),
        ] {
            assert_eq!(
                class_of("wlo1", address),
                NetworkAddressClassDto::SharedAddressSpace,
                "{address} is inside 100.64.0.0/10"
            );
        }

        for address in [
            Ipv4Addr::new(100, 63, 255, 255),
            Ipv4Addr::new(100, 128, 0, 0),
            Ipv4Addr::new(99, 64, 0, 1),
            Ipv4Addr::new(101, 64, 0, 1),
        ] {
            assert_eq!(
                class_of("wlo1", address),
                NetworkAddressClassDto::Other,
                "{address} is outside 100.64.0.0/10 and is ordinary public space"
            );
        }
    }

    // Precedence: an interface-based verdict must win over an address-based
    // one, or a Docker bridge or VPN carrying a carrier-NAT address would be
    // offered as though it were an ordinary shared-address-space candidate.
    #[test]
    fn interface_classification_outranks_shared_address_space() {
        assert_eq!(
            class_of("docker0", Ipv4Addr::new(100, 64, 0, 1)),
            NetworkAddressClassDto::Container
        );
        assert_eq!(
            class_of("br-abc123", Ipv4Addr::new(100, 64, 0, 1)),
            NetworkAddressClassDto::Container
        );
        assert_eq!(
            class_of("tun0", Ipv4Addr::new(100, 64, 0, 1)),
            NetworkAddressClassDto::Vpn
        );
        assert_eq!(
            class_of("tailscale0", Ipv4Addr::new(100, 64, 0, 1)),
            NetworkAddressClassDto::Vpn
        );
    }

    #[test]
    fn loopback_and_link_local_still_outrank_everything() {
        assert_eq!(
            class_of("lo", Ipv4Addr::new(127, 0, 0, 1)),
            NetworkAddressClassDto::Loopback
        );
        assert_eq!(
            class_of("wlo1", Ipv4Addr::new(169, 254, 1, 1)),
            NetworkAddressClassDto::LinkLocal
        );
    }
}
