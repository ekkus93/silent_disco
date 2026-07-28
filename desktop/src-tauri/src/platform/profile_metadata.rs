use crate::platform::paths::{DesktopProfilePaths, ProfilePathError};
use crate::profile::{ProfileId, ProfileMetadata, ProfileValidationError};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_PROFILE_METADATA_BYTES: usize = 16 * 1024;
const MAX_TEMP_FILE_ATTEMPTS: u64 = 32;
const MAX_TEMP_CLEANUP_ENTRIES: usize = 256;
const PROFILE_METADATA_TEMP_PREFIX: &str = ".profile.json.tmp-";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

/// Result of idempotent profile metadata initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMetadataInitialization {
    /// A new atomically visible metadata file was created.
    Created,
    /// An existing valid metadata file already contained the requested value.
    Existing,
}

/// Loads and validates profile metadata without repairing or replacing it.
///
/// # Errors
///
/// Returns [`ProfileMetadataError`] when the file is missing, is not a regular
/// file, exceeds the size limit, is malformed, or fails schema/profile validation.
pub fn load_profile_metadata(
    paths: &DesktopProfilePaths,
    expected_profile_id: &ProfileId,
) -> Result<ProfileMetadata, ProfileMetadataError> {
    let path = paths.metadata();
    let file_metadata = fs::symlink_metadata(path)
        .map_err(|source| ProfileMetadataError::InspectMetadata { source })?;
    if file_metadata.file_type().is_symlink() {
        return Err(ProfileMetadataError::MetadataSymlinkNotAllowed);
    }
    if !file_metadata.is_file() {
        return Err(ProfileMetadataError::MetadataNotRegularFile);
    }

    let file = File::open(path).map_err(|source| ProfileMetadataError::OpenMetadata { source })?;
    let mut bytes = Vec::new();
    file.take((MAX_PROFILE_METADATA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| ProfileMetadataError::ReadMetadata { source })?;
    if bytes.len() > MAX_PROFILE_METADATA_BYTES {
        return Err(ProfileMetadataError::MetadataTooLarge {
            maximum: MAX_PROFILE_METADATA_BYTES,
        });
    }

    let metadata: ProfileMetadata = serde_json::from_slice(&bytes)
        .map_err(|source| ProfileMetadataError::DeserializeMetadata { source })?;
    metadata
        .validate_for(expected_profile_id)
        .map_err(ProfileMetadataError::Validation)?;
    Ok(metadata)
}

/// Creates profile metadata atomically without overwriting an existing file.
///
/// The function writes and synchronizes a unique temporary file in the profile
/// directory, then creates the final name with a same-filesystem hard link. The
/// hard-link step is atomic and fails if another process already created the final
/// file. Block 6 will add the process-level profile lease; this function remains
/// race-safe and no-clobber independently.
///
/// # Errors
///
/// Returns [`ProfileMetadataError`] for path preparation, serialization, I/O,
/// conflicting existing metadata, unsupported hard-link behavior, or finalization
/// failure. A malformed existing file is preserved and returned as an error.
pub fn initialize_profile_metadata(
    paths: &DesktopProfilePaths,
    metadata: &ProfileMetadata,
) -> Result<ProfileMetadataInitialization, ProfileMetadataError> {
    metadata
        .validate_for(metadata.profile_id())
        .map_err(ProfileMetadataError::Validation)?;
    paths
        .prepare_directories()
        .map_err(ProfileMetadataError::PreparePaths)?;

    match fs::symlink_metadata(paths.metadata()) {
        Ok(_) => return compare_existing_metadata(paths, metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ProfileMetadataError::InspectMetadata { source });
        }
    }

    let mut encoded = serde_json::to_vec(metadata)
        .map_err(|source| ProfileMetadataError::SerializeMetadata { source })?;
    encoded.push(b'\n');
    if encoded.len() > MAX_PROFILE_METADATA_BYTES {
        return Err(ProfileMetadataError::MetadataTooLarge {
            maximum: MAX_PROFILE_METADATA_BYTES,
        });
    }

    let parent = paths
        .metadata()
        .parent()
        .ok_or(ProfileMetadataError::MetadataParentMissing)?;
    let (temporary_path, mut temporary_file) = reserve_temporary_file(parent)?;

    if let Err(source) = temporary_file.write_all(&encoded) {
        return Err(remove_temporary_after_failure(
            &temporary_path,
            ProfileMetadataError::WriteTemporary { source },
        ));
    }
    if let Err(source) = temporary_file.sync_all() {
        return Err(remove_temporary_after_failure(
            &temporary_path,
            ProfileMetadataError::SyncTemporary { source },
        ));
    }
    drop(temporary_file);

    match fs::hard_link(&temporary_path, paths.metadata()) {
        Ok(()) => finalize_committed_metadata(parent, &temporary_path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            remove_temporary(&temporary_path)?;
            return compare_existing_metadata(paths, metadata);
        }
        Err(source) => {
            return Err(remove_temporary_after_failure(
                &temporary_path,
                ProfileMetadataError::PublishMetadata { source },
            ));
        }
    }

    Ok(ProfileMetadataInitialization::Created)
}

