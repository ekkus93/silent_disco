use crate::notification_buffer::{DesktopNotificationSendError, DesktopNotificationSink};
use crate::runtime_dto::CoreNotificationDto;
use silent_disco_core::runtime::CoreNotification;
use tauri::ipc::Channel;

/// Tauri channel adapter that exposes only the redacted desktop notification DTO.
pub struct TauriNotificationSink {
    channel: Channel<CoreNotificationDto>,
}

impl TauriNotificationSink {
    #[must_use]
    pub const fn new(channel: Channel<CoreNotificationDto>) -> Self {
        Self { channel }
    }
}

impl DesktopNotificationSink for TauriNotificationSink {
    fn send(&self, notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        self.channel
            .send(CoreNotificationDto::from(notification))
            .map_err(|_| channel_closed_error())
    }
}

fn channel_closed_error() -> DesktopNotificationSendError {
    match DesktopNotificationSendError::new("desktop frontend notification channel is closed") {
        Ok(error) => error,
        Err(error) => panic!("invalid static desktop notification send error: {error}"),
    }
}
