//! Live revocation (remediation TODO §1, owner decision 2026-11-18): revoking a
//! remembered listener anywhere in the app also disconnects it from every
//! running host whose trust came from the same host key, instead of only
//! refusing it at its next join. The decision to disconnect is the core
//! actor's (`CoreCommand::ListenerRevoked`); this only routes the revoke from
//! the Settings screen, which has no handle to the running host, to it.

use std::sync::Mutex;

use silent_disco_core::domain::DeviceId;
use silent_disco_core::error::CoreErrorCode;
use silent_disco_core::runtime::{CoreActorHandle, CoreCommand, CoreCommandRequest};

use super::handle::FfiCoreHandle;
use super::types::FfiBridgeError;

/// Running hosts are few (one per app); the bound only stops a leak if
/// handles are dropped without shutting down.
const MAX_RUNNING_HOSTS: usize = 16;

struct RunningHost {
    /// The `FfiCoreHandle`'s address: a host that loads its trust again
    /// replaces its own entry instead of adding another.
    id: usize,
    host_public_key_der: Vec<u8>,
    actor: CoreActorHandle,
}

static RUNNING_HOSTS: Mutex<Vec<RunningHost>> = Mutex::new(Vec::new());

/// Called when a host loads its trust from the database.
pub(super) fn register(
    handle: &FfiCoreHandle,
    host_public_key_der: &[u8],
) -> Result<(), FfiBridgeError> {
    let actor = handle.actor_handle()?;
    let id = std::ptr::from_ref(handle) as usize;
    let mut hosts = RUNNING_HOSTS
        .lock()
        .map_err(|_| FfiBridgeError::Closed("running host registry is poisoned".to_owned()))?;
    hosts.retain(|host| host.id != id);
    if hosts.len() >= MAX_RUNNING_HOSTS {
        hosts.remove(0);
    }
    hosts.push(RunningHost {
        id,
        host_public_key_der: host_public_key_der.to_vec(),
        actor,
    });
    Ok(())
}

/// Tells every running host using `host_public_key_der` that `device_id` was
/// revoked. Hosts that have shut down are dropped. A host that is running but
/// cannot take the command is reported, so the revoke is not shown as fully
/// applied when the listener is still connected.
///
/// # Errors
/// Returns an error naming how many running hosts could not be told.
pub(crate) fn listener_revoked(
    host_public_key_der: &[u8],
    device_id: &DeviceId,
) -> Result<(), FfiBridgeError> {
    let mut hosts = RUNNING_HOSTS
        .lock()
        .map_err(|_| FfiBridgeError::Closed("running host registry is poisoned".to_owned()))?;
    let mut failures = Vec::new();
    hosts.retain(|host| {
        if host.host_public_key_der != host_public_key_der {
            return true;
        }
        let Ok(snapshot) = host.actor.current_snapshot() else {
            return false; // the host's actor is gone
        };
        let command = CoreCommand::ListenerRevoked {
            listener_id: device_id.clone(),
        };
        let outcome = CoreCommandRequest::new(snapshot.revision, command)
            .map_err(|error| error.to_string())
            .and_then(|request| {
                host.actor.submit_command(request).map_err(|error| {
                    if error.code == CoreErrorCode::ShutdownInProgress {
                        String::new()
                    } else {
                        error.message
                    }
                })
            });
        match outcome {
            Ok(_) => true,
            Err(message) if message.is_empty() => false, // shutting down
            Err(message) => {
                failures.push(message);
                true
            }
        }
    });
    if failures.is_empty() {
        Ok(())
    } else {
        Err(FfiBridgeError::Core(format!(
            "revoked, but {} running host session(s) could not disconnect the listener now; \
             it will be refused at its next join: {}",
            failures.len(),
            failures.join("; ")
        )))
    }
}
