use crate::dto::DesktopErrorDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use silent_disco_core::runtime::CoreActorRuntime;
use silent_disco_core::storage::{DatabaseWorker, StorageError};
use std::sync::Arc;

pub struct DesktopOwnedResources {
    pub notifications: Arc<DesktopNotificationBuffer>,
    pub actor: CoreActorRuntime,
    pub database: DatabaseWorker,
    pub lease: ProfileLease,
}

/// Shuts down owned resources in strict reverse startup order.
///
/// Every cleanup phase is attempted. A later cleanup failure never overwrites
/// an earlier failure; the returned bounded error describes every failed phase.
///
/// # Errors
///
/// Returns one bounded structured error when notification-worker shutdown, actor shutdown,
/// database shutdown, or explicit profile-lock release fails. All later cleanup phases are
/// still attempted.
pub fn shutdown_owned_resources(resources: DesktopOwnedResources) -> Result<(), DesktopErrorDto> {
    let notification_error = resources.notifications.shutdown().err();
    let actor_error = resources.actor.shutdown().err();
    let database_error = resources.database.stop_and_join().err();
    let lease_error = resources.lease.release().err();

    if notification_error.is_none()
        && actor_error.is_none()
        && database_error.is_none()
        && lease_error.is_none()
    {
        return Ok(());
    }

    Err(cleanup_error(
        notification_error.as_ref(),
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    ))
}

/// Cleans up database and profile-lock ownership after actor startup failed.
#[must_use]
pub fn cleanup_without_actor(
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
pub fn cleanup_lease(lease: ProfileLease, primary: DesktopErrorDto) -> DesktopErrorDto {
    let lease_error = lease.release().err();
    combine_primary(primary, None, None, lease_error.as_ref())
}

/// Cleans up actor, database, and profile-lock ownership after startup failed.
#[must_use]
pub fn cleanup_with_actor(
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
            "desktop shutdown failed (notifications={}, actor={}, database={}, profile_lock={})",
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
