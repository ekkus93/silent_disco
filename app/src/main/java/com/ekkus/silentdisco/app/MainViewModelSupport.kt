package com.ekkus.silentdisco.app

internal fun MainViewModel.clearScanState() {
    discovery.cancelScanWindow()
    _uiState.value = _uiState.value.copy(isScanning = false)
}
