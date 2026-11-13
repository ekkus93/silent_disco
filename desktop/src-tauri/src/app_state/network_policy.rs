//! Persisted host network policy switches: the shared-address-space opt-in
//! and secure transport (v4 QUIC). Both are applied to the live network
//! control first, then saved, and restored at profile open
//! (`construct.rs`), so a restart keeps the operator's choice.

use super::DesktopAppState;
use super::errors::poisoned_state_error;
use super::state::DesktopRuntimeState;
use crate::dto::DesktopErrorDto;
use crate::platform::invitation::current_wall_clock_ms;
use crate::platform::network::DesktopHostNetworkControl;
use crate::platform::network_dto::NetworkInterfaceSnapshotDto;
use silent_disco_core::domain::TuningSettings;
use silent_disco_core::storage::{DatabaseClient, StoredSettings};
use std::sync::Arc;

/// One persisted host network policy switch.
#[derive(Clone, Copy)]
enum NetworkPolicySwitch {
    AllowSharedAddressSpace,
    SecureTransport,
}

impl NetworkPolicySwitch {
    fn current(self, snapshot: &NetworkInterfaceSnapshotDto) -> bool {
        match self {
            Self::AllowSharedAddressSpace => snapshot.allow_shared_address_space,
            Self::SecureTransport => snapshot.secure_transport,
        }
    }

    fn apply(self, network: &DesktopHostNetworkControl, on: bool) -> Result<(), DesktopErrorDto> {
        match self {
            Self::AllowSharedAddressSpace => network.set_allow_shared_address_space(on),
            Self::SecureTransport => network.set_secure_transport(on),
        }
    }

    fn store(self, settings: &mut StoredSettings, on: bool) {
        match self {
            Self::AllowSharedAddressSpace => settings.allow_shared_address_space = on,
            Self::SecureTransport => settings.secure_transport = on,
        }
    }
}

impl DesktopAppState {
    /// Applies the shared-address-space opt-in, then persists it.
    pub(crate) fn set_allow_shared_address_space(
        &self,
        allow: bool,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        self.set_network_policy(NetworkPolicySwitch::AllowSharedAddressSpace, allow)
    }

    /// Turns secure transport (v4 QUIC) on or off for the next host start,
    /// then persists it (owner decision 2026-11-13; it used to reset to off
    /// on every launch).
    pub(crate) fn set_secure_transport(
        &self,
        enabled: bool,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        self.set_network_policy(NetworkPolicySwitch::SecureTransport, enabled)
    }

    /// Apply first so an invalid change (for example, while hosting) is never
    /// persisted. If storage then fails, restore the previous in-memory value
    /// so the screen and the saved profile cannot disagree.
    fn set_network_policy(
        &self,
        switch: NetworkPolicySwitch,
        on: bool,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let (network, database) = self.ready_network_and_database()?;
        let previous = switch.current(&network.snapshot()?);
        switch.apply(&network, on)?;
        if let Err(error) = persist_switch(&database, switch, on) {
            switch.apply(&network, previous)?;
            return Err(error);
        }
        network.snapshot()
    }

    fn ready_network_and_database(
        &self,
    ) -> Result<(Arc<DesktopHostNetworkControl>, DatabaseClient), DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => {
                Ok((Arc::clone(&ready.network), ready.owned.database.client()))
            }
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
            DesktopRuntimeState::Closed
            | DesktopRuntimeState::Opening { .. }
            | DesktopRuntimeState::Closing => Err(DesktopErrorDto::new(
                "desktop.profile.not_ready",
                "runtime",
                "error",
                true,
                "no desktop profile is ready",
            )),
        }
    }
}

/// `save_settings` writes the whole row, so every other field is carried
/// forward from what is stored.
fn persist_switch(
    database: &DatabaseClient,
    switch: NetworkPolicySwitch,
    on: bool,
) -> Result<(), DesktopErrorDto> {
    let storage_error = |error: silent_disco_core::storage::StorageError| {
        DesktopErrorDto::from(error.to_core_error())
    };
    let mut settings = database
        .load_settings()
        .map_err(storage_error)?
        .unwrap_or(StoredSettings {
            tuning: TuningSettings::default(),
            allow_shared_address_space: false,
            secure_transport: false,
            updated_at_ms: 0,
        });
    switch.store(&mut settings, on);
    settings.updated_at_ms = current_wall_clock_ms();
    database.save_settings(&settings).map_err(storage_error)
}
