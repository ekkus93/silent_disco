package com.ekkus.silentdisco.app

internal fun MainViewModel.clearScanState() {
    scanJob?.cancel()
    scanJob = null
    _uiState.value = _uiState.value.copy(isScanning = false)
}
