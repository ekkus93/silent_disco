//! Lab Mode's separate multi-node runtime (Block 37.2). Compiled only
//! when the `lab-mode` Cargo feature is enabled -- never linked into a
//! production build (Block 37.1 "synthetic identity and virtual adapters
//! compile only where intended").
//!
//! [`LabRuntime`] owns zero or more independent Lab nodes, each an
//! isolated `(CoreActorRuntime, DatabaseWorker)` pair with its own
//! synthetic, never-keyring identity and its own database file under a
//! dedicated `lab/` root -- structurally disjoint from
//! [`crate::platform::paths::DesktopProfilePaths`]'s `profiles/` root, so
//! a Lab node can never resolve to, collide with, or overwrite a
//! production profile (spec section 29.1: "must not inject test behavior
//! into a production core instance"; Block 37.1 "Lab profiles use
//! separate roots" / "production profile cannot be opened in Lab
//! runtime"). `LabRuntime` never references [`crate::app_state::DesktopAppState`]
//! or any other production singleton (Block 37.2 "no global production
//! singleton reuse").
//!
//! This block establishes the isolated core+storage+identity scaffolding
//! plus a deterministic virtual clock per node (Block 38, see
//! [`clock`]); virtual transport/fault-injection wiring (spec section
//! 29.3, already present in `silent_disco_core::transport`'s
//! `VirtualTransportFactory`) remains a later Lab Mode block's concern --
//! see [`scenario`]'s own module doc comment for exactly what Block 40
//! (scenario schema/runner/assertions, layered on top of this runtime) does
//! and does not wire up yet.

pub(crate) mod clock;
mod fault;
pub(crate) mod recorder;
pub(crate) mod recording;
pub(crate) mod replay;
pub(crate) mod scenario;

use crate::dto::DesktopErrorDto;
use crate::platform::identity::DesktopIdentity;
use crate::platform::paths::{
    ProfilePathError, canonicalize, ensure_owned_directory, reject_symlink_or_non_directory,
    validate_trusted_root,
};
use clock::{LabClock, LabNodeClock};
use sha2::{Digest, Sha256};
use silent_disco_core::domain::MonotonicMillis;
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreObserver,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// Hard cap on concurrently active Lab nodes (Block 37.2 "bounded node
/// count"). Generous for local desktop testing scenarios; reaching it is
/// a loud, reported error -- a new node is never silently dropped or
/// substituted for an existing one.
pub(crate) const MAX_LAB_NODES: usize = 16;

/// Identifies one Lab node, unique within a [`LabRuntime`] instance
/// (Block 37.2 "unique node IDs"). Backend-generated only -- never parsed
/// from frontend input, so it needs none of [`crate::profile::ProfileId`]'s
/// untrusted-text validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct LabNodeId(u32);

impl LabNodeId {
    fn directory_name(self) -> String {
        format!("lab-node-{:04}", self.0)
    }

    /// Exposes the node's opaque numeric identity for `crate::lab_commands`
    /// (Block 42) to encode as an IPC-safe decimal string -- the inverse of
    /// [`Self::from_u32`]. Never parsed from frontend input directly; a
    /// command always round-trips a value this module itself produced.
    #[must_use]
    pub(crate) fn as_u32(self) -> u32 {
        self.0
    }

    /// The inverse of [`Self::as_u32`], for `crate::lab_commands` to decode
    /// a node ID a Tauri command received back from the frontend. A
    /// decoded value that no longer names a live node is reported as
    /// [`LabRuntime`]'s own `unknown_node_error`, not treated specially
    /// here.
    #[must_use]
    pub(crate) fn from_u32(value: u32) -> Self {
        Self(value)
    }
}

/// One isolated Lab node's owned resources.
pub(crate) struct LabNodeHandle {
    node_id: LabNodeId,
    identity: DesktopIdentity,
    actor: CoreActorRuntime,
    database: DatabaseWorker,
    clock: Arc<LabNodeClock>,
}

impl LabNodeHandle {
    #[must_use]
    pub(crate) fn node_id(&self) -> LabNodeId {
        self.node_id
    }

    #[must_use]
    pub(crate) fn handle(&self) -> CoreActorHandle {
        self.actor.handle()
    }

    #[must_use]
    pub(crate) fn identity(&self) -> &DesktopIdentity {
        &self.identity
    }

    #[must_use]
    pub(crate) fn clock(&self) -> Arc<LabNodeClock> {
        Arc::clone(&self.clock)
    }
}

