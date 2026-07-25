#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum FixtureError {
    InvalidRelativePath(String),
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRelativePath(path) => {
                write!(formatter, "fixture path must be a safe relative path: {path}")
            }
            Self::Read { path, source } => {
                write!(formatter, "failed to read fixture {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for FixtureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRelativePath(_) => None,
            Self::Read { source, .. } => Some(source),
        }
    }
}

pub fn fixture_path(root: &Path, relative: &Path) -> Result<PathBuf, FixtureError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(FixtureError::InvalidRelativePath(
            relative.display().to_string(),
        ));
    }

    Ok(root.join(relative))
}

pub fn read_fixture(root: &Path, relative: &Path) -> Result<Vec<u8>, FixtureError> {
    let path = fixture_path(root, relative)?;
    fs::read(&path).map_err(|source| FixtureError::Read { path, source })
}

#[cfg(test)]
mod tests {
    use super::fixture_path;
    use std::path::Path;

    #[test]
    fn joins_safe_relative_fixture_path() {
        let path = fixture_path(Path::new("testdata"), Path::new("protocol/hello.json"))
            .expect("safe path should be accepted");
        assert_eq!(path, Path::new("testdata/protocol/hello.json"));
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(fixture_path(Path::new("testdata"), Path::new("../secret")).is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(fixture_path(Path::new("testdata"), Path::new("/tmp/secret")).is_err());
    }
}
