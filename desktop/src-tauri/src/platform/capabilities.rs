use silent_disco_core::runtime::CapabilitySnapshot;

#[must_use]
pub(crate) const fn desktop_capabilities() -> CapabilitySnapshot {
    CapabilitySnapshot {
        nearby_discovery_available: false,
        nearby_advertising_available: false,
        local_network_available: false,
        audio_source_selection_available: true,
        audio_output_available: false,
        secure_store_available: true,
    }
}
