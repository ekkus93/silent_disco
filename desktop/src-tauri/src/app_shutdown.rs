//! Coordinates the window-close-initiated, whole-application shutdown
//! (Block 36.3), layered above [`crate::app_state::DesktopAppState`]'s
//! profile-focused close/reopen lifecycle. Kept as a small, independently
//! testable state machine plus a bounded-wait primitive, separate from the
//! Tauri window-event glue in `lib.rs` (which stays thin and untested,
//! matching this codebase's existing precedent of testing logic, not glue).
//!
//! A close request must be idempotent (Block 36.3 "duplicate close is
//! idempotent"), its progress must be observable (Block 36.3 "progress is
//! visible"), and a stuck teardown must become a visible failure rather
//! than ever authorize forcibly freeing callback-visible memory (spec
//! section 27, Block 36.3 "timeout does not free callback-visible memory
//! unsafely").

use crate::dto::DesktopErrorDto;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// How long a full application shutdown may run before it is reported as a
/// visible failure instead of blocking the close indefinitely. Chosen
/// generously relative to this block's own observed teardown cost (a
/// handful of worker joins plus one `SQLite` WAL checkpoint, all sub-second
/// in every test in this crate) -- see
/// `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` Block 36 for the
/// reasoning.
pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Observable phase of the whole-application shutdown (Block 36.1
/// "shutting down"/"shutdown failed"/"terminated" -- "closed"/"opening"/
/// "ready" are `DesktopAppState`'s own concern, not this coordinator's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppShutdownPhase {
    NotRequested,
    ShuttingDown,
    Terminated,
    ShutdownFailed(DesktopErrorDto),
}

/// Outcome of attempting to claim the right to perform the shutdown work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShutdownClaim {
    /// This caller is the one that must perform the shutdown and report
    /// its outcome via [`AppShutdownCoordinator::finish`].
    Perform,
    /// A shutdown is already in progress, already terminated, or already
    /// failed -- this caller must not attempt teardown again (Block 36.3
    /// "duplicate close is idempotent"). The current phase is included so
    /// the caller can still react (e.g. a repeated close request once
    /// already `Terminated` should let the process actually exit).
    AlreadyHandled(AppShutdownPhase),
}

pub(crate) struct AppShutdownCoordinator {
    phase: Mutex<AppShutdownPhase>,
}

impl Default for AppShutdownCoordinator {
    fn default() -> Self {
        Self {
            phase: Mutex::new(AppShutdownPhase::NotRequested),
        }
    }
}

impl AppShutdownCoordinator {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the current phase.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the coordinator's mutex was poisoned.
    pub(crate) fn phase(&self) -> Result<AppShutdownPhase, DesktopErrorDto> {
        Ok(self
            .phase
            .lock()
            .map_err(|_| coordinator_poisoned_error())?
            .clone())
    }

    /// Atomically claims the right to perform the shutdown -- returns
    /// [`ShutdownClaim::Perform`] exactly once across this coordinator's
    /// lifetime; every other call observes
    /// [`ShutdownClaim::AlreadyHandled`] instead.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the coordinator's mutex was poisoned.
    pub(crate) fn claim(&self) -> Result<ShutdownClaim, DesktopErrorDto> {
        let mut phase = self
            .phase
            .lock()
            .map_err(|_| coordinator_poisoned_error())?;
        match &*phase {
            AppShutdownPhase::NotRequested => {
                *phase = AppShutdownPhase::ShuttingDown;
                Ok(ShutdownClaim::Perform)
            }
            other => Ok(ShutdownClaim::AlreadyHandled(other.clone())),
        }
    }

    /// Records the outcome of a claimed shutdown attempt.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the coordinator's mutex was poisoned.
    pub(crate) fn finish(
        &self,
        result: &Result<(), DesktopErrorDto>,
    ) -> Result<(), DesktopErrorDto> {
        let mut phase = self
            .phase
            .lock()
            .map_err(|_| coordinator_poisoned_error())?;
        *phase = match result {
            Ok(()) => AppShutdownPhase::Terminated,
            Err(error) => AppShutdownPhase::ShutdownFailed(error.clone()),
        };
        Ok(())
    }
}