pub(crate) struct LabRuntime {
    lab_root: PathBuf,
    next_id: AtomicU32,
    nodes: Mutex<HashMap<LabNodeId, LabNodeHandle>>,
    /// The whole scenario's one shared virtual timeline (Block 38.2).
    /// Every node's own [`LabNodeClock`] is an offset/drift view over
    /// this same clock, so advancing it is the only way virtual time
    /// ever moves for any node in this runtime.
    clock: Arc<LabClock>,
}

impl LabRuntime {
    /// Creates a Lab runtime rooted under a dedicated `lab` subtree of
    /// the trusted application-local-data root, with its virtual
    /// timeline starting at `initial_clock_ms` (Block 38.2 "deterministic
    /// initial time").
    ///
    /// # Errors
    ///
    /// Returns [`ProfilePathError`] when the trusted root is relative or
    /// contains a lexical parent traversal component.
    pub(crate) fn new(
        app_local_data_root: &Path,
        initial_clock_ms: u64,
    ) -> Result<Self, ProfilePathError> {
        validate_trusted_root(app_local_data_root)?;
        Ok(Self {
            lab_root: app_local_data_root.join("lab"),
            next_id: AtomicU32::new(1),
            nodes: Mutex::new(HashMap::new()),
            clock: Arc::new(LabClock::new(initial_clock_ms)),
        })
    }

    #[must_use]
    pub(crate) fn lab_root(&self) -> &Path {
        &self.lab_root
    }

    /// Returns the scenario's current shared virtual time (Block 38.2).
    #[must_use]
    pub(crate) fn now(&self) -> MonotonicMillis {
        self.clock.now()
    }

