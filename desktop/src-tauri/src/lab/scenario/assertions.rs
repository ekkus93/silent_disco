use super::{NodeId, ScenarioAssertion, ScenarioLifecycleTarget};
use crate::lab::recorder::{RecordedNotification, RecordedNotificationKind};
use silent_disco_core::runtime::{CoreSnapshot, PermissionCapability};

pub(super) fn assertion_deadline(assertion: &ScenarioAssertion) -> u64 {
    assertion.by_ms()
}

pub(super) fn assertion_node(assertion: &ScenarioAssertion) -> &NodeId {
    assertion.node()
}

pub(super) fn assertion_kind(assertion: &ScenarioAssertion) -> &'static str {
    assertion.kind_name()
}

pub(crate) fn evaluate_assertion(
    assertion: &ScenarioAssertion,
    snapshot: Option<&CoreSnapshot>,
    entries: &[RecordedNotification],
) -> bool {
    match assertion {
        ScenarioAssertion::LifecycleReached { target, .. } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            match target {
                ScenarioLifecycleTarget::Role(role) => snapshot.selected_role == Some(role.0),
                ScenarioLifecycleTarget::Host(state) => snapshot.host_lifecycle == state.0,
                ScenarioLifecycleTarget::Listener(state) => snapshot.listener_lifecycle == state.0,
                ScenarioLifecycleTarget::Playback(state) => snapshot.playback_state == state.0,
            }
        }
        ScenarioAssertion::CapabilityAvailable {
            capability,
            available,
            ..
        } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            let actual = match capability {
                PermissionCapability::NearbyDiscovery => {
                    snapshot.capabilities.nearby_discovery_available
                }
                PermissionCapability::NearbyAdvertising => {
                    snapshot.capabilities.nearby_advertising_available
                }
                PermissionCapability::LocalNetwork => snapshot.capabilities.local_network_available,
                PermissionCapability::AudioSourceSelection => {
                    snapshot.capabilities.audio_source_selection_available
                }
                PermissionCapability::AudioOutput => snapshot.capabilities.audio_output_available,
                PermissionCapability::SecureStore => snapshot.capabilities.secure_store_available,
            };
            actual == *available
        }
        ScenarioAssertion::ListenerCountAtLeast { count, .. } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            u32::try_from(snapshot.listeners.len()).unwrap_or(u32::MAX) >= *count
        }
        ScenarioAssertion::SyncConfidenceAtLeast { confidence, .. } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            snapshot
                .synchronization
                .is_some_and(|summary| summary.confidence.stable_code() >= confidence.stable_code())
        }
        ScenarioAssertion::SynchronizationWithinBounds {
            max_abs_offset_ms,
            max_round_trip_ms,
            ..
        } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            let Some(summary) = snapshot.synchronization else {
                return false;
            };
            max_abs_offset_ms.is_none_or(|bound| summary.offset_ms.abs() <= bound)
                && max_round_trip_ms.is_none_or(|bound| summary.round_trip_ms <= bound)
        }
        ScenarioAssertion::ErrorCodeObserved { code, .. } => entries.iter().any(|entry| {
            matches!(
                &entry.kind,
                RecordedNotificationKind::Error { code: observed, .. } if observed == code
            )
        }),
        ScenarioAssertion::DeliverySeverityIs { severity, .. } => {
            let Some(snapshot) = snapshot else {
                return false;
            };
            snapshot
                .last_delivery
                .is_some_and(|report| report.severity.stable_code() == severity.stable_code())
        }
        ScenarioAssertion::UnderrunFramesAtMost {
            max_total_missing_frames,
            ..
        } => underrun_frames_within_limit(entries, *max_total_missing_frames),
        ScenarioAssertion::CleanShutdown { .. }
        | ScenarioAssertion::NoUnexpectedFatalError { .. } => {
            !entries.iter().any(|entry| entry.kind.is_fatal_error())
        }
    }
}

fn underrun_frames_within_limit(entries: &[RecordedNotification], maximum_frames: u32) -> bool {
    let mut total = 0_u64;
    for entry in entries {
        let RecordedNotificationKind::Diagnostic { name, fields } = &entry.kind else {
            continue;
        };
        if name != "audio_underrun" {
            continue;
        }
        let Some((_, value)) = fields.iter().find(|(key, _)| key == "missing_frames") else {
            continue;
        };
        let Ok(missing) = value.parse::<u64>() else {
            return false;
        };
        total = total.saturating_add(missing);
    }
    total <= u64::from(maximum_frames)
}
