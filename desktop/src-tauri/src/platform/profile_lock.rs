use crate::platform::paths::{DesktopProfilePaths, ProfilePathError};
use crate::profile::ProfileId;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

const PROFILE_LOCK_FILENAME: &str = ".profile.lock";

/// Exclusive process-level lease for one mutable desktop profile.
///
/// The operating-system lock is advisory: all Silent Disco processes must use
/// this type before opening mutable profile state. The lock file itself is never
/// deleted as stale; kernel lock ownership ends automatically when the process or
/// file handle terminates.
#[must_use = "a profile lease must be retained for the complete mutable core lifetime"]
pub struct ProfileLease {
    profile_id: ProfileId,
    lock_path: PathBuf,
    file: Option<File>,
    release_attempted: bool,
}

impl ProfileLease {
    /// Acquires a nonblocking exclusive lock for one profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileLockError::ProfileInUse`] when another file handle or
    /// process holds the profile. Other path, open, lock, or validation failures
    /// remain distinct and visible.
    pub fn acquire(
        paths: &DesktopProfilePaths,
        profile_id: &ProfileId,
    ) -> Result<Self, ProfileLockError> {
        paths
            .prepare_directories()
            .map_err(ProfileLockError::PreparePaths)?;

        let lock_path = paths.root().join(PROFILE_LOCK_FILENAME);
        inspect_existing_lock_path(&lock_path)?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| ProfileLockError::OpenLockFile { source })?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(ProfileLockError::ProfileInUse),
            Err(TryLockError::Error(source)) => {
                return Err(ProfileLockError::AcquireLock { source });
            }
        }

        if let Err(primary) = validate_locked_path(paths.root(), &lock_path) {
            return Err(unlock_after_acquisition_failure(&file, primary));
        }

        Ok(Self {
            profile_id: profile_id.clone(),
            lock_path,
            file: Some(file),
            release_attempted: false,
        })
    }

    /// Returns the profile protected by this lease.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    /// Returns the internal lock path for diagnostics. Do not expose it through
    /// frontend IPC without the repository's redaction policy.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Explicitly releases the operating-system lock.
    ///
    /// This must happen only after core and database shutdown. Consuming `self`
    /// prevents subsequent use of the lease after release.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileLockError::ReleaseLock`] when explicit unlock fails. The
    /// file handle is still closed while unwinding this call, so the kernel gets a
    /// final opportunity to release the lock; the failure is not hidden.
    pub fn release(mut self) -> Result<(), ProfileLockError> {
        self.release_attempted = true;
        let file = self
            .file
            .take()
            .ok_or(ProfileLockError::LeaseAlreadyReleased)?;
        file.unlock()
            .map_err(|source| ProfileLockError::ReleaseLock { source })?;
        drop(file);
        Ok(())
    }
}

impl fmt::Debug for ProfileLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileLease")
            .field("profile_id", &self.profile_id)
            .field("lock_path", &self.lock_path)
            .field("held", &self.file.is_some())
            .finish()
    }
}

impl Drop for ProfileLease {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };

        let unlock_result = file.unlock();
        drop(file);

        if !self.release_attempted && !std::thread::panicking() {
            panic!(
                "ProfileLease for '{}' was dropped without explicit release; fallback unlock result: {unlock_result:?}",
                self.profile_id
            );
        }
    }
}

fn inspect_existing_lock_path(path: &Path) -> Result<(), ProfileLockError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ProfileLockError::LockFileSymlinkNotAllowed);
            }
            if !metadata.is_file() {
                return Err(ProfileLockError::LockPathNotRegularFile);
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProfileLockError::InspectLockFile { source }),
    }
}

fn validate_locked_path(profile_root: &Path, lock_path: &Path) -> Result<(), ProfileLockError> {
    inspect_existing_lock_path(lock_path)?;
    let canonical_root = fs::canonicalize(profile_root)
        .map_err(|source| ProfileLockError::CanonicalizeProfileRoot { source })?;
    let canonical_lock = fs::canonicalize(lock_path)
        .map_err(|source| ProfileLockError::CanonicalizeLockFile { source })?;
    if !canonical_lock.starts_with(canonical_root) {
        return Err(ProfileLockError::LockFileEscapedProfileRoot);
    }
    Ok(())
}

fn unlock_after_acquisition_failure(file: &File, primary: ProfileLockError) -> ProfileLockError {
    match file.unlock() {
        Ok(()) => primary,
        Err(cleanup) => ProfileLockError::AcquisitionAndUnlockFailed {
            primary: Box::new(primary),
            cleanup,
        },
    }
}

/// Failure while acquiring or releasing a desktop profile lease.
#[derive(Debug)]
pub enum ProfileLockError {
    /// Required profile paths could not be prepared safely.
    PreparePaths(ProfilePathError),
    /// A pre-existing lock path could not be inspected.
    InspectLockFile { source: std::io::Error },
    /// The lock path was a symbolic link.
    LockFileSymlinkNotAllowed,
    /// The lock path existed but was not a regular file.
    LockPathNotRegularFile,
    /// The lock file could not be opened.
    OpenLockFile { source: std::io::Error },
    /// Another process or handle currently owns the profile.
    ProfileInUse,
    /// The operating system returned a lock acquisition error.
    AcquireLock { source: std::io::Error },
    /// The profile root could not be canonicalized after locking.
    CanonicalizeProfileRoot { source: std::io::Error },
    /// The lock path could not be canonicalized after locking.
    CanonicalizeLockFile { source: std::io::Error },
    /// The canonical lock path escaped the canonical profile root.
    LockFileEscapedProfileRoot,
    /// Explicit lock release failed.
    ReleaseLock { source: std::io::Error },
    /// Release was requested after ownership was already consumed.
    LeaseAlreadyReleased,
    /// Validation failed after acquisition and cleanup unlock also failed.
    AcquisitionAndUnlockFailed {
        primary: Box<ProfileLockError>,
        cleanup: std::io::Error,
    },
}

