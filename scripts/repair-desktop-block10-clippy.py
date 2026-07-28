#!/usr/bin/env python3
"""Apply the second audited desktop Block 10 strict-Clippy repair pass."""

from pathlib import Path
import re


def replace_once(path: str, pattern: str, replacement: str, marker: str) -> None:
    target = Path(path)
    source = target.read_text()
    updated, count = re.subn(
        pattern,
        replacement,
        source,
        count=1,
        flags=re.MULTILINE | re.DOTALL,
    )
    if count == 1:
        target.write_text(updated)
        return
    if marker in source:
        return
    raise SystemExit(f"expected source was not found in {path}: {marker}")


def replace_exact(
    path: str,
    old: str,
    new: str,
    expected_count: int,
    marker: str,
) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count == expected_count:
        target.write_text(source.replace(old, new))
        return
    if marker in source:
        return
    raise SystemExit(
        f"expected {expected_count} occurrence(s) in {path}, found {count}: {marker}"
    )


app_state = "desktop/src-tauri/src/app_state.rs"

replace_once(
    app_state,
    r'''\) -> Result<OpenProfileResponse, \(DesktopErrorDto, ReadyRuntime\)> \{\s*let mut state = match self\.runtime\.lock\(\) \{\s*Ok\(state\) => state,\s*Err\(_\) => return Err\(\(poisoned_state_error\(\), ready\)\),\s*\};''',
    ''') -> Result<OpenProfileResponse, Box<(DesktopErrorDto, ReadyRuntime)>> {
        let Ok(mut state) = self.runtime.lock() else {
            return Err(Box::new((poisoned_state_error(), ready)));
        };''',
    "Box<(DesktopErrorDto, ReadyRuntime)>",
)
replace_once(
    app_state,
    r'''_ => Err\(\(\s*DesktopErrorDto::new\(\s*"desktop\.profile\.state_changed",\s*"runtime",\s*"fatal",\s*false,\s*"desktop profile lifecycle changed during startup",\s*\),\s*ready,\s*\)\),''',
    '''_ => Err(Box::new((
                DesktopErrorDto::new(
                    "desktop.profile.state_changed",
                    "runtime",
                    "fatal",
                    false,
                    "desktop profile lifecycle changed during startup",
                ),
                ready,
            ))),''',
    "Err(Box::new((",
)
replace_exact(
    app_state,
    "Err((primary, ready)) => {",
    "Err(boxed) => {\n                let (primary, ready) = *boxed;",
    2,
    "let (primary, ready) = *boxed;",
)
replace_once(
    app_state,
    r'''DesktopRuntimeState::Closed => \{\s*\*state = DesktopRuntimeState::Closed;\s*Ok\(CloseAction::AlreadyClosed\)\s*\}\s*DesktopRuntimeState::Failed\(_\) => \{\s*\*state = DesktopRuntimeState::Closed;\s*Ok\(CloseAction::AlreadyClosed\)\s*\}''',
    '''DesktopRuntimeState::Closed | DesktopRuntimeState::Failed(_) => {
                *state = DesktopRuntimeState::Closed;
                Ok(CloseAction::AlreadyClosed)
            }''',
    "DesktopRuntimeState::Closed | DesktopRuntimeState::Failed(_)",
)

replace_exact(
    app_state,
    "#[tauri::command]\npub async fn open_profile(",
    '''/// Opens one production profile after lock, identity, storage, actor, and bridge startup.
///
/// # Errors
///
/// Returns a structured desktop error for invalid profile IDs, path or lock failures,
/// unavailable secure identity, storage or actor startup failure, bridge failure, or
/// lifecycle races. Partial startup is cleaned up before the error is returned.
#[tauri::command]
pub async fn open_profile(''',
    1,
    "Opens one production profile after lock",
)
replace_exact(
    app_state,
    "#[tauri::command]\npub fn get_current_snapshot(app: AppHandle)",
    '''/// Returns the latest authoritative Rust snapshot for the open profile.
///
/// # Errors
///
/// Returns a structured error when no profile is ready or when the actor/bridge failed.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command injection supplies AppHandle by value"
)]
#[tauri::command]
pub fn get_current_snapshot(app: AppHandle)''',
    1,
    "Returns the latest authoritative Rust snapshot",
)
replace_exact(
    app_state,
    "#[tauri::command]\npub async fn close_profile(",
    '''/// Closes the active profile in reverse startup order.
///
/// # Errors
///
/// Returns a structured error when lifecycle state is unavailable, another open/close
/// is in progress, the close worker fails, or actor/database/profile-lock cleanup fails.
#[tauri::command]
pub async fn close_profile(''',
    1,
    "Closes the active profile in reverse startup order",
)

