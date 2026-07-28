use crate::platform::paths::DesktopProfilePaths;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use crate::profile::ProfileId;
use silent_disco_core::storage::{
    DatabaseConfig, DatabaseMetadata, DatabaseWorker, StorageError, StoredSettings, TrustedDevice,
};
use std::fmt;

/// Bounded typed data read from the Rust-owned domain database.
///
/// This is an internal backend record. IPC conversion must redact key references
/// and other sensitive fields before sending anything to the frontend.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileStorageInspection {
    pub metadata: DatabaseMetadata,
    pub settings: Option<StoredSettings>,
    pub trusted_devices: Vec<TrustedDevice>,
}

/// Opens, inspects, and closes one profile database through the shared Rust worker.
///
/// The operation acquires the profile lease before opening mutable storage, reads
/// only typed APIs, explicitly stops and joins the worker, and releases the lease
/// last. It never opens `SQLite` directly and never falls back to an in-memory or
/// alternate database.
///
/// # Errors
///
/// Returns [`ProfileStorageInspectionError`] while preserving database,
/// shutdown, and profile-lock release failures when more than one phase fails.
pub fn inspect_profile_storage(
    paths: &DesktopProfilePaths,
    profile_id: &ProfileId,
) -> Result<ProfileStorageInspection, ProfileStorageInspectionError> {
    let lease = ProfileLease::acquire(paths, profile_id)
        .map_err(ProfileStorageInspectionError::AcquireProfileLease)?;

    let config = match DatabaseConfig::new(paths.domain_database()) {
        Ok(config) => config,
        Err(primary) => return Err(release_after_database_start_failure(lease, primary)),
    };
    let worker = match DatabaseWorker::start(config) {
        Ok(worker) => worker,
        Err(primary) => return Err(release_after_database_start_failure(lease, primary)),
    };

    let client = worker.client();
    let inspection_result = client.metadata().and_then(|metadata| {
        let settings = client.load_settings()?;
        let trusted_devices = client.list_trusted_devices()?;
        Ok(ProfileStorageInspection {
            metadata,
            settings,
            trusted_devices,
        })
    });

    let shutdown_result = worker.stop_and_join();
    let release_result = lease.release();

    match (inspection_result, shutdown_result, release_result) {
        (Ok(inspection), Ok(()), Ok(())) => Ok(inspection),
        (Err(primary), shutdown, release) => {
            Err(ProfileStorageInspectionError::InspectionAndCleanup {
                primary,
                shutdown_error: shutdown.err(),
                release_error: release.err(),
            })
        }
        (Ok(_), Err(primary), release) => Err(ProfileStorageInspectionError::ShutdownAndRelease {
            primary,
            release_error: release.err(),
        }),
        (Ok(_), Ok(()), Err(primary)) => {
            Err(ProfileStorageInspectionError::ReleaseProfileLease(primary))
        }
    }
}

fn release_after_database_start_failure(
    lease: ProfileLease,
    primary: StorageError,
) -> ProfileStorageInspectionError {
    match lease.release() {
        Ok(()) => ProfileStorageInspectionError::OpenDatabase(primary),
        Err(release_error) => ProfileStorageInspectionError::OpenAndRelease {
            primary,
            release_error,
        },
    }
}

/// Failure while performing a complete one-shot desktop storage inspection.
#[derive(Debug)]
pub enum ProfileStorageInspectionError {
    /// The exclusive profile lease could not be acquired.
    AcquireProfileLease(ProfileLockError),
    /// Database configuration or startup failed and the lease released cleanly.
    OpenDatabase(StorageError),
    /// Database startup and profile-lease release both failed.
    OpenAndRelease {
        primary: StorageError,
        release_error: ProfileLockError,
    },
    /// A typed inspection query failed; cleanup outcomes remain attached.
    InspectionAndCleanup {
        primary: StorageError,
        shutdown_error: Option<StorageError>,
        release_error: Option<ProfileLockError>,
    },
    /// Database shutdown failed; lease release was still attempted afterward.
    ShutdownAndRelease {
        primary: StorageError,
        release_error: Option<ProfileLockError>,
    },
    /// Inspection and shutdown succeeded, but profile-lease release failed.
    ReleaseProfileLease(ProfileLockError),
}

