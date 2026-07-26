package com.ekkus.silentdisco.preview

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.feature.settings.TrustedDevicesScreen
import com.ekkus.silentdisco.feature.settings.TrustedDevicesUiState
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

private val previewTrustedDevices = listOf(
    RustTrustedDevice(
        deviceId = "preview-kitchen-phone",
        displayName = "Kitchen phone",
        lastSeenMs = 1_750_000_000_000L,
    ),
    RustTrustedDevice(
        deviceId = "preview-riley-phone",
        displayName = "Riley's phone",
        lastSeenMs = 1_750_086_400_000L,
    ),
)

@Preview(name = "Approved devices - loading", showBackground = true)
@Composable
private fun TrustedDevicesLoadingPreview() {
    TrustedDevicesPreviewSurface(
        uiState = TrustedDevicesUiState(isLoading = true),
    )
}

@Preview(name = "Approved devices - empty", showBackground = true)
@Composable
private fun TrustedDevicesEmptyPreview() {
    TrustedDevicesPreviewSurface(
        uiState = TrustedDevicesUiState(),
    )
}

@Preview(name = "Approved devices - populated", showBackground = true)
@Composable
private fun TrustedDevicesPopulatedPreview() {
    TrustedDevicesPreviewSurface(
        uiState = TrustedDevicesUiState(devices = previewTrustedDevices),
    )
}

@Preview(name = "Approved devices - removing", showBackground = true)
@Composable
private fun TrustedDevicesRemovingPreview() {
    TrustedDevicesPreviewSurface(
        uiState = TrustedDevicesUiState(
            devices = previewTrustedDevices,
            deletingDeviceId = previewTrustedDevices.first().deviceId,
        ),
    )
}

@Preview(name = "Approved devices - failure", showBackground = true)
@Composable
private fun TrustedDevicesFailurePreview() {
    TrustedDevicesPreviewSurface(
        uiState = TrustedDevicesUiState(
            error = "Approved devices could not be loaded. Try again.",
        ),
    )
}

@Composable
private fun TrustedDevicesPreviewSurface(uiState: TrustedDevicesUiState) {
    SilentDiscoTheme {
        TrustedDevicesScreen(
            uiState = uiState,
            onBack = {},
            onRefresh = {},
            onDelete = {},
        )
    }
}