/// Removes only regular files bearing the private metadata temporary-file prefix.
///
/// Call this only after acquiring the Block 6 profile lease. The scan is bounded,
/// never follows symlinks, and does not remove unrelated files.
///
/// # Errors
///
/// Returns [`ProfileMetadataError`] when the directory scan exceeds its bound, an
/// apparent temporary entry is not a regular file, or a removal fails.
pub fn cleanup_incomplete_profile_metadata_files(
    paths: &DesktopProfilePaths,
) -> Result<usize, ProfileMetadataError> {
    let entries = fs::read_dir(paths.root())
        .map_err(|source| ProfileMetadataError::ReadProfileDirectory { source })?;
    let mut inspected = 0_usize;
    let mut removed = 0_usize;

    for entry_result in entries {
        inspected = inspected.saturating_add(1);
        if inspected > MAX_TEMP_CLEANUP_ENTRIES {
            return Err(ProfileMetadataError::CleanupEntryLimitExceeded {
                maximum: MAX_TEMP_CLEANUP_ENTRIES,
            });
        }

        let entry =
            entry_result.map_err(|source| ProfileMetadataError::ReadProfileDirectory { source })?;
        if !entry
            .file_name()
            .as_encoded_bytes()
            .starts_with(PROFILE_METADATA_TEMP_PREFIX.as_bytes())
        {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|source| ProfileMetadataError::InspectTemporary { source })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(ProfileMetadataError::UnsafeTemporaryEntry);
        }
        fs::remove_file(entry.path())
            .map_err(|source| ProfileMetadataError::RemoveTemporary { source })?;
        removed = removed.saturating_add(1);
    }

    Ok(removed)
}

fn compare_existing_metadata(
    paths: &DesktopProfilePaths,
    requested: &ProfileMetadata,
) -> Result<ProfileMetadataInitialization, ProfileMetadataError> {
    let existing = load_profile_metadata(paths, requested.profile_id())?;
    if existing == *requested {
        Ok(ProfileMetadataInitialization::Existing)
    } else {
        Err(ProfileMetadataError::ExistingMetadataConflict)
    }
}

fn reserve_temporary_file(parent: &Path) -> Result<(PathBuf, File), ProfileMetadataError> {
    for _ in 0..MAX_TEMP_FILE_ATTEMPTS {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let filename = format!(
            "{PROFILE_METADATA_TEMP_PREFIX}{}-{sequence}",
            std::process::id()
        );
        let path = parent.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(ProfileMetadataError::ReserveTemporary { source });
            }
        }
    }

    Err(ProfileMetadataError::TemporaryNameExhausted {
        attempts: MAX_TEMP_FILE_ATTEMPTS,
    })
}

fn finalize_committed_metadata(
    parent: &Path,
    temporary_path: &Path,
) -> Result<(), ProfileMetadataError> {
    let cleanup_error = match fs::remove_file(temporary_path) {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(error),
    };
    let directory_sync_error = sync_parent_directory(parent).err();

    if cleanup_error.is_some() || directory_sync_error.is_some() {
        return Err(ProfileMetadataError::CommittedFinalizationFailed {
            cleanup_error,
            directory_sync_error,
        });
    }
    Ok(())
}