impl fmt::Display for ProfileStorageInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcquireProfileLease(error) => {
                write!(formatter, "could not acquire profile lease: {error}")
            }
            Self::OpenDatabase(error) => {
                write!(formatter, "could not open profile database: {error}")
            }
            Self::OpenAndRelease {
                primary,
                release_error,
            } => write!(
                formatter,
                "could not open profile database: {primary}; profile lease release also failed: {release_error}"
            ),
            Self::InspectionAndCleanup {
                primary,
                shutdown_error,
                release_error,
            } => write!(
                formatter,
                "profile storage inspection failed: {primary}; shutdown: {}; lease release: {}",
                display_optional_error(shutdown_error.as_ref()),
                display_optional_error(release_error.as_ref())
            ),
            Self::ShutdownAndRelease {
                primary,
                release_error,
            } => write!(
                formatter,
                "profile database shutdown failed: {primary}; lease release: {}",
                display_optional_error(release_error.as_ref())
            ),
            Self::ReleaseProfileLease(error) => {
                write!(formatter, "profile lease release failed: {error}")
            }
        }
    }
}

fn display_optional_error(error: Option<&impl fmt::Display>) -> String {
    error.map_or_else(|| "ok".to_owned(), ToString::to_string)
}

impl std::error::Error for ProfileStorageInspectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AcquireProfileLease(source) | Self::ReleaseProfileLease(source) => Some(source),
            Self::OpenDatabase(source)
            | Self::OpenAndRelease {
                primary: source, ..
            }
            | Self::InspectionAndCleanup {
                primary: source, ..
            }
            | Self::ShutdownAndRelease {
                primary: source, ..
            } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProfileStorageInspectionError, inspect_profile_storage};
    use crate::platform::paths::DesktopProfilePaths;
    use crate::platform::profile_lock::ProfileLease;
    use crate::profile::ProfileId;
    use silent_disco_core::storage::LATEST_SCHEMA_VERSION;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-desktop-storage-inspection-{}-{sequence}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("remove stale storage-inspection test directory");
            }
            fs::create_dir_all(&path).expect("create storage-inspection test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                assert!(
                    error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                    "failed to remove storage-inspection test directory: {error}"
                );
            }
        }
    }

    fn profile(root: &TestDirectory) -> (ProfileId, DesktopProfilePaths) {
        let profile_id = ProfileId::parse("main").expect("valid profile ID");
        let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
            .expect("valid profile paths");
        (profile_id, paths)
    }

    #[test]
    fn first_open_creates_real_schema_and_reopen_reads_latest_schema() {
        let root = TestDirectory::new();
        let (profile_id, paths) = profile(&root);

        let first = inspect_profile_storage(&paths, &profile_id).expect("first inspection");
        assert_eq!(first.metadata.schema_version, LATEST_SCHEMA_VERSION);
        assert_eq!(first.metadata.journal_mode, "wal");
        assert!(first.metadata.foreign_keys_enabled);
        assert_eq!(first.metadata.integrity_check, "ok");
        assert!(first.settings.is_none());
        assert!(first.trusted_devices.is_empty());
        assert!(paths.domain_database().is_file());

        let second = inspect_profile_storage(&paths, &profile_id).expect("reopen inspection");
        assert_eq!(second.metadata.schema_version, LATEST_SCHEMA_VERSION);
        assert_eq!(
            second.metadata.applied_migrations,
            first.metadata.applied_migrations
        );
    }

    #[test]
    fn failed_database_open_releases_profile_lock_without_fallback() {
        let root = TestDirectory::new();
        let (profile_id, paths) = profile(&root);
        paths.prepare_directories().expect("prepare profile paths");
        fs::create_dir(paths.domain_database()).expect("create invalid database directory");

        assert!(matches!(
            inspect_profile_storage(&paths, &profile_id),
            Err(ProfileStorageInspectionError::OpenDatabase(_))
                | Err(ProfileStorageInspectionError::OpenAndRelease { .. })
        ));
        assert!(paths.domain_database().is_dir());
        assert!(!paths.root().join("database.sqlite3").exists());
        assert!(!paths.root().join("fallback.sqlite3").exists());

        ProfileLease::acquire(&paths, &profile_id)
            .expect("profile lock released after failed database open")
            .release()
            .expect("release verification lease");
    }

    #[test]
    fn corrupt_database_failure_preserves_file_and_releases_profile_lock() {
        let root = TestDirectory::new();
        let (profile_id, paths) = profile(&root);
        paths.prepare_directories().expect("prepare profile paths");
        let corrupt = b"this is not a sqlite database";
        fs::write(paths.domain_database(), corrupt).expect("write corrupt database");

        assert!(inspect_profile_storage(&paths, &profile_id).is_err());
        assert_eq!(
            fs::read(paths.domain_database()).expect("read preserved corrupt database"),
            corrupt
        );

        ProfileLease::acquire(&paths, &profile_id)
            .expect("profile lock released after corrupt database failure")
            .release()
            .expect("release verification lease");
    }
}
