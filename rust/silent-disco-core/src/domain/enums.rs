use core::{fmt, str::FromStr};
use std::error::Error;

/// Failure to decode a stable domain-enum representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumDecodeError {
    UnsupportedWireName { enum_name: &'static str },
    UnsupportedStableCode { enum_name: &'static str, code: u16 },
}

impl fmt::Display for EnumDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedWireName { enum_name } => {
                write!(formatter, "unsupported wire name for {enum_name}")
            }
            Self::UnsupportedStableCode { enum_name, code } => {
                write!(formatter, "unsupported stable code {code} for {enum_name}")
            }
        }
    }
}

impl Error for EnumDecodeError {}

macro_rules! define_stable_enum {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $(
                $variant:ident = $code:literal => $wire_name:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        #[repr(u16)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                $variant = $code,
            )+
        }

        impl $name {
            /// Returns the stable numeric representation used across bindings and storage.
            #[must_use]
            pub const fn stable_code(self) -> u16 {
                self as u16
            }

            /// Returns the stable, presentation-independent wire name.
            #[must_use]
            pub const fn wire_name(self) -> &'static str {
                match self {
                    $(
                        Self::$variant => $wire_name,
                    )+
                }
            }

            /// Decodes a stable numeric representation.
            ///
            /// # Errors
            ///
            /// Returns [`EnumDecodeError`] when the code is not assigned to a
            /// known variant.
            pub const fn from_stable_code(code: u16) -> Result<Self, EnumDecodeError> {
                match code {
                    $(
                        $code => Ok(Self::$variant),
                    )+
                    _ => Err(EnumDecodeError::UnsupportedStableCode {
                        enum_name: stringify!($name),
                        code,
                    }),
                }
            }

            /// Decodes a stable wire name.
            ///
            /// # Errors
            ///
            /// Returns [`EnumDecodeError`] when the name is not assigned to a
            /// known variant. The rejected value is intentionally not copied
            /// into the error.
            pub fn from_wire_name(value: &str) -> Result<Self, EnumDecodeError> {
                match value {
                    $(
                        $wire_name => Ok(Self::$variant),
                    )+
                    _ => Err(EnumDecodeError::UnsupportedWireName {
                        enum_name: stringify!($name),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.wire_name())
            }
        }

        impl FromStr for $name {
            type Err = EnumDecodeError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::from_wire_name(value)
            }
        }

        impl TryFrom<u16> for $name {
            type Error = EnumDecodeError;

            fn try_from(value: u16) -> Result<Self, EnumDecodeError> {
                Self::from_stable_code(value)
            }
        }
    };
}

define_stable_enum! {
    /// Application role selected by the user.
    pub enum AppRole {
        Host = 1 => "host",
        Listener = 2 => "listener",
    }
}

define_stable_enum! {
    /// Policy used to admit listeners to a host session.
    pub enum ApprovalMode {
        Manual = 1 => "manual",
        TrustedDevices = 2 => "trusted_devices",
        InviteCode = 3 => "invite_code",
    }
}

define_stable_enum! {
    /// Authoritative host-session lifecycle.
    pub enum HostLifecycle {
        Idle = 1 => "idle",
        CreatingSession = 2 => "creating_session",
        Advertising = 3 => "advertising",
        WaitingForListeners = 4 => "waiting_for_listeners",
        Ready = 5 => "ready",
        Streaming = 6 => "streaming",
        Paused = 7 => "paused",
        EndingSession = 8 => "ending_session",
        Error = 9 => "error",
    }
}

define_stable_enum! {
    /// Authoritative listener-session lifecycle.
    pub enum ListenerLifecycle {
        Idle = 1 => "idle",
        Scanning = 2 => "scanning",
        SessionSelected = 3 => "session_selected",
        JoinRequested = 4 => "join_requested",
        AwaitingApproval = 5 => "awaiting_approval",
        Approved = 6 => "approved",
        Connecting = 7 => "connecting",
        SyncingClock = 8 => "syncing_clock",
        Buffering = 9 => "buffering",
        Playing = 10 => "playing",
        Reconnecting = 11 => "reconnecting",
        Desynced = 12 => "desynced",
        Disconnected = 13 => "disconnected",
        Error = 14 => "error",
    }
}

