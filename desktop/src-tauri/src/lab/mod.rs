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
//! This block only establishes the isolated core+storage+identity
//! scaffolding; virtual transport and clock wiring (spec section 29.2/
//! 29.3, already present in `silent_disco_core::transport`'s
//! `VirtualTransportFactory`/`ManualTransportClock`) are a later Lab
//! Mode block's concern.

use crate::dto::DesktopErrorDto;
use crate::platform::identity::DesktopIdentity;
use crate::platform::paths::{
    ProfilePathError, canonicalize, ensure_owned_directory, reject_symlink_or_non_directory,
    validate_trusted_root,
};
use sha2::{Digest, Sha256};
use silent_disco_core::runtime::{CoreActorConfig, CoreActorHandle, CoreActorRuntime};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
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
}

/// One isolated Lab node's owned resources.
pub(crate) struct LabNodeHandle {
    node_id: LabNodeId,
    identity: DesktopIdentity,
    actor: CoreActorRuntime,
    database: DatabaseWorker,
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
}

pub(crate) struct LabRuntime {
    lab_root: PathBuf,
    next_id: AtomicU32,
    nodes: Mutex<HashMap<LabNodeId, LabNodeHandle>>,
}

impl LabRuntime {
    /// Creates a Lab runtime rooted under a dedicated `lab` subtree of
    /// the trusted application-local-data root.
    ///
    /// # Errors
    ///
    /// Returns [`ProfilePathError`] when the trusted root is relative or
    /// contains a lexical parent traversal component.
    pub(crate) fn new(app_local_data_root: &Path) -> Result<Self, ProfilePathError> {
        validate_trusted_root(app_local_data_root)?;
        Ok(Self {
            lab_root: app_local_data_root.join("lab"),
            next_id: AtomicU32::new(1),
            nodes: Mutex::new(HashMap::new()),
        })
    }

    #[must_use]
    pub(crate) fn lab_root(&self) -> &Path {
        &self.lab_root
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

    /// Starts one new, fully isolated Lab node: its own directory, its
    /// own database file, and its own synthetic (never-keyring) identity
    /// (Block 37.2 "isolated databases", "isolated identities", "explicit
    /// start"; Block 37.3 "profile roots differ").
    ///
    /// # Errors
    ///
    /// Returns a structured error when the bounded node count is already
    /// reached, the node's directory cannot be prepared safely, its
    /// database cannot be opened, or its core actor fails to start.
    pub(crate) fn start_node(&self) -> Result<LabNodeId, DesktopErrorDto> {
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
            // Lab Mode has no scenario recorder yet (a later block's
            // concern) -- every node's own actor still runs the exact
            // same production state machine, it simply has no observer
            // side effects to report today.
            |_notification| Ok(()),
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
