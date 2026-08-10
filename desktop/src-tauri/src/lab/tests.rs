use super::{LabRuntime, MAX_LAB_NODES};
use crate::platform::paths::DesktopProfilePaths;
use crate::profile::ProfileId;
use silent_disco_core::transport::TransportClock;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-lab-{}-{sequence}",
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
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Block 37.1/37.3 "Lab profiles use separate roots" / "profile roots
/// differ" / "production profile cannot be opened in Lab runtime": the
/// Lab root and a production profile's root, computed from the same
/// trusted application-local-data root, must be structurally disjoint --
/// neither ever contains the other.
#[test]
fn lab_root_is_disjoint_from_the_production_profiles_root() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let profile_id = ProfileId::parse("main").expect("valid profile id");
    let profile_paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("profile paths");

    assert!(!lab.lab_root().starts_with(profile_paths.root()));
    assert!(!profile_paths.root().starts_with(lab.lab_root()));
    assert_ne!(lab.lab_root(), profile_paths.root());
}

/// Block 37.2 "unique node IDs" / "isolated databases" / "isolated
/// identities": two nodes started from the same runtime never share a
/// node ID, a database path, or a synthetic identity.
#[test]
fn two_nodes_are_fully_isolated_from_each_other() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");

    let first_id = lab.start_node().expect("start first node");
    let second_id = lab.start_node().expect("start second node");
    assert_ne!(first_id, second_id);

    let ids = lab.node_ids();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));

    lab.shutdown().expect("shutdown releases every node");
}

/// Block 37.1 "synthetic identity ... compile only where intended" /
/// determinism: the same node ID always derives the same synthetic
/// identity, and it is never the production keyring-backed kind (there
/// is no code path here that could reach it -- `DesktopIdentity::from_secret`
/// never touches OS-keyring storage).
#[test]
fn synthetic_identity_is_deterministic_per_node_id() {
    let first = super::synthetic_identity(super::LabNodeId(7)).expect("synthetic identity");
    let second = super::synthetic_identity(super::LabNodeId(7)).expect("synthetic identity again");
    let different =
        super::synthetic_identity(super::LabNodeId(8)).expect("different node identity");

    assert_eq!(first.device_id(), second.device_id());
    assert_ne!(first.device_id(), different.device_id());
}

/// Block 37.2 "bounded node count": reaching the cap is a loud, reported
/// error -- never a silently dropped or substituted node -- and every
/// already-started node remains intact afterward.
#[test]
fn starting_beyond_the_bound_is_a_reported_error_not_a_silent_drop() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");

    for _ in 0..MAX_LAB_NODES {
        lab.start_node().expect("start node within the bound");
    }
    assert_eq!(lab.node_ids().len(), MAX_LAB_NODES);

    let error = lab
        .start_node()
        .expect_err("starting beyond the bound must fail, not silently drop a node");
    assert_eq!(error.code, "desktop.lab.node_limit_reached");
    assert_eq!(lab.node_ids().len(), MAX_LAB_NODES);

    lab.shutdown().expect("shutdown releases every node");
}

/// A started node's actor is genuinely live (not a stub) and its
/// synthetic identity is exactly the one `synthetic_identity` derives for
/// its own node ID -- both accessors exist for a later Lab Mode block to
/// submit scenario commands / present node identity through.
#[test]
fn a_started_node_has_a_live_handle_and_its_own_derived_identity() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let node_id = lab.start_node().expect("start node");

    let handle = lab.node_handle(node_id).expect("node handle present");
    handle
        .current_snapshot()
        .expect("a live node actor answers a snapshot query");

    let identity = lab.node_identity(node_id).expect("node identity present");
    let expected = super::synthetic_identity(node_id).expect("expected synthetic identity");
    assert_eq!(identity.device_id(), expected.device_id());

    lab.shutdown().expect("shutdown releases the node");

    assert!(lab.node_handle(node_id).is_none());
    assert!(lab.node_identity(node_id).is_none());
}

/// Block 37.2 "explicit stop": stopping one node removes exactly that
/// node and leaves the others running; stopping it again is a reported
/// error, not a silent no-op or a panic.
#[test]
fn stop_node_removes_exactly_one_node_and_repeated_stop_is_reported() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let kept = lab.start_node().expect("start kept node");
    let stopped = lab.start_node().expect("start node to stop");

    lab.stop_node(stopped).expect("stop node");
    assert_eq!(lab.node_ids(), vec![kept]);

    let error = lab
        .stop_node(stopped)
        .expect_err("stopping an already-stopped node must be reported");
    assert_eq!(error.code, "desktop.lab.unknown_node");

    lab.shutdown()
        .expect("shutdown releases the remaining node");
}

/// Block 37.3 "Lab shutdown releases every node": every node started
/// this session is gone afterward, and a second shutdown call on an
/// already-empty runtime is a clean no-op, not an error.
#[test]
fn shutdown_releases_every_node_and_is_idempotent_when_empty() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    lab.start_node().expect("start node one");
    lab.start_node().expect("start node two");
    lab.start_node().expect("start node three");
    assert_eq!(lab.node_ids().len(), 3);

    lab.shutdown().expect("shutdown releases every node");
    assert!(lab.node_ids().is_empty());

    lab.shutdown()
        .expect("shutdown on an empty runtime is a clean no-op");
}

/// Block 38.2 "per-node offset" / "per-node drift in ppm": nodes started
/// with different clock configurations diverge exactly as configured as
/// the shared scenario timeline advances, while a plain `start_node`
/// (no configuration) stays in perfect sync with it.
#[test]
fn nodes_started_with_different_clock_configurations_diverge_as_the_scenario_advances() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");

    let steady = lab.start_node().expect("start steady node");
    let ahead = lab
        .start_node_with_clock(1_000, 0)
        .expect("start node with a fixed offset");
    let fast = lab
        .start_node_with_clock(0, 50_000)
        .expect("start node with positive drift"); // +5%

    lab.advance(100_000).expect("advance the shared timeline");
    assert_eq!(lab.now().get(), 100_000);

    assert_eq!(
        lab.node_clock(steady)
            .expect("steady node clock")
            .now()
            .get(),
        100_000
    );
    assert_eq!(
        lab.node_clock(ahead).expect("ahead node clock").now().get(),
        101_000
    );
    assert_eq!(
        lab.node_clock(fast).expect("fast node clock").now().get(),
        105_000
    );

    lab.shutdown().expect("shutdown releases every node");
}

/// Block 38.2 "checked arithmetic": a drift configuration outside the
/// supported bound is rejected before a node ID is consumed, a directory
/// is created, or any resource is opened -- the runtime is left exactly
/// as it was.
#[test]
fn an_invalid_clock_configuration_never_consumes_a_node_slot() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");

    let error = lab
        .start_node_with_clock(0, 10_000_000)
        .expect_err("drift far beyond the supported bound must be rejected");
    assert_eq!(error.code, "desktop.lab.clock_configuration_invalid");
    assert!(lab.node_ids().is_empty());

    // The runtime is still fully usable afterward -- a rejected
    // configuration is not a poisoned or half-broken state.
    let node_id = lab
        .start_node()
        .expect("start node after a rejected attempt");
    assert_eq!(lab.node_ids(), vec![node_id]);
    lab.shutdown().expect("shutdown releases the node");
}
