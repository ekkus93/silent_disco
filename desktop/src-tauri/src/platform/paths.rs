use crate::profile::{ProfileId, ProfileValidationError};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

/// Deterministic application-owned paths for one desktop profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopProfilePaths {
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
            domain_database: root.join("silent-disco.sqlite3"),
            p2_database: None,
            metadata: root.join("profile.json"),
            sources: root.join("sources"),
            diagnostics: root.join("diagnostics"),
            cache: root.join("cache"),
            root,
        })
    }

    /// Creates only the directories owned by this profile.
    ///
    /// This method does not create or open either database file.
    ///
    /// # Errors
    ///
    /// Returns [`ProfilePathError::CreateDirectory`] with the failed operation when
    /// any required directory cannot be created.
    pub fn prepare_directories(&self) -> Result<(), ProfilePathError> {
        for (operation, path) in [
            ("create profile root", self.root.as_path()),
            ("create source directory", self.sources.as_path()),
            ("create diagnostics directory", self.diagnostics.as_path()),
            ("create cache directory", self.cache.as_path()),
        ] {
            fs::create_dir_all(path).map_err(|source| ProfilePathError::CreateDirectory {
                operation,
                source,
            })?;
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
    /// A profile identifier failed validation before path construction.
    InvalidProfile(ProfileValidationError),
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
                write!(formatter, "could not resolve application-local-data path: {message}")
            }
            Self::CreateDirectory { operation, source } => {
                write!(formatter, "{operation} failed: {source}")
            }
            Self::InvalidProfile(error) => write!(formatter, "invalid profile: {error}"),
        }
    }
}

impl std::error::Error for ProfilePathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. } => Some(source),
            Self::InvalidProfile(source) => Some(source),
            Self::TrustedRootNotAbsolute
            | Self::TrustedRootContainsTraversal
            | Self::ProfileEscapedTrustedRoot
            | Self::TauriPathResolution(_) => None,
        }
    }
}

impl From<ProfileValidationError> for ProfilePathError {
    fn from(value: ProfileValidationError) -> Self {
        Self::InvalidProfile(value)
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

        let first = DesktopProfilePaths::from_trusted_app_local_data_root(
            &test_root.0,
            &first_id,
        )
        .expect("valid paths");
        let second = DesktopProfilePaths::from_trusted_app_local_data_root(
            &test_root.0,
            &second_id,
        )
        .expect("valid paths");

        assert_eq!(first.root(), test_root.0.join("profiles/main"));
        assert_eq!(
            first.domain_database(),
            test_root.0.join("profiles/main/silent-disco.sqlite3")
        );
        assert_eq!(first.metadata(), test_root.0.join("profiles/main/profile.json"));
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
}
