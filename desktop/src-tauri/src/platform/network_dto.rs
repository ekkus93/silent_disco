use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NetworkAddressClassDto {
    Loopback,
    LinkLocal,
    PrivateLan,
    Vpn,
    Container,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[allow(clippy::struct_excessive_bools)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NetworkAddressCandidateDto {
    pub interface_name: String,
    pub interface_index: u32,
    pub address: String,
    pub prefix_length: u8,
    pub classification: NetworkAddressClassDto,
    pub is_default_route: bool,
    pub is_active: bool,
    pub is_physical: bool,
    pub selectable: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NetworkBindPreferenceDto {
    pub mode: String,
    pub interface_name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SetNetworkBindPreferenceRequest {
    pub mode: String,
    pub interface_name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NetworkBindingDto {
    pub interface_name: String,
    pub address: String,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NetworkInterfaceSnapshotDto {
    pub preference: NetworkBindPreferenceDto,
    pub candidates: Vec<NetworkAddressCandidateDto>,
    pub automatic_selection: Option<NetworkAddressCandidateDto>,
    pub resolved_selection: Option<NetworkAddressCandidateDto>,
    pub requires_explicit_selection: bool,
    pub selection_error: Option<String>,
    pub active_binding: Option<NetworkBindingDto>,
    pub active_binding_valid: bool,
    pub interface_change: Option<String>,
}
