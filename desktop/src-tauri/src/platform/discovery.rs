use super::failure::DesktopPlatformFailure;
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::PlatformEffectRequest;

/// Returns the explicit failure for discovery, advertising, and network effects whose
/// production adapters are intentionally deferred to later desktop blocks.
#[must_use]
pub(super) fn unsupported_effect(request: &PlatformEffectRequest) -> DesktopPlatformFailure {
    match request {
        PlatformEffectRequest::StartAdvertising(_) | PlatformEffectRequest::StopAdvertising => {
            DesktopPlatformFailure::new(
                CoreErrorCode::CapabilityUnavailable,
                "desktop session advertising is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::StartDiscovery(_) | PlatformEffectRequest::StopDiscovery => {
            DesktopPlatformFailure::new(
                CoreErrorCode::CapabilityUnavailable,
                "desktop session discovery is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::EstablishNetwork(_) | PlatformEffectRequest::ReleaseNetwork => {
            DesktopPlatformFailure::new(
                CoreErrorCode::TransportUnavailable,
                "desktop standard-IP transport is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::RequestCapabilities(_)
        | PlatformEffectRequest::PrepareAudioSource(_)
        | PlatformEffectRequest::StartAudioOutput(_)
        | PlatformEffectRequest::StopAudioOutput
        | PlatformEffectRequest::ShareDiagnostics { .. } => DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "effect was routed to the wrong desktop platform adapter",
            ErrorSeverity::Fatal,
            false,
        ),
    }
}