    /// Moves the scenario's shared virtual timeline forward by exactly
    /// `delta_ms`, running any due scheduled wakeups in deterministic
    /// order (Block 38.2 "manual advance"; Block 38.3 "scheduler event
    /// order at equal timestamps"). Never sleeps.
    ///
    /// # Errors
    ///
    /// Returns a structured error when `delta_ms` would overflow the
    /// current virtual time; time is left completely unchanged.
    pub(crate) fn advance(&self, delta_ms: u64) -> Result<MonotonicMillis, DesktopErrorDto> {
        self.clock.advance(delta_ms).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.lab.clock_advance_failed",
                "runtime",
                "error",
                false,
                &error.to_string(),
            )
        })
    }

    #[must_use]
    pub(crate) fn node_ids(&self) -> Vec<LabNodeId> {
        self.nodes
            .lock()
            .map(|nodes| nodes.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Returns a live, cloneable handle to one node's core actor, for a
    /// later Lab Mode block to submit scenario commands through -- `None`
    /// once the node has been stopped or never existed.
    #[must_use]
    pub(crate) fn node_handle(&self, node_id: LabNodeId) -> Option<CoreActorHandle> {
        self.nodes
            .lock()
            .ok()
            .and_then(|nodes| nodes.get(&node_id).map(LabNodeHandle::handle))
    }

    /// Returns one node's synthetic identity, for a later Lab Mode block's
    /// scenario/diagnostics presentation -- `None` once the node has been
    /// stopped or never existed.
    #[must_use]
    pub(crate) fn node_identity(&self, node_id: LabNodeId) -> Option<DesktopIdentity> {
        self.nodes
            .lock()
            .ok()
            .and_then(|nodes| nodes.get(&node_id).map(|handle| handle.identity().clone()))
    }

    /// Returns one node's own clock view -- a per-node offset/drift
    /// window over the scenario's shared timeline (Block 38.2 "per-node
    /// offset", "per-node drift in ppm") -- for a later Lab Mode block's
    /// virtual transport wiring to time-stamp that node's events with.
    /// `None` once the node has been stopped or never existed.
    #[must_use]
    pub(crate) fn node_clock(&self, node_id: LabNodeId) -> Option<Arc<LabNodeClock>> {
        self.nodes
            .lock()
            .ok()
            .and_then(|nodes| nodes.get(&node_id).map(LabNodeHandle::clock))
    }

    /// Starts one new, fully isolated Lab node with no clock offset or
    /// drift (in sync with the shared scenario timeline) -- the common
    /// case. See [`Self::start_node_with_clock`] for a node that should
    /// run ahead/behind or drift relative to the others.
    ///
    /// # Errors
    ///
    /// See [`Self::start_node_with_clock`].
    pub(crate) fn start_node(&self) -> Result<LabNodeId, DesktopErrorDto> {
        self.start_node_with_clock(0, 0)
    }

    /// Starts one new, fully isolated Lab node: its own directory, its
    /// own database file, its own synthetic (never-keyring) identity, and
    /// its own clock view offset by `offset_ms` and drifting by
    /// `drift_ppm` relative to the shared scenario timeline (Block 37.2
    /// "isolated databases", "isolated identities", "explicit start";
    /// Block 37.3 "profile roots differ"; Block 38.2 "per-node offset",
    /// "per-node drift in ppm"). Its actor has no observer beyond the
    /// production no-op default -- see [`Self::start_node_with_clock_and_observer`]
    /// for a node whose notifications a caller (the Block 40 scenario
    /// runner) needs to observe.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the bounded node count is already
    /// reached, `drift_ppm` is out of the supported bound, the node's
    /// directory cannot be prepared safely, its database cannot be
    /// opened, or its core actor fails to start.
    pub(crate) fn start_node_with_clock(
        &self,
        offset_ms: i64,
        drift_ppm: i64,
    ) -> Result<LabNodeId, DesktopErrorDto> {
        self.start_node_with_clock_and_observer(offset_ms, drift_ppm, |_notification| Ok(()))
    }

    /// As [`Self::start_node_with_clock`], but with a caller-supplied
    /// [`CoreObserver`] instead of the production no-op default -- used by
    /// the Block 40 scenario runner (`super::scenario::run_scenario`) to
    /// attach a [`super::recorder::ScenarioRecorder`] to every node it
    /// starts, since a rejected command or an emitted diagnostic is
    /// otherwise invisible outside the observer stream (see
    /// `super::scenario`'s own module doc comment, "Step settlement").
    ///
    /// # Errors
    ///
    /// See [`Self::start_node_with_clock`].
    pub(crate) fn start_node_with_clock_and_observer<O: CoreObserver>(
        &self,
        offset_ms: i64,
        drift_ppm: i64,
        observer: O,
    ) -> Result<LabNodeId, DesktopErrorDto> {
        let mut nodes = self.nodes.lock().map_err(|_| lab_poisoned_error())?;
        if nodes.len() >= MAX_LAB_NODES {
            return Err(DesktopErrorDto::new(
                "desktop.lab.node_limit_reached",
                "runtime",
                "error",
                false,
                &format!("Lab Mode is bounded to {MAX_LAB_NODES} concurrent nodes"),
            ));
        }
        // Validated before allocating a node ID or touching the
        // filesystem -- an out-of-bounds clock configuration never
        // consumes a node slot or leaves a half-created node behind.
        let clock = Arc::new(
            LabNodeClock::new(Arc::clone(&self.clock), offset_ms, drift_ppm).map_err(|error| {
                DesktopErrorDto::new(
                    "desktop.lab.clock_configuration_invalid",
                    "runtime",
                    "error",
                    false,
                    &error.to_string(),
                )
            })?,
        );

        let node_id = LabNodeId(self.next_id.fetch_add(1, Ordering::AcqRel));
        let node_root = self.lab_root.join(node_id.directory_name());
        prepare_node_directory(&self.lab_root, &node_root)
            .map_err(|error| lab_path_error(node_id, &error))?;

        let identity = synthetic_identity(node_id).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.lab.identity_failed",
                "platform",
                "fatal",
                false,
                &format!(
                    "Lab node {} synthetic identity failed: {error}",
                    node_id.directory_name()
                ),
            )
        })?;

        let database_config =
            DatabaseConfig::new(node_root.join("lab.sqlite3")).map_err(|error| {
                DesktopErrorDto::new(
                    "desktop.lab.storage_configure_failed",
                    "storage",
                    "fatal",
                    false,
                    &error.to_string(),
                )
            })?;
        let database = DatabaseWorker::start(database_config).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.lab.storage_open_failed",
                "storage",
                "fatal",
                false,
                &error.to_string(),
            )
        })?;

        let actor = match CoreActorRuntime::start(
            CoreActorConfig::new(identity.device_id().clone()),
            observer,
        ) {
            Ok(actor) => actor,
            Err(error) => {
                // The database was already opened -- attempted even
                // though this node never gets inserted, so nothing is
                // leaked on a failed start.
                let _ = database.stop_and_join();
                return Err(DesktopErrorDto::from(error));
            }
        };

        nodes.insert(
            node_id,
            LabNodeHandle {
                node_id,
                identity,
                actor,
                database,
                clock,
            },
        );
        Ok(node_id)
    }

    /// Stops and joins exactly one node (Block 37.2 "explicit stop").
    ///
    /// # Errors
    ///
    /// Returns a structured error when the node does not exist or its
    /// teardown fails.
    pub(crate) fn stop_node(&self, node_id: LabNodeId) -> Result<(), DesktopErrorDto> {
        let handle = {
            let mut nodes = self.nodes.lock().map_err(|_| lab_poisoned_error())?;
            nodes
                .remove(&node_id)
                .ok_or_else(|| unknown_node_error(node_id))?
        };
        shutdown_node(handle)
    }

    /// Stops and joins every remaining node (Block 37.2 "explicit join";
    /// Block 37.3 "Lab shutdown releases every node"). Every node's
    /// teardown is attempted even when an earlier one fails.
    ///
    /// # Errors
    ///
    /// Returns one bounded structured error naming every node that
    /// failed to tear down cleanly.
    pub(crate) fn shutdown(&self) -> Result<(), DesktopErrorDto> {
        let handles: Vec<LabNodeHandle> = {
            let mut nodes = self.nodes.lock().map_err(|_| lab_poisoned_error())?;
            nodes.drain().map(|(_, handle)| handle).collect()
        };
        let mut failed_nodes = Vec::new();
        for handle in handles {
            let node_id = handle.node_id();
            if let Err(error) = shutdown_node(handle) {
                failed_nodes.push((node_id, error));
            }
        }
        if failed_nodes.is_empty() {
            return Ok(());
        }
        let mut message = String::from("Lab runtime shutdown failed for node(s): ");
        for (index, (node_id, error)) in failed_nodes.iter().enumerate() {
            if index > 0 {
                message.push_str(", ");
            }
            let _ = write!(message, "{} ({})", node_id.directory_name(), error.message);
        }
        Err(DesktopErrorDto::new(
            "desktop.lab.shutdown_failed",
            "runtime",
            "fatal",
            false,
            &message,
        ))
    }
}

