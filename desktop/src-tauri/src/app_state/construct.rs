//! The profile-open construction pipeline: acquires the profile lock, loads
//! identities, opens storage, starts the core actor, and wires the platform
//! and storage effect runners -- in that order, with each failure branch
//! tearing down exactly what it had already acquired (via `cleanup_lease`/
//! `cleanup_without_actor`/`cleanup_with_actor`), in reverse order.

use super::state::ReadyRuntime;
use crate::dto::DesktopErrorDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::effect_runner::{
    DesktopCoreObserver, DesktopPlatformEffectDispatcher, DesktopPlatformEffectRunner,
};
use crate::platform::identity::DesktopIdentityProvider;
use crate::platform::invitation_identity::DesktopHostSigningIdentityProvider;
use crate::platform::network::DesktopHostNetworkControl;
use crate::platform::paths::DesktopProfilePaths;
use crate::platform::profile_lock::ProfileLease;
use crate::platform::source_staging::cleanup_incomplete_sources;
use crate::platform::storage_effect_runner::{
    DesktopStorageEffectDispatcher, DesktopStorageEffectRunner,
};
use crate::profile::ProfileId;
use crate::runtime_dto::CoreSnapshotDto;
use crate::shutdown::{
    DesktopOwnedResources, cleanup_lease, cleanup_with_actor, cleanup_without_actor,
};
use silent_disco_core::runtime::{CoreActorConfig, CoreActorRuntime};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::sync::Arc;
use std::time::Duration;

const INITIAL_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

/// Opens one production profile after lock, identity, storage, actor, and bridge startup.
///
/// # Errors
///
/// Returns a structured error for profile, path, lock, identity, storage, actor, bridge, or lifecycle failure; partial startup is cleaned up before returning.
pub(super) fn open_runtime(
    paths: &DesktopProfilePaths,
    profile_id: ProfileId,
    provider: &dyn DesktopIdentityProvider,
    signing_provider: &dyn DesktopHostSigningIdentityProvider,
    notifications: Arc<DesktopNotificationBuffer>,
) -> Result<(ReadyRuntime, CoreSnapshotDto), DesktopErrorDto> {
    let lease = ProfileLease::acquire(paths, &profile_id).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.profile.lock_failed",
            "storage",
            "fatal",
            false,
            &error.to_string(),
        )
    })?;

    if let Err(primary) = cleanup_incomplete_sources(paths.sources()) {
        return Err(cleanup_lease(lease, primary));
    }

    let identity = match provider.load_or_create(&profile_id) {
        Ok(identity) => identity,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.identity.unavailable",
                "platform",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };

    let signing_identity = match signing_provider.load_or_create(&profile_id) {
        Ok(signing_identity) => signing_identity,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.invitation_identity.unavailable",
                "platform",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };

    let database_config = match DatabaseConfig::new(paths.domain_database()) {
        Ok(config) => config,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.storage.configure_failed",
                "storage",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };
    let database = match DatabaseWorker::start(database_config) {
        Ok(database) => database,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.storage.open_failed",
                "storage",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };

    let network = Arc::new(DesktopHostNetworkControl::production());
    let (platform_dispatcher, platform_inbox) = DesktopPlatformEffectDispatcher::channel();
    let (storage_dispatcher, storage_inbox) = DesktopStorageEffectDispatcher::channel();
    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        Arc::clone(&network),
        storage_dispatcher.clone(),
    );
    let actor =
        match CoreActorRuntime::start(CoreActorConfig::new(identity.device_id().clone()), observer)
        {
            Ok(actor) => actor,
            Err(error) => {
                let primary = DesktopErrorDto::from(error);
                return Err(cleanup_without_actor(database, lease, primary));
            }
        };
    let handle = actor.handle();

    let delivered_snapshot = match notifications.wait_for_initial_snapshot(INITIAL_SNAPSHOT_TIMEOUT)
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };
    let current_snapshot = match handle.current_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };
    if current_snapshot.revision != delivered_snapshot.revision {
        let primary = DesktopErrorDto::new(
            "desktop.bridge.initial_snapshot_mismatch",
            "runtime",
            "fatal",
            false,
            "initial delivered snapshot does not match the actor snapshot cache",
        );
        return Err(cleanup_with_actor(actor, database, lease, primary));
    }

    let storage_runner = match DesktopStorageEffectRunner::start(
        storage_inbox,
        storage_dispatcher,
        Arc::new(handle.clone()),
        database.client(),
    ) {
        Ok(runner) => runner,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };

    let (platform_runner, current_snapshot) = match DesktopPlatformEffectRunner::start(
        platform_inbox,
        platform_dispatcher,
        handle.clone(),
        paths.clone(),
        Arc::clone(&network),
    ) {
        Ok(started) => started,
        Err(error) => {
            let mut primary = DesktopErrorDto::from(error);
            if let Err(storage_error) = storage_runner.shutdown() {
                primary = primary.with_appended_cleanup(Some(DesktopErrorDto::from(storage_error)));
            }
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };

    let snapshot = CoreSnapshotDto::from(current_snapshot);
    Ok((
        ReadyRuntime {
            profile_id,
            sources: paths.sources().to_path_buf(),
            identity,
            signing_identity,
            handle,
            notifications: Arc::clone(&notifications),
            network,
            owned: DesktopOwnedResources {
                platform_runner,
                storage_runner,
                notifications,
                actor,
                database,
                lease,
            },
        },
        snapshot,
    ))
}