define_stable_enum! {
    /// Playback state shared by host and listener domain models.
    pub enum PlaybackState {
        Stopped = 1 => "stopped",
        Buffering = 2 => "buffering",
        Ready = 3 => "ready",
        Playing = 4 => "playing",
        Paused = 5 => "paused",
        Underrun = 6 => "underrun",
        Error = 7 => "error",
    }
}

define_stable_enum! {
    /// Platform-independent transport lifecycle.
    pub enum TransportState {
        Idle = 1 => "idle",
        Discovering = 2 => "discovering",
        Advertising = 3 => "advertising",
        Connecting = 4 => "connecting",
        Connected = 5 => "connected",
        Retrying = 6 => "retrying",
        Disconnected = 7 => "disconnected",
        Failed = 8 => "failed",
    }
}

define_stable_enum! {
    /// Confidence classification for the current synchronization estimate.
    pub enum SyncConfidence {
        Unknown = 1 => "unknown",
        Poor = 2 => "poor",
        Fair = 3 => "fair",
        Good = 4 => "good",
        Excellent = 5 => "excellent",
    }
}

define_stable_enum! {
    /// Trust policy associated with a listener device.
    pub enum TrustState {
        SessionOnly = 1 => "session_only",
        Trusted = 2 => "trusted",
    }
}

define_stable_enum! {
    /// Delivery classification for a broadcast operation.
    pub enum DeliverySeverity {
        Ok = 1 => "ok",
        ZeroPeers = 2 => "zero_peers",
        PartialFailure = 3 => "partial_failure",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppRole, ApprovalMode, DeliverySeverity, EnumDecodeError, HostLifecycle, ListenerLifecycle,
        PlaybackState, SyncConfidence, TransportState, TrustState,
    };

    #[test]
    fn preserves_semantic_equivalence_with_current_android_states() {
        assert_eq!(AppRole::Host.wire_name(), "host");
        assert_eq!(ApprovalMode::TrustedDevices.wire_name(), "trusted_devices");
        assert_eq!(
            HostLifecycle::WaitingForListeners.wire_name(),
            "waiting_for_listeners"
        );
        assert_eq!(ListenerLifecycle::SyncingClock.wire_name(), "syncing_clock");
        assert_eq!(PlaybackState::Underrun.wire_name(), "underrun");
        assert_eq!(TransportState::Retrying.wire_name(), "retrying");
        assert_eq!(SyncConfidence::Excellent.wire_name(), "excellent");
        assert_eq!(TrustState::SessionOnly.wire_name(), "session_only");
        assert_eq!(
            DeliverySeverity::PartialFailure.wire_name(),
            "partial_failure"
        );
    }

    #[test]
    fn numeric_and_text_representations_round_trip_deterministically() {
        let lifecycle = ListenerLifecycle::Reconnecting;
        assert_eq!(
            ListenerLifecycle::from_stable_code(lifecycle.stable_code()),
            Ok(lifecycle)
        );
        assert_eq!(
            ListenerLifecycle::from_wire_name(lifecycle.wire_name()),
            Ok(lifecycle)
        );
        assert_eq!(lifecycle.to_string(), "reconnecting");
    }

    #[test]
    fn rejects_unknown_representations_without_copying_unknown_text() {
        assert_eq!(
            AppRole::from_stable_code(0),
            Err(EnumDecodeError::UnsupportedStableCode {
                enum_name: "AppRole",
                code: 0,
            })
        );

        let error = AppRole::from_wire_name("secret-looking-unknown-value")
            .expect_err("unknown wire name must fail");
        assert_eq!(
            error,
            EnumDecodeError::UnsupportedWireName {
                enum_name: "AppRole",
            }
        );
        assert!(!error.to_string().contains("secret-looking-unknown-value"));
    }

    #[test]
    fn domain_names_are_not_localized_presentation_labels() {
        assert_eq!(
            HostLifecycle::CreatingSession.to_string(),
            "creating_session"
        );
        assert_ne!(
            HostLifecycle::CreatingSession.to_string(),
            "Creating session…"
        );
        assert_eq!(ApprovalMode::InviteCode.to_string(), "invite_code");
        assert_ne!(ApprovalMode::InviteCode.to_string(), "Invite code");
    }
}
