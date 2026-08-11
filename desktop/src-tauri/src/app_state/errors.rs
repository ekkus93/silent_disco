//! Error-mapping helpers shared across the `app_state` module tree.

use crate::dto::DesktopErrorDto;
use crate::platform::invitation::InvitationError;

pub(super) fn poisoned_state_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.bridge.state_poisoned",
        "runtime",
        "fatal",
        false,
        "desktop application state mutex was poisoned",
    )
}

/// Maps a QR invitation build failure onto a diagnostic-visible DTO,
/// distinguishing a platform CSPRNG failure (`InvitationError::Nonce`) from
/// a shared-validator rejection (`InvitationError::Invitation`) rather than
/// collapsing both into one generic "validation" category -- a CSPRNG
/// outage is a platform failure the user did not cause and may be able to
/// retry, not a data-shape mistake.
pub(super) fn invitation_error_dto(error: &InvitationError) -> DesktopErrorDto {
    match error {
        InvitationError::Nonce(_) => DesktopErrorDto::new(
            "desktop.invitation.nonce_unavailable",
            "platform",
            "error",
            true,
            &error.to_string(),
        ),
        InvitationError::Invitation(_) => DesktopErrorDto::new(
            "desktop.invitation.build_failed",
            "validation",
            "error",
            false,
            &error.to_string(),
        ),
    }
}
