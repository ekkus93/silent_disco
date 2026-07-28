use crate::profile::ProfileId;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

/// Deterministic application-owned paths for one desktop profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopProfilePaths {
    profiles_root: PathBuf,
    root: PathBuf,
    domain_database: PathBuf,
    p2_database: Option<PathBuf>,
    metadata: PathBuf,
    sources: PathBuf,
    diagnostics: PathBuf,
    cache: PathBuf,
}

impl DesktopProfilePaths {
    /// Constructs profile paths below a trusted absolute application-local-data root.
    ///
    /// The complete root is never accepted from frontend input. Callers provide only
    /// a validated [`ProfileId`], while Tauri or a test harness supplies the trusted
    /// application root.
    ///
    /// # Errors
    ///
    /// Returns [`ProfilePathError`] when the trusted root is relative or contains a
    /// lexical parent traversal component.
    pub fn from_trusted_app_local_data_root(
        app_local_data_root: &Path,
        profile_id: &ProfileId,
    ) -> Result<Self, ProfilePathError> {
        validate_trusted_root(app_local_data_root)?;

        let profiles_root = app_local_data_root.join("profiles");
        let root = profiles_root.join(profile_id.as_str());
        if !root.starts_with(&profiles_root) {
            return Err(ProfilePathError::ProfileEscapedTrustedRoot);
        }

        Ok(Self {
            profiles_root,
            domain_database: root.join("silent-disco.sqlite3"),
            p2_database: None,
            metadata: root.join("profile.json"),
            sources: root.join("sources"),
            diagnostics: root.join("diagnostics"),
            cache: root.join("cache"),
            root,
        })
    }

    /// Creates and validates only the directories owned by this profile.
    ///
    /// This method rejects pre-existing symlinks for the profile root and each
    /// writable child directory. It does not create or open either database file.
    ///
    /// # Errors
    ///
    /// Returns [`ProfilePathError`] when a directory cannot be created, is not a
    /// directory, is a symlink, cannot be canonicalized, or escapes its trusted root.
    pub fn prepare_directories(&self) -> Result<(), ProfilePathError> {
        fs::create_dir_all(&self.profiles_root).map_err(|source| {
            ProfilePathError::CreateDirectory {
                operation: "create profiles root",
                source,
            }
        })?;
        reject_symlink_or_non_directory("profiles root", &self.profiles_root)?;
        let canonical_profiles_root = canonicalize("profiles root", &self.profiles_root)?;

        ensure_owned_directory("profile root", &self.root, Some(&canonical_profiles_root))?;
        let canonical_profile_root = canonicalize("profile root", &self.root)?;

        for (label, path) in [
            ("source directory", self.sources.as_path()),
            ("diagnostics directory", self.diagnostics.as_path()),
            ("cache directory", self.cache.as_path()),
        ] {
            ensure_owned_directory(label, path, Some(&canonical_profile_root))?;
        }

        Ok(())
    }

    /// Returns the internal profile root. Do not send this through IPC without an
    /// explicit redaction policy.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the complete path passed to the Rust-owned domain database worker.
    #[must_use]
    pub fn domain_database(&self) -> &Path {
        &self.domain_database
    }

    /// Returns the optional P2 database path when a separate store is selected.
    #[must_use]
    pub fn p2_database(&self) -> Option<&Path> {
        self.p2_database.as_deref()
    }

    /// Returns the versioned profile metadata path.
    #[must_use]
    pub fn metadata(&self) -> &Path {
        &self.metadata
    }

    /// Returns the application-owned staged-source directory.
    #[must_use]
    pub fn sources(&self) -> &Path {
        &self.sources
    }

    /// Returns the application-owned diagnostics directory.
    #[must_use]
    pub fn diagnostics(&self) -> &Path {
        &self.diagnostics
    }

    /// Returns the disposable profile cache directory.
    #[must_use]
    pub fn cache(&self) -> &Path {
        &self.cache
    }
}

