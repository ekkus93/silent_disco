use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TestDatabasePath {
    path: PathBuf,
}

impl TestDatabasePath {
    pub(crate) fn new(label: &str) -> Self {
        let unique = NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-{label}-{}-{unique}.sqlite3",
            std::process::id()
        ));
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDatabasePath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(PathBuf::from(format!("{}-wal", self.path.display())));
        let _ = fs::remove_file(PathBuf::from(format!("{}-shm", self.path.display())));
    }
}
