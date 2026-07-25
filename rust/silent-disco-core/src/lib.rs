#![forbid(unsafe_code)]

/// ABI-independent version information for the shared domain core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl CoreVersion {
    pub const CURRENT: Self = Self {
        major: 0,
        minor: 1,
        patch: 0,
    };
}

/// Returns the shared core version without consulting platform state.
#[must_use]
pub const fn core_version() -> CoreVersion {
    CoreVersion::CURRENT
}

/// Deterministic smoke function used only to verify host and binding integration.
///
/// This is intentionally not domain behavior. It may be removed once the real
/// command/snapshot API supplies an equivalent integration test surface.
#[must_use]
pub const fn deterministic_smoke(input: u64) -> u64 {
    input.rotate_left(13) ^ 0x5344_4953_434f_0001
}

#[cfg(test)]
mod tests {
    use super::{core_version, deterministic_smoke, CoreVersion};

    #[test]
    fn exposes_current_version() {
        assert_eq!(
            core_version(),
            CoreVersion {
                major: 0,
                minor: 1,
                patch: 0,
            }
        );
    }

    #[test]
    fn smoke_function_is_deterministic() {
        assert_eq!(deterministic_smoke(42), deterministic_smoke(42));
        assert_ne!(deterministic_smoke(42), deterministic_smoke(43));
    }
}
