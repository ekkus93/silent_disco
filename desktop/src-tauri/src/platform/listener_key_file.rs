//! Owner-only key file for the desktop listener's proof key.
//!
//! The listener proves possession of this P-256 key over TLS (see
//! `silent_disco_core::transport::secure_listener_proof`). The key lives in a
//! file in the profile's app data directory, readable only by its owner
//! (`0600`, inside a `0700` directory).
//!
//! The file is never silently repaired. If it exists but has the wrong mode,
//! is a symlink, is malformed, or holds an invalid scalar, loading fails and
//! the file is left untouched. A new key is generated only when no file exists
//! at all. Regenerating over a damaged key would silently change the listener's
//! identity, which the trust policy must see as a change, not as a fresh device.
//!
//! This store is separate from the host's OS-keyring identity in `identity.rs`.
//! It is supported only on Unix, where owner-only permissions can be enforced;
//! other platforms fail closed.

use std::path::Path;

use p256::ecdsa::SigningKey;

/// File name inside the profile's app data directory.
pub const LISTENER_KEY_FILE_NAME: &str = "listener-proof-key.v1";
/// A P-256 scalar is 32 bytes.
const KEY_FILE_BYTES: usize = 32;

#[derive(Debug)]
pub enum ListenerKeyFileError {
    /// Owner-only permissions cannot be enforced on this platform.
    UnsupportedPlatform,
    /// The app data directory is missing, not a directory, a symlink, or
    /// readable or writable by group or others.
    DirectoryNotOwnerOnly { mode: u32 },
    /// The key path exists but is a symlink.
    SymlinkRefused,
    /// The key path exists but is not a regular file.
    NotRegularFile,
    /// The key file's mode is not exactly `0600`. It is refused, not repaired.
    KeyFileModeNotOwnerOnly { mode: u32 },
    /// The key file does not hold exactly 32 bytes.
    MalformedKeyFile { length: usize },
    /// The stored scalar is zero or not below the curve order.
    InvalidScalar,
    /// The system random source failed.
    RandomUnavailable,
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ListenerKeyFileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter
                .write_str("owner-only listener key files are not supported on this platform"),
            Self::DirectoryNotOwnerOnly { mode } => write!(
                formatter,
                "app data directory must be owner-only (found mode {mode:o})"
            ),
            Self::SymlinkRefused => formatter.write_str("listener key path is a symlink"),
            Self::NotRegularFile => formatter.write_str("listener key path is not a regular file"),
            Self::KeyFileModeNotOwnerOnly { mode } => write!(
                formatter,
                "listener key file must be mode 0600 (found {mode:o}); it was not changed"
            ),
            Self::MalformedKeyFile { length } => write!(
                formatter,
                "listener key file holds {length} bytes, expected 32; it was not changed"
            ),
            Self::InvalidScalar => formatter
                .write_str("listener key file holds an invalid P-256 scalar; it was not changed"),
            Self::RandomUnavailable => formatter.write_str("system random source is unavailable"),
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
        }
    }
}

impl std::error::Error for ListenerKeyFileError {}