replace_exact(
    app_state,
    "open_runtime(paths, profile_id, &provider, notifications)",
    "open_runtime(&paths, profile_id, &provider, notifications)",
    2,
    "open_runtime(&paths, profile_id",
)
replace_exact(
    app_state,
    "fn open_runtime(\n    paths: DesktopProfilePaths,",
    "fn open_runtime(\n    paths: &DesktopProfilePaths,",
    1,
    "paths: &DesktopProfilePaths",
)
replace_exact(
    app_state,
    "ProfileLease::acquire(&paths, &profile_id)",
    "ProfileLease::acquire(paths, &profile_id)",
    1,
    "ProfileLease::acquire(paths, &profile_id)",
)

replace_exact(
    app_state,
    '''let cleanup = shutdown_owned_resources(ready.owned);
                let error = append_cleanup(primary, cleanup.err());
                state.fail_open(error.clone())?;
                Err(error)''',
    '''let cleanup = shutdown_owned_resources(ready.owned);
                let error = append_cleanup(primary, cleanup.err());
                if let Err(state_error) = state.fail_open(error.clone()) {
                    return Err(append_cleanup(error, Some(state_error)));
                }
                Err(error)''',
    1,
    "if let Err(state_error) = state.fail_open",
)
replace_once(
    app_state,
    r'''fn append_cleanup\(primary: DesktopErrorDto, cleanup: Option<DesktopErrorDto>\) -> DesktopErrorDto \{\s*cleanup\.map_or\(primary\.clone\(\), \|cleanup\| \{\s*DesktopErrorDto::new\(\s*&primary\.code,\s*&primary\.subsystem,\s*&primary\.severity,\s*primary\.retryable,\s*&format!\("\{\}; \{\}", primary\.message, cleanup\.message\),\s*\)\s*\}\)\s*\}''',
    '''fn append_cleanup(primary: DesktopErrorDto, cleanup: Option<DesktopErrorDto>) -> DesktopErrorDto {
    let Some(cleanup) = cleanup else {
        return primary;
    };
    let cleanup_message = cleanup.message;
    DesktopErrorDto::new(
        &primary.code,
        &primary.subsystem,
        &primary.severity,
        primary.retryable,
        &format!("{}; {cleanup_message}", primary.message),
    )
}''',
    "let cleanup_message = cleanup.message;",
)

shutdown = "desktop/src-tauri/src/shutdown.rs"
replace_exact(
    shutdown,
    '''/// Every cleanup phase is attempted. A later cleanup failure never overwrites
/// an earlier failure; the returned bounded error describes every failed phase.
pub fn shutdown_owned_resources''',
    '''/// Every cleanup phase is attempted. A later cleanup failure never overwrites
/// an earlier failure; the returned bounded error describes every failed phase.
///
/// # Errors
///
/// Returns one bounded structured error when actor shutdown, database shutdown, or
/// explicit profile-lock release fails. All later cleanup phases are still attempted.
pub fn shutdown_owned_resources''',
    1,
    "Returns one bounded structured error when actor shutdown",
)
replace_exact(
    shutdown,
    "pub fn cleanup_without_actor(",
    '''/// Cleans up database and profile-lock ownership after actor startup failed.
#[must_use]
pub fn cleanup_without_actor(''',
    1,
    "Cleans up database and profile-lock ownership",
)
replace_exact(
    shutdown,
    "pub fn cleanup_lease(",
    '''/// Releases a profile lease after an earlier startup stage failed.
#[must_use]
pub fn cleanup_lease(''',
    1,
    "Releases a profile lease after an earlier startup stage failed",
)
replace_exact(
    shutdown,
    "pub fn cleanup_with_actor(",
    '''/// Cleans up actor, database, and profile-lock ownership after startup failed.
#[must_use]
pub fn cleanup_with_actor(''',
    1,
    "Cleans up actor, database, and profile-lock ownership",
)