/// Resolves profile paths from Tauri's application-local-data directory.
///
/// # Errors
///
/// Returns [`ProfilePathError`] when Tauri cannot resolve the application path or
/// when path construction fails validation.
pub fn resolve_profile_paths<R: Runtime>(
    app: &AppHandle<R>,
    profile_id: &ProfileId,
) -> Result<DesktopProfilePaths, ProfilePathError> {
    let app_local_data_root = app
        .path()
        .app_local_data_dir()
        .map_err(|error| ProfilePathError::TauriPathResolution(error.to_string()))?;
    DesktopProfilePaths::from_trusted_app_local_data_root(&app_local_data_root, profile_id)
}

fn validate_trusted_root(root: &Path) -> Result<(), ProfilePathError> {
    if !root.is_absolute() {
        return Err(ProfilePathError::TrustedRootNotAbsolute);
    }
    if root
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ProfilePathError::TrustedRootContainsTraversal);
    }
    Ok(())
}

fn ensure_owned_directory(
    label: &'static str,
    path: &Path,
    canonical_parent: Option<&Path>,
) -> Result<(), ProfilePathError> {
    match fs::symlink_metadata(path) {
        Ok(_) => reject_symlink_or_non_directory(label, path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| ProfilePathError::CreateDirectory {
                operation: label,
                source,
            })?;
            reject_symlink_or_non_directory(label, path)?;
        }
        Err(source) => {
            return Err(ProfilePathError::InspectPath {
                operation: label,
                source,
            });
        }
    }

    let canonical = canonicalize(label, path)?;
    if let Some(parent) = canonical_parent {
        if !canonical.starts_with(parent) {
            return Err(ProfilePathError::DirectoryEscapedTrustedRoot(label));
        }
    }
    Ok(())
}

fn reject_symlink_or_non_directory(
    label: &'static str,
    path: &Path,
) -> Result<(), ProfilePathError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ProfilePathError::InspectPath {
        operation: label,
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ProfilePathError::SymlinkNotAllowed(label));
    }
    if !metadata.is_dir() {
        return Err(ProfilePathError::NotDirectory(label));
    }
    Ok(())
}

fn canonicalize(label: &'static str, path: &Path) -> Result<PathBuf, ProfilePathError> {
    fs::canonicalize(path).map_err(|source| ProfilePathError::CanonicalizePath {
        operation: label,
        source,
    })
}

/// Failure while resolving or preparing desktop profile paths.
#[derive(Debug)]
pub enum ProfilePathError {
    /// The platform-provided root was not absolute.
    TrustedRootNotAbsolute,
    /// The platform-provided root contained lexical parent traversal.
    TrustedRootContainsTraversal,
    /// A profile path escaped its trusted parent after construction.
    ProfileEscapedTrustedRoot,
    /// Tauri could not resolve the application-local-data directory.
    TauriPathResolution(String),
    /// A required owned directory could not be created.
    CreateDirectory {
        operation: &'static str,
        source: std::io::Error,
    },
    /// An existing path could not be inspected safely.
    InspectPath {
        operation: &'static str,
        source: std::io::Error,
    },
    /// A required directory path was a symbolic link.
    SymlinkNotAllowed(&'static str),
    /// A required directory path existed but was not a directory.
    NotDirectory(&'static str),
    /// An owned directory could not be canonicalized.
    CanonicalizePath {
        operation: &'static str,
        source: std::io::Error,
    },
    /// A canonical owned directory escaped the expected parent.
    DirectoryEscapedTrustedRoot(&'static str),
}

impl fmt::Display for ProfilePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustedRootNotAbsolute => {
                formatter.write_str("application-local-data root is not absolute")
            }
            Self::TrustedRootContainsTraversal => formatter
                .write_str("application-local-data root contains a parent traversal component"),
            Self::ProfileEscapedTrustedRoot => {
                formatter.write_str("profile path escaped the trusted application data root")
            }
            Self::TauriPathResolution(message) => {
                write!(
                    formatter,
                    "could not resolve application-local-data path: {message}"
                )
            }
            Self::CreateDirectory { operation, source } => {
                write!(formatter, "{operation} failed: {source}")
            }
            Self::InspectPath { operation, source } => {
                write!(formatter, "could not inspect {operation}: {source}")
            }
            Self::SymlinkNotAllowed(operation) => {
                write!(formatter, "{operation} must not be a symbolic link")
            }
            Self::NotDirectory(operation) => {
                write!(formatter, "{operation} exists but is not a directory")
            }
            Self::CanonicalizePath { operation, source } => {
                write!(formatter, "could not canonicalize {operation}: {source}")
            }
            Self::DirectoryEscapedTrustedRoot(operation) => {
                write!(
                    formatter,
                    "{operation} escaped its trusted parent directory"
                )
            }
        }
    }
}

