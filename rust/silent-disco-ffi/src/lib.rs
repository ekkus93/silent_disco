#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

//! Binding shell for the shared Rust core.
//!
//! This crate contains no domain logic. Unsafe code is denied by default. A
//! module that requires an unsafe export attribute may opt in only at the
//! smallest scope, while unsafe blocks and foreign-pointer dereferences remain
//! prohibited unless their invariants are documented explicitly.

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

/// Native symbols consumed by Android before generated bindings are introduced.
///
/// Rust 2024 requires `no_mangle` to be declared as an unsafe attribute. This
/// module permits only those attributes. The functions do not dereference JNI
/// handles, access Java state, allocate, block, or run on the audio render path.
mod android_abi {
    #![allow(unsafe_code)]

    use core::ffi::c_void;

    /// ABI contract implemented by the current native library.
    pub const CORE_ABI_VERSION: u32 = 1;

    /// Returns the stable ABI contract version exposed to non-Rust callers.
    #[unsafe(no_mangle)]
    pub extern "C" fn silent_disco_core_abi_version() -> u32 {
        CORE_ABI_VERSION
    }

    /// JNI entry point used only by the Android platform bridge and smoke tests.
    #[allow(non_snake_case)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeAbiVersion(
        _environment: *mut c_void,
        _class: *mut c_void,
    ) -> i32 {
        match i32::try_from(silent_disco_core_abi_version()) {
            Ok(version) => version,
            Err(_) => -1,
        }
    }
}

pub use android_abi::{CORE_ABI_VERSION, silent_disco_core_abi_version};

#[cfg(test)]
mod tests {
    use super::{
        CORE_ABI_VERSION, FfiCoreVersion, ffi_core_version, ffi_deterministic_smoke,
        silent_disco_core_abi_version,
    };

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

    #[test]
    fn exports_stable_abi_version() {
        assert_eq!(silent_disco_core_abi_version(), CORE_ABI_VERSION);
        assert_eq!(CORE_ABI_VERSION, 1);
    }
}
