//! Explicit, distinguishable playback-runtime failure exposed to platform
//! bindings.

use core::fmt;

use crate::audio_abi::AudioAbiError;

/// Explicit, distinguishable failure exposed to the platform binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenerPlaybackError {
    /// The requested scheduler, ring, or pump configuration was rejected.
    InvalidConfiguration(String),
    /// The handle was already stopped.
    Stopped(String),
    /// The pump thread could not be started, or ended abnormally.
    PumpThread(String),
    /// A sync probe or response was rejected by the estimator.
    Sync(String),
    /// The optional debug PCM capture could not be started.
    DebugCapture(String),
}

impl fmt::Display for ListenerPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Stopped(message)
            | Self::PumpThread(message)
            | Self::Sync(message)
            | Self::DebugCapture(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ListenerPlaybackError {}

impl From<AudioAbiError> for ListenerPlaybackError {
    fn from(error: AudioAbiError) -> Self {
        Self::InvalidConfiguration(format!("{error:?}"))
    }
}
