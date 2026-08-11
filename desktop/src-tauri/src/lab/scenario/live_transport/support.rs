use super::super::{NodeId, Scenario};
use crate::dto::DesktopErrorDto;
use silent_disco_core::domain::DeliverySeverity;
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::DeliveryReport;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct ReceiveFaultProfile {
    pub(super) latency_ms: u64,
    pub(super) jitter_ms: u64,
    pub(super) loss_permille: u16,
    pub(super) seed: u64,
}

pub(super) fn build_receive_profiles(
    scenario: &Scenario,
) -> Result<HashMap<NodeId, ReceiveFaultProfile>, DesktopErrorDto> {
    let mut profiles: HashMap<NodeId, ReceiveFaultProfile> = HashMap::new();
    for link in &scenario.links {
        let candidate = ReceiveFaultProfile {
            latency_ms: link.latency_ms,
            jitter_ms: link.jitter_ms,
            loss_permille: link.loss_permille,
            seed: node_seed(scenario.seed, &link.to),
        };
        if let Some(existing) = profiles.get(&link.to) {
            if existing.latency_ms != candidate.latency_ms
                || existing.jitter_ms != candidate.jitter_ms
                || existing.loss_permille != candidate.loss_permille
            {
                return Err(live_error(
                    "ambiguous_link_faults",
                    "multiple links targeting one Lab node must use the same receive-side latency/jitter/loss profile",
                ));
            }
        } else {
            profiles.insert(link.to.clone(), candidate);
        }
    }
    Ok(profiles)
}

fn node_seed(base: u64, node: &NodeId) -> u64 {
    let mut value = base ^ 0x9E37_79B9_7F4A_7C15;
    for byte in node.as_str().bytes() {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x100_0000_01B3);
    }
    value
}

pub(super) const fn failed_delivery_report() -> DeliveryReport {
    DeliveryReport {
        intended_peers: 1,
        successful_peers: 0,
        failed_peers: 1,
        severity: DeliverySeverity::PartialFailure,
    }
}

pub(super) fn core_error(error_value: CoreError) -> DesktopErrorDto {
    let code = error_value.code.stable_name();
    let message = error_value.message;
    live_error("core_rejected_fact", &format!("{code}: {message}"))
}

pub(super) fn transport_error(
    context: &str,
    error_value: &silent_disco_core::transport::TransportError,
) -> DesktopErrorDto {
    live_error("transport_failed", &format!("{context}: {error_value}"))
}

pub(super) fn live_error(suffix: &str, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(
        &format!("desktop.lab.live_transport_{suffix}"),
        "transport",
        "error",
        false,
        message,
    )
}