fn remove_temporary(path: &Path) -> Result<(), ProfileMetadataError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProfileMetadataError::RemoveTemporary { source }),
    }
}

fn remove_temporary_after_failure(
    path: &Path,
    primary: ProfileMetadataError,
) -> ProfileMetadataError {
    match fs::remove_file(path) {
        Ok(()) => primary,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => primary,
        Err(cleanup) => ProfileMetadataError::PrimaryAndCleanup {
            primary: Box::new(primary),
            cleanup,
        },
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), std::io::Error> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

/// Failure while reading, creating, or cleaning profile metadata.
#[derive(Debug)]
pub enum ProfileMetadataError {
    /// Required profile directories could not be prepared safely.
    PreparePaths(ProfilePathError),
    /// The metadata path could not be inspected.
    InspectMetadata { source: std::io::Error },
    /// Metadata must never be a symbolic link.
    MetadataSymlinkNotAllowed,
    /// Metadata existed but was not a regular file.
    MetadataNotRegularFile,
    /// Metadata could not be opened.
    OpenMetadata { source: std::io::Error },
    /// Metadata could not be read.
    ReadMetadata { source: std::io::Error },
    /// Metadata exceeded its configured byte bound.
    MetadataTooLarge { maximum: usize },
    /// Metadata JSON could not be decoded.
    DeserializeMetadata { source: serde_json::Error },
    /// Metadata failed schema, identifier, or field validation.
    Validation(ProfileValidationError),
    /// Metadata could not be encoded.
    SerializeMetadata { source: serde_json::Error },
    /// The metadata path had no parent directory.
    MetadataParentMissing,
    /// A unique temporary file could not be reserved.
    ReserveTemporary { source: std::io::Error },
    /// Every bounded temporary filename attempt collided.
    TemporaryNameExhausted { attempts: u64 },
    /// The temporary file could not be fully written.
    WriteTemporary { source: std::io::Error },
    /// The temporary file could not be synchronized.
    SyncTemporary { source: std::io::Error },
    /// The atomic no-clobber publication operation failed.
    PublishMetadata { source: std::io::Error },
    /// Existing valid metadata differs from the requested metadata.
    ExistingMetadataConflict,
    /// A temporary file could not be removed.
    RemoveTemporary { source: std::io::Error },
    /// The metadata file is committed, but cleanup or directory sync failed.
    CommittedFinalizationFailed {
        cleanup_error: Option<std::io::Error>,
        directory_sync_error: Option<std::io::Error>,
    },
    /// Both the primary operation and its cleanup failed.
    PrimaryAndCleanup {
        primary: Box<ProfileMetadataError>,
        cleanup: std::io::Error,
    },
    /// The profile directory could not be enumerated.
    ReadProfileDirectory { source: std::io::Error },
    /// A possible temporary entry could not be inspected.
    InspectTemporary { source: std::io::Error },
    /// A matching cleanup entry was a symlink or non-file.
    UnsafeTemporaryEntry,
    /// The bounded cleanup scan encountered too many entries.
    CleanupEntryLimitExceeded { maximum: usize },
}

impl fmt::Display for ProfileMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreparePaths(error) => {
                write!(formatter, "profile path preparation failed: {error}")
            }
            Self::InspectMetadata { source } => {
                write!(formatter, "could not inspect profile metadata: {source}")
            }
            Self::MetadataSymlinkNotAllowed => {
                formatter.write_str("profile metadata must not be a symbolic link")
            }
            Self::MetadataNotRegularFile => {
                formatter.write_str("profile metadata exists but is not a regular file")
            }
            Self::OpenMetadata { source } => {
                write!(formatter, "could not open profile metadata: {source}")
            }
            Self::ReadMetadata { source } => {
                write!(formatter, "could not read profile metadata: {source}")
            }
            Self::MetadataTooLarge { maximum } => {
                write!(
                    formatter,
                    "profile metadata exceeds the {maximum}-byte limit"
                )
            }
            Self::DeserializeMetadata { source } => {
                write!(formatter, "profile metadata is malformed: {source}")
            }
            Self::Validation(error) => write!(formatter, "profile metadata is invalid: {error}"),
            Self::SerializeMetadata { source } => {
                write!(formatter, "could not serialize profile metadata: {source}")
            }
            Self::MetadataParentMissing => {
                formatter.write_str("profile metadata path has no parent directory")
            }
            Self::ReserveTemporary { source } => {
                write!(
                    formatter,
                    "could not reserve metadata temporary file: {source}"
                )
            }
            Self::TemporaryNameExhausted { attempts } => write!(
                formatter,
                "could not reserve metadata temporary file after {attempts} attempts"
            ),
            Self::WriteTemporary { source } => {
                write!(
                    formatter,
                    "could not write metadata temporary file: {source}"
                )
            }
            Self::SyncTemporary { source } => {
                write!(
                    formatter,
                    "could not synchronize metadata temporary file: {source}"
                )
            }
            Self::PublishMetadata { source } => {
                write!(
                    formatter,
                    "could not publish profile metadata atomically: {source}"
                )
            }
            Self::ExistingMetadataConflict => formatter
                .write_str("existing profile metadata conflicts with the requested metadata"),
            Self::RemoveTemporary { source } => {
                write!(
                    formatter,
                    "could not remove metadata temporary file: {source}"
                )
            }
            Self::CommittedFinalizationFailed {
                cleanup_error,
                directory_sync_error,
            } => write!(
                formatter,
                "profile metadata was committed but finalization failed (cleanup: {}, directory sync: {})",
                cleanup_error
                    .as_ref()
                    .map_or_else(|| "ok".to_owned(), ToString::to_string),
                directory_sync_error
                    .as_ref()
                    .map_or_else(|| "ok".to_owned(), ToString::to_string)
            ),
            Self::PrimaryAndCleanup { primary, cleanup } => write!(
                formatter,
                "{primary}; metadata temporary cleanup also failed: {cleanup}"
            ),
            Self::ReadProfileDirectory { source } => {
                write!(formatter, "could not read profile directory: {source}")
            }
            Self::InspectTemporary { source } => {
                write!(
                    formatter,
                    "could not inspect metadata temporary entry: {source}"
                )
            }
            Self::UnsafeTemporaryEntry => formatter
                .write_str("metadata temporary cleanup refused a symlink or non-regular entry"),
            Self::CleanupEntryLimitExceeded { maximum } => write!(
                formatter,
                "metadata temporary cleanup exceeded its {maximum}-entry scan limit"
            ),
        }
    }
}