/// Loads the listener key from `directory`, or creates it if no file exists.
///
/// # Errors
/// Returns an error, and changes nothing, when the directory or file is not
/// owner-only, is a symlink, is malformed or holds an invalid scalar. Also
/// returns an error on platforms without owner-only file permissions.
pub fn load_or_create_listener_key(directory: &Path) -> Result<SigningKey, ListenerKeyFileError> {
    #[cfg(unix)]
    {
        unix::load_or_create(directory)
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(ListenerKeyFileError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
mod unix {
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    use p256::ecdsa::SigningKey;

    use super::{KEY_FILE_BYTES, LISTENER_KEY_FILE_NAME, ListenerKeyFileError};

    const OWNER_ONLY_FILE: u32 = 0o600;
    const OWNER_ONLY_DIRECTORY: u32 = 0o700;

    fn io(operation: &'static str, source: std::io::Error) -> ListenerKeyFileError {
        ListenerKeyFileError::Io { operation, source }
    }

    pub(super) fn load_or_create(directory: &Path) -> Result<SigningKey, ListenerKeyFileError> {
        ensure_owner_only_directory(directory)?;
        let path = directory.join(LISTENER_KEY_FILE_NAME);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(ListenerKeyFileError::SymlinkRefused);
                }
                if !metadata.is_file() {
                    return Err(ListenerKeyFileError::NotRegularFile);
                }
                let mode = metadata.mode() & 0o777;
                if mode != OWNER_ONLY_FILE {
                    return Err(ListenerKeyFileError::KeyFileModeNotOwnerOnly { mode });
                }
                read_existing(&path)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_new(&path),
            Err(error) => Err(io("inspect listener key file", error)),
        }
    }

    fn ensure_owner_only_directory(directory: &Path) -> Result<(), ListenerKeyFileError> {
        match fs::symlink_metadata(directory) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(ListenerKeyFileError::DirectoryNotOwnerOnly {
                        mode: metadata.mode() & 0o777,
                    });
                }
                let mode = metadata.mode() & 0o777;
                if mode & 0o077 != 0 {
                    return Err(ListenerKeyFileError::DirectoryNotOwnerOnly { mode });
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(OWNER_ONLY_DIRECTORY)
                    .create(directory)
                    .map_err(|source| io("create listener key directory", source))?;
                Ok(())
            }
            Err(error) => Err(io("inspect listener key directory", error)),
        }
    }

    fn read_existing(path: &Path) -> Result<SigningKey, ListenerKeyFileError> {
        let mut file = File::open(path).map_err(|source| io("open listener key file", source))?;
        let opened = file
            .metadata()
            .map_err(|source| io("inspect opened listener key file", source))?;
        if !opened.is_file() {
            return Err(ListenerKeyFileError::NotRegularFile);
        }
        let mode = opened.mode() & 0o777;
        if mode != OWNER_ONLY_FILE {
            return Err(ListenerKeyFileError::KeyFileModeNotOwnerOnly { mode });
        }
        let mut bytes = Vec::with_capacity(KEY_FILE_BYTES + 1);
        file.read_to_end(&mut bytes)
            .map_err(|source| io("read listener key file", source))?;
        if bytes.len() != KEY_FILE_BYTES {
            return Err(ListenerKeyFileError::MalformedKeyFile {
                length: bytes.len(),
            });
        }
        SigningKey::from_slice(&bytes).map_err(|_| ListenerKeyFileError::InvalidScalar)
    }

    fn create_new(path: &Path) -> Result<SigningKey, ListenerKeyFileError> {
        let key = generate_key()?;
        let bytes = key.to_bytes();
        // `create_new` never overwrites an existing file, so a concurrent
        // creator makes this call fail instead of replacing a key.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(OWNER_ONLY_FILE)
            .open(path)
            .map_err(|source| io("create listener key file", source))?;
        file.write_all(bytes.as_slice())
            .map_err(|source| io("write listener key file", source))?;
        file.sync_all()
            .map_err(|source| io("sync listener key file", source))?;
        // The umask can only remove bits from the creation mode, but set the
        // final mode explicitly so the result never depends on the umask.
        fs::set_permissions(path, fs::Permissions::from_mode(OWNER_ONLY_FILE))
            .map_err(|source| io("set listener key file mode", source))?;
        Ok(key)
    }

    /// Draws 32 random bytes until they form a valid P-256 scalar. The chance
    /// of a rejected draw is about 2^-32, so the loop almost never repeats.
    fn generate_key() -> Result<SigningKey, ListenerKeyFileError> {
        loop {
            let mut candidate = [0_u8; KEY_FILE_BYTES];
            getrandom::fill(&mut candidate).map_err(|_| ListenerKeyFileError::RandomUnavailable)?;
            if let Ok(key) = SigningKey::from_slice(&candidate) {
                return Ok(key);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::Path;

    use p256::pkcs8::EncodePublicKey;

    use super::{LISTENER_KEY_FILE_NAME, ListenerKeyFileError, load_or_create_listener_key};

    fn public_der(directory: &Path) -> Vec<u8> {
        load_or_create_listener_key(directory)
            .expect("load")
            .verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec()
    }

    fn key_path(directory: &Path) -> std::path::PathBuf {
        directory.join(LISTENER_KEY_FILE_NAME)
    }

    fn owner_only_directory() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("tempdir");
        let directory = root.path().join("profile");
        fs::create_dir(&directory).expect("directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("mode");
        root
    }

    #[test]
    fn creates_an_owner_only_key_file_and_reloads_the_same_public_key() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        let created = public_der(&directory);

        let mode = fs::metadata(key_path(&directory))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the key file must be owner-only");
        assert_eq!(
            public_der(&directory),
            created,
            "reload must keep the same identity"
        );
    }

    #[test]
    fn creates_the_directory_owner_only_when_it_is_missing() {
        let root = tempfile::tempdir().expect("tempdir");
        let directory = root.path().join("fresh-profile");
        public_der(&directory);
        let mode = fs::metadata(&directory)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn refuses_a_key_file_readable_by_group_or_others_and_leaves_it_unchanged() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        public_der(&directory);
        let path = key_path(&directory);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("widen");
        let before = fs::read(&path).expect("read");

        let error = load_or_create_listener_key(&directory).expect_err("must refuse");
        assert!(matches!(
            error,
            ListenerKeyFileError::KeyFileModeNotOwnerOnly { mode: 0o644 }
        ));
        assert_eq!(
            fs::read(&path).expect("read"),
            before,
            "the file must not be rewritten"
        );
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o644,
            "the mode must not be silently repaired"
        );
    }

    #[test]
    fn refuses_a_symlinked_key_path_and_creates_nothing_behind_it() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        let target = root.path().join("elsewhere.key");
        fs::write(&target, [7_u8; 32]).expect("target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("mode");
        symlink(&target, key_path(&directory)).expect("symlink");

        let error = load_or_create_listener_key(&directory).expect_err("must refuse");
        assert!(matches!(error, ListenerKeyFileError::SymlinkRefused));
        assert_eq!(fs::read(&target).expect("read"), vec![7_u8; 32]);
    }

    #[test]
    fn refuses_a_truncated_key_file_without_regenerating_it() {
        let root = owner_only_directory();
        let directory = root.path().join("profile");
        public_der(&directory);
        let path = key_path(&directory);
        fs::write(&path, [9_u8; 31]).expect("truncate");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");

        let error = load_or_create_listener_key(&directory).expect_err("must refuse");
        assert!(matches!(
            error,
            ListenerKeyFileError::MalformedKeyFile { length: 31 }
        ));
        assert_eq!(
            fs::read(&path).expect("read"),
            vec![9_u8; 31],
            "a damaged key must not be replaced"
        );
    }

    #[test]
    fn refuses_zero_and_out_of_range_scalars() {
        for scalar in [[0_u8; 32], [0xff_u8; 32]] {
            let root = owner_only_directory();
            let directory = root.path().join("profile");
            fs::create_dir_all(&directory).expect("directory");
            let path = key_path(&directory);
            fs::write(&path, scalar).expect("write");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
            let error = load_or_create_listener_key(&directory).expect_err("must refuse");
            assert!(
                matches!(error, ListenerKeyFileError::InvalidScalar),
                "scalar {scalar:?}"
            );
            assert_eq!(fs::read(&path).expect("read"), scalar.to_vec());
        }
    }

    #[test]
    fn refuses_a_directory_readable_by_others_and_creates_nothing_inside() {
        let root = tempfile::tempdir().expect("tempdir");
        let directory = root.path().join("shared");
        fs::create_dir(&directory).expect("directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).expect("mode");

        let error = load_or_create_listener_key(&directory).expect_err("must refuse");
        assert!(matches!(
            error,
            ListenerKeyFileError::DirectoryNotOwnerOnly { mode: 0o755 }
        ));
        assert!(!key_path(&directory).exists());
    }
}
