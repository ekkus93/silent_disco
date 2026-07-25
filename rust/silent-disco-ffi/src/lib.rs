#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

//! Binding shell for the shared Rust core.
//!
//! This crate contains no domain logic. Unsafe code is denied by default. A
//! future module that must dereference foreign pointers may opt in only at the
//! smallest module or function scope, and every unsafe operation must document
//! its caller and lifetime invariants with a `# Safety` section.

use silent_disco_core::{CoreVersion, core_version, deterministic_smoke};

/// Binding-facing version record. Domain version ownership remains in the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfiCoreVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl From<CoreVersion> for FfiCoreVersion {
    fn from(value: CoreVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            patch: value.patch,
        }
    }
}

#[must_use]
pub fn ffi_core_version() -> FfiCoreVersion {
    core_version().into()
}

#[must_use]
pub const fn ffi_deterministic_smoke(input: u64) -> u64 {
    deterministic_smoke(input)
}

#[cfg(test)]
mod tests {
    use super::{FfiCoreVersion, ffi_core_version, ffi_deterministic_smoke};

    #[test]
    fn delegates_version_to_core() {
        assert_eq!(
            ffi_core_version(),
            FfiCoreVersion {
                major: 0,
                minor: 1,
                patch: 0,
            }
        );
    }

    #[test]
    fn delegates_smoke_function_to_core() {
        assert_eq!(ffi_deterministic_smoke(7), ffi_deterministic_smoke(7));
    }
}