impl fmt::Display for ProfileLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreparePaths(error) => {
                write!(formatter, "profile path preparation failed: {error}")
            }
            Self::InspectLockFile { source } => {
                write!(formatter, "could not inspect profile lock file: {source}")
            }
            Self::LockFileSymlinkNotAllowed => {
                formatter.write_str("profile lock file must not be a symbolic link")
            }
            Self::LockPathNotRegularFile => {
                formatter.write_str("profile lock path exists but is not a regular file")
            }
            Self::OpenLockFile { source } => {
                write!(formatter, "could not open profile lock file: {source}")
            }
            Self::ProfileInUse => formatter.write_str("desktop profile is already in use"),
            Self::AcquireLock { source } => {
                write!(formatter, "could not acquire profile lock: {source}")
            }
            Self::CanonicalizeProfileRoot { source } => {
                write!(formatter, "could not canonicalize profile root: {source}")
            }
            Self::CanonicalizeLockFile { source } => {
                write!(
                    formatter,
                    "could not canonicalize profile lock file: {source}"
                )
            }
            Self::LockFileEscapedProfileRoot => {
                formatter.write_str("profile lock file escaped the profile root")
            }
            Self::ReleaseLock { source } => {
                write!(formatter, "could not release profile lock: {source}")
            }
            Self::LeaseAlreadyReleased => formatter.write_str("profile lease was already released"),
            Self::AcquisitionAndUnlockFailed { primary, cleanup } => write!(
                formatter,
                "{primary}; cleanup unlock after acquisition also failed: {cleanup}"
            ),
        }
    }
}

impl std::error::Error for ProfileLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PreparePaths(source) => Some(source),
            Self::InspectLockFile { source }
            | Self::OpenLockFile { source }
            | Self::AcquireLock { source }
            | Self::CanonicalizeProfileRoot { source }
            | Self::CanonicalizeLockFile { source }
            | Self::ReleaseLock { source } => Some(source),
            Self::AcquisitionAndUnlockFailed { primary, .. } => Some(primary.as_ref()),
            Self::LockFileSymlinkNotAllowed
            | Self::LockPathNotRegularFile
            | Self::ProfileInUse
            | Self::LockFileEscapedProfileRoot
            | Self::LeaseAlreadyReleased => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProfileLease, ProfileLockError};
    use crate::platform::paths::DesktopProfilePaths;
    use crate::profile::ProfileId;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-desktop-lock-test-{}-{sequence}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("remove stale test directory");
            }
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    panic!("failed to remove test directory: {error}");
                }
            }
        }
    }

    fn paths(root: &TestDirectory, id: &ProfileId) -> DesktopProfilePaths {
        DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, id)
            .expect("valid profile paths")
    }

    #[test]
    fn second_handle_cannot_acquire_same_profile() {
        let root = TestDirectory::new();
        let id = ProfileId::parse("main").expect("valid ID");
        let paths = paths(&root, &id);
        let first = ProfileLease::acquire(&paths, &id).expect("acquire first lease");

        assert!(matches!(
            ProfileLease::acquire(&paths, &id),
            Err(ProfileLockError::ProfileInUse)
        ));

        first.release().expect("release first lease");
        ProfileLease::acquire(&paths, &id)
            .expect("reacquire after release")
            .release()
            .expect("release reacquired lease");
    }

    #[test]
    fn separate_profiles_can_be_locked_concurrently() {
        let root = TestDirectory::new();
        let first_id = ProfileId::parse("first").expect("valid ID");
        let second_id = ProfileId::parse("second").expect("valid ID");
        let first_paths = paths(&root, &first_id);
        let second_paths = paths(&root, &second_id);

        let first = ProfileLease::acquire(&first_paths, &first_id).expect("first lease");
        let second = ProfileLease::acquire(&second_paths, &second_id).expect("second lease");

        second.release().expect("release second lease");
        first.release().expect("release first lease");
    }

    #[cfg(unix)]
    #[test]
    fn lock_file_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let id = ProfileId::parse("main").expect("valid ID");
        let paths = paths(&root, &id);
        paths.prepare_directories().expect("prepare profile");
        let outside_file = outside.0.join("outside.lock");
        fs::write(&outside_file, b"outside").expect("write outside file");
        symlink(&outside_file, paths.root().join(".profile.lock")).expect("create lock symlink");

        assert!(matches!(
            ProfileLease::acquire(&paths, &id),
            Err(ProfileLockError::LockFileSymlinkNotAllowed)
        ));
        assert_eq!(
            fs::read(outside_file).expect("read outside file"),
            b"outside"
        );
    }
}
