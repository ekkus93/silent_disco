//! Runtime state-machine types shared across the `app_state` module tree.
//!
//! [`DesktopRuntimeState`] is the single source of truth for
//! `DesktopAppState`'s lifecycle; [`ReadyRuntime`] holds every handle the
//! open profile owns; [`CloseAction`] is the result `take_for_close` hands
//! back to its two callers (the Tauri `close_profile` command and the
//! shutdown-thread path) so they can decide what, if anything, to tear down.

use crate::dto::DesktopErrorDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::identity::DesktopIdentity;
use crate::platform::invitation_identity::DesktopHostSigningIdentity;
use crate::platform::network::DesktopHostNetworkControl;
use crate::profile::ProfileId;
use crate::shutdown::DesktopOwnedResources;
use silent_disco_core::runtime::CoreActorHandle;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) enum DesktopRuntimeState {
    Closed,
    Opening {
        profile_id: ProfileId,
    },
    Ready(Box<ReadyRuntime>),
    Closing,
    /// A profile failed to open. Retryable per the stored error's own
    /// `retryable` field -- a new open attempt after `close`/reset is
    /// generally safe, since nothing was ever fully constructed.
    Failed(DesktopErrorDto),
    /// A shutdown was attempted and did not complete cleanly (Block 36.1
    /// "shutdown failed") -- distinct from [`Self::Failed`], which means a
    /// profile never became ready. Deliberately never treated as
    /// reopen-safe (see `begin_open`): on a genuine timeout, owned
    /// resources may still be alive on a detached background thread (Block
    /// 36.2/36.3 "timeout does not free callback-visible memory unsafely"),
    /// so a fresh open could race a still-tearing-down profile. Recovery
    /// requires restarting the application.
    ShutdownFailed(DesktopErrorDto),
}

pub(super) struct ReadyRuntime {
    pub(super) profile_id: ProfileId,
    pub(super) sources: PathBuf,
    pub(super) identity: DesktopIdentity,
    pub(super) signing_identity: DesktopHostSigningIdentity,
    pub(super) handle: CoreActorHandle,
    pub(super) notifications: Arc<DesktopNotificationBuffer>,
    pub(super) network: Arc<DesktopHostNetworkControl>,
    pub(super) owned: DesktopOwnedResources,
}

pub(super) enum CloseAction {
    AlreadyClosed,
    /// Another close is already tearing this profile down; this caller
    /// does not own that teardown and must not attempt a second one (Block
    /// 36.3 "duplicate close is idempotent").
    AlreadyInProgress,
    Shutdown(Box<ReadyRuntime>),
}
