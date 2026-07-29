use super::{
    ActorState, ApplyOutcome, CoreError, CoreNotification, PendingStorageOperation,
    StorageCompletion, StorageEvent, invalid_argument, invalid_state,
};

impl ActorState {
    pub(super) fn apply_tuning_storage(
        &mut self,
        event: StorageEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        event.validate().map_err(|error| {
            invalid_argument(error.to_string(), Some(event.operation_id().clone()))
        })?;
        let wrapper_id = event.operation_id().clone();
        let pending = self
            .remove_pending_storage(&wrapper_id)
            .ok_or_else(|| {
                invalid_state(
                    "stale or duplicate tuning storage completion",
                    Some(wrapper_id.clone()),
                )
            })?;
        match (pending, event) {
            (
                PendingStorageOperation::PersistTuning(settings),
                StorageEvent::OperationSucceeded {
                    completion: StorageCompletion::SettingsSaved,
                    ..
                },
            ) => {
                self.snapshot.tuning = settings;
                let mut outcome = self.diagnostic("settings_saved", Vec::new())?;
                outcome.changed = true;
                Ok(outcome)
            }
            (
                PendingStorageOperation::PersistTuning(_),
                StorageEvent::OperationFailed {
                    operation_id,
                    mut error,
                },
            ) => {
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
            _ => Err(invalid_state(
                "tuning storage completion kind does not match the pending operation",
                Some(wrapper_id),
            )),
        }
    }
}