fn coordinator_poisoned_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.shutdown.coordinator_poisoned",
        "runtime",
        "fatal",
        false,
        "the application shutdown coordinator's mutex was poisoned",
    )
}

/// Runs `work` on a dedicated background thread and waits up to `timeout`
/// for it to finish.
///
/// On timeout, the spawned thread's ownership of `work`'s captured
/// resources is **never** joined, cancelled, or dropped by this call --
/// the thread keeps running independently to completion (best case) or
/// indefinitely (worst case), and this call reports a visible timeout
/// failure instead. This is deliberate: forcibly reclaiming a still-live
/// worker's resources risks freeing memory a real-time callback or worker
/// thread may still dereference (spec section 27, "a timeout does not
/// authorize freeing callback-visible memory").
///
/// # Errors
///
/// Returns a structured error if the worker thread could not be spawned,
/// if `work` itself fails, or if `work` does not finish within `timeout`.
pub(crate) fn run_with_timeout<F>(work: F, timeout: Duration) -> Result<(), DesktopErrorDto>
where
    F: FnOnce() -> Result<(), DesktopErrorDto> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("silent-disco-desktop-app-shutdown".to_owned())
        .spawn(move || {
            // The result is best-effort delivered; if the receiving end
            // was already dropped (a prior timeout gave up waiting), there
            // is nothing left to report to and that is not itself a
            // failure of the shutdown work.
            let _ = sender.send(work());
        });
    if let Err(error) = spawned {
        return Err(DesktopErrorDto::new(
            "desktop.shutdown.worker_spawn_failed",
            "runtime",
            "fatal",
            true,
            &format!("could not start the shutdown worker thread: {error}"),
        ));
    }
    receiver.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(DesktopErrorDto::new(
            "desktop.shutdown.timed_out",
            "runtime",
            "fatal",
            false,
            "application shutdown did not complete within the bounded timeout; \
             in-progress teardown was left running rather than forcibly freed",
        ))
    })
}

/// Returns the current whole-application shutdown phase (Block 36.1/36.3
/// "progress is visible"). The frontend polls this the same way it polls
/// every other status screen in this codebase -- no push channel is
/// needed since the webview stays fully alive while a close request is
/// pending (the native close is prevented, not the process).
///
/// # Errors
///
/// Returns a structured error only if the coordinator's mutex was
/// poisoned.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_app_shutdown_state(
    coordinator: tauri::State<'_, AppShutdownCoordinator>,
) -> Result<crate::dto::AppShutdownPhaseDto, DesktopErrorDto> {
    coordinator
        .phase()
        .map(crate::dto::AppShutdownPhaseDto::from)
}

