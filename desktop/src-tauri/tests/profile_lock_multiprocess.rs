use silent_disco_desktop_lib::platform::paths::DesktopProfilePaths;
use silent_disco_desktop_lib::platform::profile_lock::{ProfileLease, ProfileLockError};
use silent_disco_desktop_lib::profile::ProfileId;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const CHILD_MODE_ENV: &str = "SILENT_DISCO_PROFILE_LOCK_CHILD_MODE";
const CHILD_ROOT_ENV: &str = "SILENT_DISCO_PROFILE_LOCK_CHILD_ROOT";
const CHILD_READY_ENV: &str = "SILENT_DISCO_PROFILE_LOCK_CHILD_READY";
const CHILD_RELEASE_ENV: &str = "SILENT_DISCO_PROFILE_LOCK_CHILD_RELEASE";
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-profile-lock-{label}-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale profile-lock test directory");
        }
        fs::create_dir_all(&path).expect("create profile-lock test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound && !thread::panicking() {
                panic!("failed to remove profile-lock test directory: {error}");
            }
        }
    }
}

#[test]
fn child_lock_holder() {
    let Some(mode) = std::env::var_os(CHILD_MODE_ENV) else {
        return;
    };
    let root = PathBuf::from(
        std::env::var_os(CHILD_ROOT_ENV).expect("child profile root environment variable"),
    );
    let ready = PathBuf::from(
        std::env::var_os(CHILD_READY_ENV).expect("child ready environment variable"),
    );
    let release = PathBuf::from(
        std::env::var_os(CHILD_RELEASE_ENV).expect("child release environment variable"),
    );
    let profile_id = ProfileId::parse("main").expect("valid child profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root, &profile_id)
        .expect("valid child profile paths");
    let lease = ProfileLease::acquire(&paths, &profile_id).expect("child acquires profile lease");
    fs::write(&ready, b"ready").expect("child writes ready marker");

    match mode.to_str().expect("UTF-8 child mode") {
        "release" => {
            wait_for_path(&release, WAIT_TIMEOUT).expect("child observes release marker");
            lease.release().expect("child releases profile lease");
        }
        "kill" => loop {
            thread::sleep(Duration::from_secs(1));
        },
        other => panic!("unknown child mode: {other}"),
    }
}

#[test]
fn another_process_is_rejected_until_normal_release() {
    let root = TestDirectory::new("normal-release");
    let ready = root.0.join("child.ready");
    let release = root.0.join("child.release");
    let mut child = spawn_child("release", &root.0, &ready, &release);
    wait_for_child_ready(&mut child, &ready).expect("child acquires profile lease");

    let profile_id = ProfileId::parse("main").expect("valid parent profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid parent profile paths");
    assert!(matches!(
        ProfileLease::acquire(&paths, &profile_id),
        Err(ProfileLockError::ProfileInUse)
    ));

    fs::write(&release, b"release").expect("write child release marker");
    let status = wait_for_child_exit(&mut child, WAIT_TIMEOUT).expect("child exits after release");
    assert!(status.success(), "child failed with status {status}");

    ProfileLease::acquire(&paths, &profile_id)
        .expect("parent acquires after normal child release")
        .release()
        .expect("parent releases profile lease");
}

#[test]
fn operating_system_releases_lock_after_child_termination() {
    let root = TestDirectory::new("abnormal-termination");
    let ready = root.0.join("child.ready");
    let release = root.0.join("unused.release");
    let mut child = spawn_child("kill", &root.0, &ready, &release);
    wait_for_child_ready(&mut child, &ready).expect("child acquires profile lease");

    let profile_id = ProfileId::parse("main").expect("valid parent profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid parent profile paths");
    assert!(matches!(
        ProfileLease::acquire(&paths, &profile_id),
        Err(ProfileLockError::ProfileInUse)
    ));

    child.kill().expect("terminate child process");
    let _status = wait_for_child_exit(&mut child, WAIT_TIMEOUT).expect("terminated child exits");

    ProfileLease::acquire(&paths, &profile_id)
        .expect("parent acquires after child termination")
        .release()
        .expect("parent releases profile lease");
}

fn spawn_child(mode: &str, root: &Path, ready: &Path, release: &Path) -> Child {
    Command::new(std::env::current_exe().expect("resolve integration test executable"))
        .arg("--exact")
        .arg("child_lock_holder")
        .arg("--nocapture")
        .env(CHILD_MODE_ENV, mode)
        .env(CHILD_ROOT_ENV, root)
        .env(CHILD_READY_ENV, ready)
        .env(CHILD_RELEASE_ENV, release)
        .spawn()
        .expect("spawn profile-lock child process")
}

fn wait_for_child_ready(child: &mut Child, ready: &Path) -> Result<(), String> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if ready.is_file() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not inspect child status: {error}"))?
        {
            return Err(format!(
                "child exited before acquiring the profile lease: {status}"
            ));
        }
        if Instant::now() >= deadline {
            let kill_error = child.kill().err();
            let wait_error = child.wait().err();
            return Err(format!(
                "child did not acquire the profile lease before timeout; kill={kill_error:?}, wait={wait_error:?}"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not inspect child status: {error}"))?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let kill_error = child.kill().err();
            let wait_result = child.wait();
            return Err(format!(
                "child did not exit before timeout; kill={kill_error:?}, wait={wait_result:?}"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_path(path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if path.is_file() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("path did not appear before timeout: {}", path.display()));
        }
        thread::sleep(POLL_INTERVAL);
    }
}
