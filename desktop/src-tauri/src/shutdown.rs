use crate::dto::DesktopErrorDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::effect_runner::DesktopPlatformEffectRunner;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use crate::platform::storage_effect_runner::DesktopStorageEffectRunner;
use silent_disco_core::runtime::CoreActorRuntime;
use silent_disco_core::storage::{DatabaseWorker, StorageError};
use std::sync::Arc;

pub(crate) struct DesktopOwnedResources {
    pub(crate) platform_runner: DesktopPlatformEffectRunner,
    pub(crate) storage_runner: DesktopStorageEffectRunner,
    pub(crate) notifications: Arc<DesktopNotificationBuffer>,
    pub(crate) actor: CoreActorRuntime,
    pub(crate) database: DatabaseWorker,
    pub(crate) lease: ProfileLease,
}

/// Shuts down owned resources in dependency-safe order.
///
/// Platform-effect admission stops first while the actor is still available to receive
/// cancellation failures for queued operations. The actor then performs controlled shutdown.
/// The database worker checkpoints and closes next, then the notification dispatcher stops --
/// deliberately last among the workers (spec order 10 before 11: a dispatcher stopped before
/// the database closes could never relay a final database-close-related notification) -- and
/// finally the profile lease is released. Every cleanup phase is attempted; a later failure
/// never overwrites an earlier one.
///
/// # Errors
///
/// Returns one bounded structured error when platform-runner, actor, notification-worker,
/// database, or explicit profile-lock cleanup fails.
pub(crate) fn shutdown_owned_resources(
    resources: DesktopOwnedResources,
) -> Result<(), DesktopErrorDto> {
    let platform_error = resources.platform_runner.shutdown().err();
    let storage_error = resources.storage_runner.shutdown().err();
    let actor_error = resources.actor.shutdown().err();
    let database_error = resources.database.stop_and_join().err();
    let notification_error = resources.notifications.shutdown().err();
    let lease_error = resources.lease.release().err();

    if platform_error.is_none()
        && storage_error.is_none()
        && notification_error.is_none()
        && actor_error.is_none()
        && database_error.is_none()
        && lease_error.is_none()
    {
        return Ok(());
    }

    Err(cleanup_error(
        platform_error.as_ref(),
        storage_error.as_ref(),
        notification_error.as_ref(),
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    ))
}

/// Cleans up database and profile-lock ownership after actor startup failed.
#[must_use]
pub(crate) fn cleanup_without_actor(
    database: DatabaseWorker,
    lease: ProfileLease,
    primary: DesktopErrorDto,
) -> DesktopErrorDto {
    let database_error = database.stop_and_join().err();
    let lease_error = lease.release().err();
    combine_primary(primary, None, database_error.as_ref(), lease_error.as_ref())
}

/// Releases a profile lease after an earlier startup stage failed.
#[must_use]
pub(crate) fn cleanup_lease(lease: ProfileLease, primary: DesktopErrorDto) -> DesktopErrorDto {
    let lease_error = lease.release().err();
    combine_primary(primary, None, None, lease_error.as_ref())
}

/// Cleans up actor, database, and profile-lock ownership after startup failed.
#[must_use]
pub(crate) fn cleanup_with_actor(
    actor: CoreActorRuntime,
    database: DatabaseWorker,
    lease: ProfileLease,
    primary: DesktopErrorDto,
) -> DesktopErrorDto {
    let actor_error = actor.shutdown().err();
    let database_error = database.stop_and_join().err();
    let lease_error = lease.release().err();
    combine_primary(
        primary,
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    )
}

fn cleanup_error(
    platform: Option<&silent_disco_core::error::CoreError>,
    storage: Option<&silent_disco_core::error::CoreError>,
    notifications: Option<&silent_disco_core::error::CoreError>,
    actor: Option<&silent_disco_core::error::CoreError>,
    database: Option<&StorageError>,
    lease: Option<&ProfileLockError>,
) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.shutdown.failed",
        "runtime",
        "fatal",
        false,
        &format!(
            "desktop shutdown failed (platform_runner={}, storage_runner={}, notifications={}, actor={}, database={}, profile_lock={})",
            status(platform),
            status(storage),
            status(notifications),
            status(actor),
            status(database),
            status(lease)
        ),
    )
}

fn combine_primary(
    primary: DesktopErrorDto,
    actor: Option<&silent_disco_core::error::CoreError>,
    database: Option<&StorageError>,
    lease: Option<&ProfileLockError>,
) -> DesktopErrorDto {
    if actor.is_none() && database.is_none() && lease.is_none() {
        return primary;
    }

    DesktopErrorDto::new(
        &primary.code,
        &primary.subsystem,
        &primary.severity,
        primary.retryable,
        &format!(
            "{}; startup cleanup failed (actor={}, database={}, profile_lock={})",
            primary.message,
            status(actor),
            status(database),
            status(lease)
        ),
    )
}

fn status<T: std::fmt::Display>(error: Option<&T>) -> String {
    error.map_or_else(|| "ok".to_owned(), ToString::to_string)
}