impl std::error::Error for ProfilePathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. }
            | Self::InspectPath { source, .. }
            | Self::CanonicalizePath { source, .. } => Some(source),
            Self::TrustedRootNotAbsolute
            | Self::TrustedRootContainsTraversal
            | Self::ProfileEscapedTrustedRoot
            | Self::TauriPathResolution(_)
            | Self::SymlinkNotAllowed(_)
            | Self::NotDirectory(_)
            | Self::DirectoryEscapedTrustedRoot(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopProfilePaths, ProfilePathError};
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
                "silent-disco-desktop-profile-test-{}-{sequence}",
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

    #[test]
    fn constructs_isolated_deterministic_paths() {
        let test_root = TestDirectory::new();
        let first_id = ProfileId::parse("main").expect("valid ID");
        let second_id = ProfileId::parse("lab_2").expect("valid ID");

        let first = DesktopProfilePaths::from_trusted_app_local_data_root(&test_root.0, &first_id)
            .expect("valid paths");
        let second =
            DesktopProfilePaths::from_trusted_app_local_data_root(&test_root.0, &second_id)
                .expect("valid paths");

        assert_eq!(first.root(), test_root.0.join("profiles/main"));
        assert_eq!(
            first.domain_database(),
            test_root.0.join("profiles/main/silent-disco.sqlite3")
        );
        assert_eq!(
            first.metadata(),
            test_root.0.join("profiles/main/profile.json")
        );
        assert_ne!(first.root(), second.root());
        assert!(first.p2_database().is_none());
    }

    #[test]
    fn rejects_relative_or_lexically_traversing_trusted_root() {
        let id = ProfileId::parse("main").expect("valid ID");
        assert!(matches!(
            DesktopProfilePaths::from_trusted_app_local_data_root(
                std::path::Path::new("relative"),
                &id
            ),
            Err(ProfilePathError::TrustedRootNotAbsolute)
        ));

        let traversing = std::env::temp_dir().join("safe/../other");
        assert!(matches!(
            DesktopProfilePaths::from_trusted_app_local_data_root(&traversing, &id),
            Err(ProfilePathError::TrustedRootContainsTraversal)
        ));
    }

    #[test]
    fn prepares_only_owned_directories_and_does_not_open_database() {
        let test_root = TestDirectory::new();
        let id = ProfileId::parse("main").expect("valid ID");
        let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&test_root.0, &id)
            .expect("valid paths");

        paths.prepare_directories().expect("prepare directories");

        assert!(paths.root().is_dir());
        assert!(paths.sources().is_dir());
        assert!(paths.diagnostics().is_dir());
        assert!(paths.cache().is_dir());
        assert!(!paths.domain_database().exists());
        assert!(!paths.metadata().exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_profile_root_symlink_escape() {
        use std::os::unix::fs::symlink;

        let test_root = TestDirectory::new();
        let outside = TestDirectory::new();
        let profiles = test_root.0.join("profiles");
        fs::create_dir_all(&profiles).expect("create profiles directory");
        symlink(&outside.0, profiles.join("main")).expect("create profile symlink");

        let id = ProfileId::parse("main").expect("valid ID");
        let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&test_root.0, &id)
            .expect("valid paths");

        assert!(matches!(
            paths.prepare_directories(),
            Err(ProfilePathError::SymlinkNotAllowed("profile root"))
        ));
        assert!(!outside.0.join("sources").exists());
    }
}
