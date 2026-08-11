//! Interface enumeration and normalization: reads the OS's raw network
//! interface list (via `netdev`) and turns it into this crate's own
//! bounded, `netdev`-independent [`InterfaceRecord`]/[`AddressRecord`]
//! shape that the rest of the `network` module tree works with.

use crate::platform::network_error::DesktopNetworkError;
use netdev::Interface;
use std::net::IpAddr;

const MAX_INTERFACE_RECORDS: usize = 256;
const MAX_ADDRESS_RECORDS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub(in crate::platform) struct InterfaceRecord {
    pub name: String,
    pub index: u32,
    pub up: bool,
    pub running: bool,
    pub oper_up: bool,
    pub loopback: bool,
    pub point_to_point: bool,
    pub tun: bool,
    pub physical: bool,
    pub default_route: bool,
    pub addresses: Vec<AddressRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::platform) struct AddressRecord {
    pub address: IpAddr,
    pub prefix_length: u8,
}

pub(in crate::platform) trait NetworkInterfaceProvider:
    Send + Sync + 'static
{
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, DesktopNetworkError>;
}

#[derive(Debug, Default)]
pub(super) struct NetdevNetworkInterfaceProvider;

impl NetworkInterfaceProvider for NetdevNetworkInterfaceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, DesktopNetworkError> {
        normalize_interfaces(netdev::get_interfaces())
    }
}

pub(super) fn normalize_interfaces(
    interfaces: Vec<Interface>,
) -> Result<Vec<InterfaceRecord>, DesktopNetworkError> {
    if interfaces.len() > MAX_INTERFACE_RECORDS {
        return Err(DesktopNetworkError::resource_limit(
            "desktop network interface count exceeds the supported limit",
        ));
    }
    let mut address_count = 0usize;
    interfaces
        .into_iter()
        .map(|interface| {
            let up = interface.is_up();
            let running = interface.is_running();
            let oper_up = interface.is_oper_up();
            let loopback = interface.is_loopback();
            let point_to_point = interface.is_point_to_point();
            let tun = interface.is_tun();
            let physical = interface.is_physical();
            let default_route = interface.default;
            let index = interface.index;
            let name = interface.name.clone();
            let mut addresses = Vec::with_capacity(interface.ipv4.len() + interface.ipv6.len());
            for network in &interface.ipv4 {
                addresses.push(AddressRecord {
                    address: IpAddr::V4(network.addr()),
                    prefix_length: network.prefix_len(),
                });
            }
            for network in &interface.ipv6 {
                addresses.push(AddressRecord {
                    address: IpAddr::V6(network.addr()),
                    prefix_length: network.prefix_len(),
                });
            }
            address_count = address_count.saturating_add(addresses.len());
            if address_count > MAX_ADDRESS_RECORDS {
                return Err(DesktopNetworkError::resource_limit(
                    "desktop network address count exceeds the supported limit",
                ));
            }
            Ok(InterfaceRecord {
                name,
                index,
                up,
                running,
                oper_up,
                loopback,
                point_to_point,
                tun,
                physical,
                default_route,
                addresses,
            })
        })
        .collect()
}
