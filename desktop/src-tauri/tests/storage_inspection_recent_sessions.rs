use silent_disco_core::domain::{AppRole, SessionId};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker, SessionStart};
use silent_disco_desktop_lib::platform::paths::DesktopProfilePaths;
use silent_disco_desktop_lib::platform::storage_inspection::inspect_profile_storage;
use silent_disco_desktop_lib::profile::ProfileId;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-storage-inspection-history-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale storage inspection history directory");
        }
        fs::create_dir_all(&path).expect("create storage inspection history directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove storage inspection history directory: {error}"
            );
        }
    }
}

#[test]
fn desktop_inspection_surfaces_recent_sessions_from_the_real_rust_worker() {
    let root = TestDirectory::new();
    let profile_id = ProfileId::parse("main").expect("valid profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid profile paths");
    paths.prepare_directories().expect("prepare profile paths");

    let worker = DatabaseWorker::start(
        DatabaseConfig::new(paths.domain_database()).expect("valid database configuration"),
    )
    .expect("start database worker");
    worker
        .client()
        .begin_session(&SessionStart {
            session_id: SessionId::new("session-storage-inspection")
                .expect("valid session identifier"),
            role: AppRole::Host,
            session_name: "Storage inspection fixture".to_owned(),
            started_at_ms: 1_000,
        })
        .expect("persist session history fixture");
    worker.stop_and_join().expect("stop database worker");

    let inspection = inspect_profile_storage(&paths, &profile_id).expect("inspect profile storage");
    assert_eq!(inspection.recent_sessions.len(), 1);
    let session = &inspection.recent_sessions[0];
    assert_eq!(session.session_id.as_str(), "session-storage-inspection");
    assert_eq!(session.role, AppRole::Host);
    assert_eq!(session.session_name, "Storage inspection fixture");
    assert_eq!(session.started_at_ms, 1_000);
    assert!(!inspection.p2_store_applicable);
}