impl std::error::Error for ProfileMetadataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PreparePaths(source) => Some(source),
            Self::InspectMetadata { source }
            | Self::OpenMetadata { source }
            | Self::ReadMetadata { source }
            | Self::ReserveTemporary { source }
            | Self::WriteTemporary { source }
            | Self::SyncTemporary { source }
            | Self::PublishMetadata { source }
            | Self::RemoveTemporary { source }
            | Self::ReadProfileDirectory { source }
            | Self::InspectTemporary { source } => Some(source),
            Self::DeserializeMetadata { source } | Self::SerializeMetadata { source } => {
                Some(source)
            }
            Self::Validation(source) => Some(source),
            Self::PrimaryAndCleanup { primary, .. } => Some(primary.as_ref()),
            Self::MetadataSymlinkNotAllowed
            | Self::MetadataNotRegularFile
            | Self::MetadataTooLarge { .. }
            | Self::MetadataParentMissing
            | Self::TemporaryNameExhausted { .. }
            | Self::ExistingMetadataConflict
            | Self::CommittedFinalizationFailed { .. }
            | Self::UnsafeTemporaryEntry
            | Self::CleanupEntryLimitExceeded { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PROFILE_METADATA_TEMP_PREFIX, ProfileMetadataError, ProfileMetadataInitialization,
        cleanup_incomplete_profile_metadata_files, initialize_profile_metadata,
        load_profile_metadata,
    };
    use crate::platform::paths::DesktopProfilePaths;
    use crate::profile::{ProfileDisplayName, ProfileId, ProfileMetadata};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-desktop-metadata-test-{}-{sequence}",
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

    fn profile(test_root: &TestDirectory) -> (DesktopProfilePaths, ProfileMetadata) {
        let id = ProfileId::parse("main").expect("valid profile ID");
        let name = ProfileDisplayName::parse("Main Profile").expect("valid display name");
        let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&test_root.0, &id)
            .expect("valid profile paths");
        (paths, ProfileMetadata::new(id, name))
    }

    #[test]
    fn initializes_loads_and_reopens_idempotently() {
        let test_root = TestDirectory::new();
        let (paths, metadata) = profile(&test_root);

        assert_eq!(
            initialize_profile_metadata(&paths, &metadata).expect("initialize metadata"),
            ProfileMetadataInitialization::Created
        );
        assert_eq!(
            load_profile_metadata(&paths, metadata.profile_id()).expect("load metadata"),
            metadata
        );
        assert_eq!(
            initialize_profile_metadata(&paths, &metadata).expect("idempotent initialize"),
            ProfileMetadataInitialization::Existing
        );
    }

    #[test]
    fn malformed_existing_metadata_is_preserved() {
        let test_root = TestDirectory::new();
        let (paths, metadata) = profile(&test_root);
        paths.prepare_directories().expect("prepare profile");
        fs::write(paths.metadata(), b"{broken").expect("write malformed metadata");

        assert!(matches!(
            initialize_profile_metadata(&paths, &metadata),
            Err(ProfileMetadataError::DeserializeMetadata { .. })
        ));
        assert_eq!(
            fs::read(paths.metadata()).expect("read preserved file"),
            b"{broken"
        );
    }

    #[test]
    fn conflicting_existing_metadata_is_not_overwritten() {
        let test_root = TestDirectory::new();
        let (paths, metadata) = profile(&test_root);
        initialize_profile_metadata(&paths, &metadata).expect("initialize metadata");
        let changed = ProfileMetadata::new(
            metadata.profile_id().clone(),
            ProfileDisplayName::parse("Renamed").expect("valid display name"),
        );

        assert!(matches!(
            initialize_profile_metadata(&paths, &changed),
            Err(ProfileMetadataError::ExistingMetadataConflict)
        ));
        assert_eq!(
            load_profile_metadata(&paths, metadata.profile_id()).expect("load original metadata"),
            metadata
        );
    }

    #[test]
    fn cleanup_removes_only_owned_regular_temporary_files() {
        let test_root = TestDirectory::new();
        let (paths, _) = profile(&test_root);
        paths.prepare_directories().expect("prepare profile");
        let temporary = paths
            .root()
            .join(format!("{PROFILE_METADATA_TEMP_PREFIX}test"));
        let unrelated = paths.root().join("keep.txt");
        fs::write(&temporary, b"partial").expect("write temporary file");
        fs::write(&unrelated, b"keep").expect("write unrelated file");

        assert_eq!(
            cleanup_incomplete_profile_metadata_files(&paths).expect("cleanup temporary files"),
            1
        );
        assert!(!temporary.exists());
        assert!(unrelated.exists());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_symlink_is_rejected_without_following_it() {
        use std::os::unix::fs::symlink;

        let test_root = TestDirectory::new();
        let outside = TestDirectory::new();
        let (paths, metadata) = profile(&test_root);
        paths.prepare_directories().expect("prepare profile");
        let target = outside.0.join("outside.json");
        fs::write(&target, b"outside").expect("write outside target");
        symlink(&target, paths.metadata()).expect("create metadata symlink");

        assert!(matches!(
            initialize_profile_metadata(&paths, &metadata),
            Err(ProfileMetadataError::MetadataSymlinkNotAllowed)
        ));
        assert_eq!(fs::read(target).expect("read outside target"), b"outside");
    }
}