/// Handles one window `CloseRequested` event. The native close itself is
/// always prevented by the caller (`lib.rs`) before this runs; this
/// function decides whether to actually exit the process, start a bounded
/// shutdown attempt, or do nothing (Block 36.3).
///
/// Runs on Tauri's event-loop thread and must never block it -- the real
/// shutdown work always happens on a dedicated spawned thread, freeing
/// this call to return immediately either way.
pub(crate) fn handle_close_requested(app: &AppHandle) {
    let coordinator = app.state::<AppShutdownCoordinator>();
    // A poisoned coordinator leaves nothing safe to decide -- the native
    // close stays prevented and the window remains open rather than
    // guessing.
    let Ok(claim) = coordinator.claim() else {
        return;
    };
    match claim {
        ShutdownClaim::AlreadyHandled(AppShutdownPhase::Terminated) => {
            // Shutdown already completed on an earlier request; this
            // close request is what finally lets the process exit.
            app.exit(0);
        }
        // Already shutting down, or already failed -- idempotent no-op;
        // the in-flight attempt (or the now-visible failure) stands.
        ShutdownClaim::AlreadyHandled(_) => {}
        ShutdownClaim::Perform => {
            let worker_app = app.clone();
            thread::spawn(move || {
                let perform_app = worker_app.clone();
                let result = run_with_timeout(
                    move || crate::app_state::close_profile_sync(&perform_app),
                    SHUTDOWN_TIMEOUT,
                );
                let coordinator = worker_app.state::<AppShutdownCoordinator>();
                // A poisoned coordinator here means the phase can no
                // longer be recorded, but the exit decision below still
                // reflects the real, just-computed result.
                let _ = coordinator.finish(&result);
                if result.is_ok() {
                    worker_app.exit(0);
                }
                // On failure the window is deliberately left open --
                // `ShutdownFailed` is now visible via `get_app_shutdown_state`,
                // and no resources were forcibly freed to get here.
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppShutdownCoordinator, AppShutdownPhase, ShutdownClaim, run_with_timeout};
    use crate::dto::DesktopErrorDto;
    use std::sync::mpsc;
    use std::time::Duration;

    fn sample_error() -> DesktopErrorDto {
        DesktopErrorDto::new("desktop.test.failure", "runtime", "error", true, "boom")
    }

    /// Block 36.3 "duplicate close is idempotent": only the first claim
    /// performs the work; every later call observes the same in-flight (or
    /// resolved) phase instead of being told to perform it again.
    #[test]
    fn only_the_first_claim_performs_and_later_claims_observe_the_same_phase() {
        let coordinator = AppShutdownCoordinator::new();
        assert_eq!(coordinator.claim().expect("claim"), ShutdownClaim::Perform);
        assert_eq!(
            coordinator.claim().expect("second claim"),
            ShutdownClaim::AlreadyHandled(AppShutdownPhase::ShuttingDown)
        );
        assert_eq!(
            coordinator.claim().expect("third claim"),
            ShutdownClaim::AlreadyHandled(AppShutdownPhase::ShuttingDown)
        );
    }

    #[test]
    fn finish_success_reaches_terminated_and_stays_idempotent_after() {
        let coordinator = AppShutdownCoordinator::new();
        assert_eq!(coordinator.claim().expect("claim"), ShutdownClaim::Perform);
        coordinator.finish(&Ok(())).expect("finish");
        assert_eq!(
            coordinator.phase().expect("phase"),
            AppShutdownPhase::Terminated
        );
        assert_eq!(
            coordinator.claim().expect("claim after terminated"),
            ShutdownClaim::AlreadyHandled(AppShutdownPhase::Terminated)
        );
    }

    /// Block 36.1 "shutdown failed": a failed attempt's error is preserved
    /// and observable, not collapsed into a generic state.
    #[test]
    fn finish_failure_reaches_shutdown_failed_with_the_real_error() {
        let coordinator = AppShutdownCoordinator::new();
        assert_eq!(coordinator.claim().expect("claim"), ShutdownClaim::Perform);
        coordinator.finish(&Err(sample_error())).expect("finish");
        assert_eq!(
            coordinator.phase().expect("phase"),
            AppShutdownPhase::ShutdownFailed(sample_error())
        );
    }

    /// The common, fast path: work finishes well within the timeout.
    #[test]
    fn run_with_timeout_returns_the_real_result_when_work_finishes_in_time() {
        let result = run_with_timeout(|| Ok(()), Duration::from_secs(5));
        assert!(result.is_ok());

        let result = run_with_timeout(|| Err(sample_error()), Duration::from_secs(5));
        assert_eq!(result.expect_err("failure"), sample_error());
    }

    /// Block 36.3 "timeout becomes visible failure" / "timeout does not
    /// free callback-visible memory unsafely": when `work` does not finish
    /// in time, this call returns promptly with a timeout error -- without
    /// waiting for, joining, or otherwise reclaiming the still-running
    /// worker. Releasing the gate afterward and letting the detached
    /// worker finish independently proves nothing was force-torn-down.
    #[test]
    fn run_with_timeout_returns_promptly_and_never_joins_a_slow_worker() {
        let (release_sender, release_receiver) = mpsc::channel::<()>();
        let (finished_sender, finished_receiver) = mpsc::channel::<()>();

        let result = run_with_timeout(
            move || {
                // Blocks until explicitly released below -- simulates a
                // stuck teardown phase.
                let _ = release_receiver.recv();
                let _ = finished_sender.send(());
                Ok(())
            },
            Duration::from_millis(50),
        );

        assert_eq!(
            result.expect_err("must time out, not hang").code,
            "desktop.shutdown.timed_out"
        );

        // The detached worker is still alive and makes real progress once
        // released -- it was never joined, cancelled, or torn down by the
        // timeout itself.
        release_sender.send(()).expect("release the worker");
        finished_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the detached worker eventually completes on its own");
    }
}
