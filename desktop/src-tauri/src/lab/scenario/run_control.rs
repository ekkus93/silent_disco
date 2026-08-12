use std::fmt;
use std::sync::{Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScenarioRunControlError {
    Stopped,
    Poisoned,
}

impl fmt::Display for ScenarioRunControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => formatter.write_str("scenario run was stopped by the operator"),
            Self::Poisoned => formatter.write_str("scenario run-control mutex was poisoned"),
        }
    }
}

impl std::error::Error for ScenarioRunControlError {}

#[derive(Debug, Default)]
struct ScenarioRunControlState {
    paused: bool,
    stop_requested: bool,
}

/// Cooperative control for one live scenario execution.
///
/// A pause is observed only at explicit deterministic scenario boundaries:
/// after the current step has settled and before the next virtual-time/step
/// transition begins. This keeps the current scenario step atomic. A stop is
/// observed both at those boundaries and inside bounded settlement waits so
/// teardown is prompt even when a step would otherwise consume the full
/// wall-clock settlement timeout.
#[derive(Debug, Default)]
pub(crate) struct ScenarioRunControl {
    state: Mutex<ScenarioRunControlState>,
    resumed: Condvar,
}

impl ScenarioRunControl {
    pub(crate) fn pause(&self) -> Result<(), ScenarioRunControlError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ScenarioRunControlError::Poisoned)?;
        if state.stop_requested {
            return Err(ScenarioRunControlError::Stopped);
        }
        state.paused = true;
        Ok(())
    }

    pub(crate) fn resume(&self) -> Result<(), ScenarioRunControlError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ScenarioRunControlError::Poisoned)?;
        if state.stop_requested {
            return Err(ScenarioRunControlError::Stopped);
        }
        state.paused = false;
        self.resumed.notify_all();
        Ok(())
    }

    pub(crate) fn request_stop(&self) -> Result<(), ScenarioRunControlError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ScenarioRunControlError::Poisoned)?;
        state.stop_requested = true;
        state.paused = false;
        self.resumed.notify_all();
        Ok(())
    }

    /// Waits while paused, then returns unless a stop was requested.
    pub(crate) fn wait_until_runnable(&self) -> Result<(), ScenarioRunControlError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ScenarioRunControlError::Poisoned)?;
        while state.paused && !state.stop_requested {
            state = self
                .resumed
                .wait(state)
                .map_err(|_| ScenarioRunControlError::Poisoned)?;
        }
        if state.stop_requested {
            Err(ScenarioRunControlError::Stopped)
        } else {
            Ok(())
        }
    }

    /// Checks only for stop, deliberately ignoring pause. Used while the
    /// current deterministic step is settling so pause cannot split a step in
    /// half, while stop can still interrupt a five-second settlement wait.
    pub(crate) fn check_stop(&self) -> Result<(), ScenarioRunControlError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ScenarioRunControlError::Poisoned)?;
        if state.stop_requested {
            Err(ScenarioRunControlError::Stopped)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScenarioRunControl, ScenarioRunControlError};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn pause_blocks_a_boundary_until_resume() {
        let control = Arc::new(ScenarioRunControl::default());
        control.pause().expect("pause");
        let waiter = Arc::clone(&control);
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = thread::spawn(move || {
            waiter.wait_until_runnable().expect("resume releases boundary");
            tx.send(()).expect("signal completion");
        });

        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        control.resume().expect("resume");
        rx.recv_timeout(Duration::from_secs(1))
            .expect("boundary released");
        thread.join().expect("waiter joins");
    }

    #[test]
    fn stop_releases_a_paused_boundary_as_stopped() {
        let control = Arc::new(ScenarioRunControl::default());
        control.pause().expect("pause");
        let waiter = Arc::clone(&control);
        let thread = thread::spawn(move || waiter.wait_until_runnable());

        control.request_stop().expect("stop");
        assert_eq!(
            thread.join().expect("waiter joins"),
            Err(ScenarioRunControlError::Stopped)
        );
    }
}
