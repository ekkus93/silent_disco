use super::{
    ActorState, ApplyOutcome, CoreError, CoreNotification, StorageCompletion, StorageEvent,
    invalid_argument, invalid_state,
};

impl ActorState {
    pub(super) fn apply_storage(&mut self, event: StorageEvent) -> Result<ApplyOutcome, CoreError> {
        event.validate().map_err(|error| {
            invalid_argument(error.to_string(), Some(event.operation_id().clone()))
        })?;
        match event {
            StorageEvent::OperationSucceeded { completion, .. } => match completion {
                StorageCompletion::SettingsLoaded(Some(settings)) => {
                    self.snapshot.tuning = settings.tuning;
                    Ok(ApplyOutcome::changed())
                }
                StorageCompletion::SettingsLoaded(None) => Ok(ApplyOutcome::default()),
                StorageCompletion::SettingsSaved => self.diagnostic("settings_saved", Vec::new()),
                StorageCompletion::TrustedDevicesLoaded(devices) => self.diagnostic(
                    "trusted_devices_loaded",
                    vec![Self::field("count", &devices.len().to_string())?],
                ),
                StorageCompletion::TrustedDeviceUpdated { device_id } => self.diagnostic(
                    "trusted_device_updated",
                    vec![Self::field("device_id", device_id.as_str())?],
                ),
                StorageCompletion::DiagnosticsExportReady { export_id } => self.diagnostic(
                    "diagnostics_export_ready",
                    vec![Self::field("export_id", &export_id)?],
                ),
            },
            StorageEvent::OperationFailed {
                operation_id,
                mut error,
            } => {
                if let Some(inner_id) = &error.operation_id
                    && inner_id != &operation_id
                {
                    return Err(invalid_state(
                        "storage failure operation ID does not match its wrapper",
                        Some(operation_id),
                    ));
                }
                error.operation_id = Some(operation_id);
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![CoreNotification::Error(error)],
                    changed: true,
                    stop_requested: false,
                })
            }
        }
    }
}
