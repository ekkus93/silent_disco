use crate::dto::DesktopErrorDto;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use silent_disco_core::runtime::CoreActorRuntime;
use silent_disco_core::storage::{DatabaseWorker, StorageError};

pub struct DesktopOwnedResources {
    pub actor: CoreActorRuntime,
    pub database: DatabaseWorker,
    pub lease: ProfileLease,
}

/// Shuts down owned resources in strict reverse startup order.
///
/// Every cleanup phase is attempted. A later cleanup failure never overwrites
/// an earlier failure; the returned bounded error describes every failed phase.
pub fn shutdown_owned_resources(resources: DesktopOwnedResources) -> Result<(), DesktopErrorDto> {
    let actor_error = resources.actor.shutdown().err();
    let database_error = resources.database.stop_and_join().err();
    let lease_error = resources.lease.release().err();

    if actor_error.is_none() && database_error.is_none() && lease_error.is_none() {
        return Ok(());
    }

    Err(cleanup_error(
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    ))
}

pub fn cleanup_without_actor(
    database: DatabaseWorker,
    lease: ProfileLease,
    primary: DesktopErrorDto,
) -> DesktopErrorDto {
    let database_error = database.stop_and_join().err();
    let lease_error = lease.release().err();
    combine_primary(primary, None, database_error.as_ref(), lease_error.as_ref())
}

pub fn cleanup_lease(lease: ProfileLease, primary: DesktopErrorDto) -> DesktopErrorDto {
    let lease_error = lease.release().err();
    combine_primary(primary, None, None, lease_error.as_ref())
}

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
            "desktop shutdown failed (actor={}, database={}, profile_lock={})",
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
