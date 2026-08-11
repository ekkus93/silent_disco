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

impl super::LiveTransportDriver {
    pub(super) fn profile(&self, node_id: &NodeId) -> ReceiveFaultProfile {
        self.profiles.get(node_id).copied().unwrap_or_default()
    }

    pub(super) fn has_link(&self, from: &NodeId, to: &NodeId) -> bool {
        self.links
            .iter()
            .any(|link| &link.from == from && &link.to == to)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), DesktopErrorDto> {
        let mut failure = None;
        let mut listener_ids: Vec<NodeId> = self.listeners.keys().cloned().collect();
        listener_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for node_id in listener_ids {
            let Some(mut listener) = self.listeners.remove(&node_id) else {
                append_failure(
                    &mut failure,
                    live_error(
                        "listener_missing",
                        &format!("Lab listener '{node_id}' disappeared during shutdown"),
                    ),
                );
                continue;
            };
            if let Err(error) = listener.transport.shutdown() {
                append_failure(
                    &mut failure,
                    transport_error(
                        &format!("shutdown Lab listener '{node_id}'"),
                        &error,
                    ),
                );
            }
        }

        let mut host_ids: Vec<NodeId> = self.hosts.keys().cloned().collect();
        host_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for node_id in host_ids {
            let Some(mut host) = self.hosts.remove(&node_id) else {
                append_failure(
                    &mut failure,
                    live_error(
                        "host_missing",
                        &format!("Lab host '{node_id}' disappeared during shutdown"),
                    ),
                );
                continue;
            };
            if let Err(error) = host.transport.shutdown() {
                append_failure(
                    &mut failure,
                    transport_error(&format!("shutdown Lab host '{node_id}'"), &error),
                );
            }
        }

        failure.map_or(Ok(()), Err)
    }
}

fn append_failure(primary: &mut Option<DesktopErrorDto>, next: DesktopErrorDto) {
    *primary = Some(match primary.take() {
        Some(previous) => previous.with_appended_cleanup(Some(next)),
        None => next,
    });
}
