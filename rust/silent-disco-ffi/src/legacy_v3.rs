//! Whether this build may use plaintext v3 at all, for hosting or joining.

/// Whether this build may host or join plaintext v3.
///
/// Phase 4 (owner decision 2026-11-13): production builds (Android, which
/// builds this crate without features) host and join v4 only; listeners
/// join through the host's signed QR. The `legacy-v3` feature keeps
/// v3 for tests and lab tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyV3Policy {
    Allowed,
    Forbidden,
}

impl LegacyV3Policy {
    /// The policy this build was compiled with.
    pub(crate) const BUILD: Self = if cfg!(feature = "legacy-v3") {
        Self::Allowed
    } else {
        Self::Forbidden
    };
}