fn shutdown_node(handle: LabNodeHandle) -> Result<(), DesktopErrorDto> {
    let actor_result = handle.actor.shutdown();
    let database_result = handle.database.stop_and_join();
    match (actor_result, database_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(actor_error), _) => Err(DesktopErrorDto::from(actor_error)),
        (Ok(()), Err(database_error)) => Err(DesktopErrorDto::new(
            "desktop.lab.storage_close_failed",
            "storage",
            "fatal",
            false,
            &database_error.to_string(),
        )),
    }
}

/// Deterministically derives a synthetic, never-keyring identity from a
/// Lab node's own ID (Block 37.1 "synthetic identity"). Reproducible
/// across runs of the same node -- consistent with Lab Mode's general
/// determinism requirement (spec section 29.2) -- and structurally
/// incapable of reading or writing OS-keyring-backed production identity
/// material, since [`DesktopIdentity::from_secret`] never touches it.
fn synthetic_identity(
    node_id: LabNodeId,
) -> Result<DesktopIdentity, crate::platform::identity::DesktopIdentityError> {
    let mut hasher = Sha256::new();
    hasher.update(b"silent-disco-lab-synthetic-identity");
    hasher.update([0]);
    hasher.update(node_id.directory_name().as_bytes());
    let secret: [u8; 32] = hasher.finalize().into();
    DesktopIdentity::from_secret(&secret)
}

fn prepare_node_directory(lab_root: &Path, node_root: &Path) -> Result<(), ProfilePathError> {
    fs::create_dir_all(lab_root).map_err(|source| ProfilePathError::CreateDirectory {
        operation: "lab root",
        source,
    })?;
    reject_symlink_or_non_directory("lab root", lab_root)?;
    let canonical_lab_root = canonicalize("lab root", lab_root)?;
    ensure_owned_directory("lab node root", node_root, Some(&canonical_lab_root))
}

fn lab_poisoned_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.state_poisoned",
        "runtime",
        "fatal",
        false,
        "the Lab runtime's node registry mutex was poisoned",
    )
}

fn unknown_node_error(node_id: LabNodeId) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.unknown_node",
        "runtime",
        "error",
        false,
        &format!(
            "Lab node {} does not exist or was already stopped",
            node_id.directory_name()
        ),
    )
}

fn lab_path_error(node_id: LabNodeId, error: &ProfilePathError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.directory_failed",
        "storage",
        "fatal",
        false,
        &format!(
            "Lab node {} directory could not be prepared: {error}",
            node_id.directory_name()
        ),
    )
}

#[cfg(test)]
mod tests;
