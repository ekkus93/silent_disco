use super::failure::DesktopPlatformFailure;
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::PlatformEffectRequest;

/// Returns a visible failure for audio effects that require the secure source registry or
/// native output implementation delivered by later desktop blocks.
#[must_use]
pub(super) fn unsupported_effect(request: &PlatformEffectRequest) -> DesktopPlatformFailure {
    match request {
        PlatformEffectRequest::PrepareAudioSource(_) => DesktopPlatformFailure::new(
            CoreErrorCode::CapabilityUnavailable,
            "desktop audio source preparation requires the secure source-selection block",
            ErrorSeverity::Error,
            false,
        ),
        PlatformEffectRequest::StartAudioOutput(_) | PlatformEffectRequest::StopAudioOutput => {
            DesktopPlatformFailure::new(
                CoreErrorCode::AudioEngineUnavailable,
                "desktop native audio output is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::RequestCapabilities(_)
        | PlatformEffectRequest::StartAdvertising(_)
        | PlatformEffectRequest::StopAdvertising
        | PlatformEffectRequest::StartDiscovery(_)
        | PlatformEffectRequest::StopDiscovery
        | PlatformEffectRequest::EstablishNetwork(_)
        | PlatformEffectRequest::ReleaseNetwork
        | PlatformEffectRequest::ShareDiagnostics { .. } => DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "effect was routed to the wrong desktop audio adapter",
            ErrorSeverity::Fatal,
            false,
        ),
    }
}
