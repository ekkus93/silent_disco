//! `close_profile_sync`, the shared close body driving both the Tauri
//! `close_profile` command (`mod.rs`, via `spawn_blocking`) and
//! `app_shutdown.rs`'s window-close-triggered shutdown thread, plus its
//! `merge_close_results` helper.
//!
//! The four `#[tauri::command]`-annotated entry points
//! (`open_profile`/`get_current_snapshot`/`attach_notifications`/
//! `close_profile`) live directly in `mod.rs`, not here: Tauri's command
//! macro generates hidden companion items (e.g. `__cmd__open_profile`)
//! alongside each function that `generate_handler!` looks up at the exact
//! path named in `lib.rs` (`app_state::open_profile`) -- a `pub use
//! commands::open_profile;` re-export only re-exports the function itself,
//! not those hidden macro companions, so `generate_handler!` would fail to
//! find them. Keeping the annotated functions defined directly where
//! `lib.rs` names them avoids that without resorting to a wildcard
//! `pub use commands::*;` re-export.

use super::DesktopAppState;
use super::state::CloseAction;
use crate::dto::DesktopErrorDto;
use crate::platform::file_picker::SelectedSourceRegistry;
use crate::platform::source_staging_control::SourceStagingControl;
use crate::shutdown::shutdown_owned_resources;
use tauri::{AppHandle, Manager};

/// Synchronous, `AppHandle`-driven equivalent of [`super::close_profile`]'s
/// body -- shared by that command (via `spawn_blocking`, since it must not
/// block the async runtime) and by the window-close-triggered whole-
/// application shutdown (`app_shutdown.rs`, Block 36.3), which is already
/// running on its own dedicated thread and calls this directly. Keeping
/// one shared function means the two paths can never drift apart.
///
/// A duplicate call while another close is already in flight is
/// idempotent success, not an error (Block 36.3 "duplicate close is
/// idempotent").
///
/// # Errors
///
/// Returns a structured error for lifecycle, worker, actor, database, or
/// profile-lock cleanup failure.
pub(crate) fn close_profile_sync(app: &AppHandle) -> Result<(), DesktopErrorDto> {
    let state = app.state::<DesktopAppState>();
    let action = state.take_for_close()?;
    let ready = match action {
        // Nothing new to tear down: either already closed, or another
        // close already owns the real teardown -- this call must not
        // attempt a second teardown or touch staging/registry state the
        // original owns.
        CloseAction::AlreadyClosed | CloseAction::AlreadyInProgress => None,
        CloseAction::Shutdown(ready) => Some(ready),
    };
    let Some(ready) = ready else {
        return Ok(());
    };
    let staging_cleanup = app.state::<SourceStagingControl>().cancel_and_wait();
    let result = shutdown_owned_resources(ready.owned);
    let registry_cleanup = app.state::<SelectedSourceRegistry>().clear().map(|_| ());
    let result = merge_close_results(result, staging_cleanup);
    state.finish_close(merge_close_results(result, registry_cleanup))
}

fn merge_close_results(
    primary: Result<(), DesktopErrorDto>,
    registry_cleanup: Result<(), DesktopErrorDto>,
) -> Result<(), DesktopErrorDto> {
    match (primary, registry_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(primary.with_appended_cleanup(Some(cleanup))),
    }
}
