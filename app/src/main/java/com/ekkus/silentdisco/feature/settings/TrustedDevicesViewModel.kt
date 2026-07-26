package com.ekkus.silentdisco.feature.settings

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

data class TrustedDevicesUiState(
    val isLoading: Boolean = false,
    val devices: List<RustTrustedDevice> = emptyList(),
    val deletingDeviceId: String? = null,
    val error: String? = null,
    val message: String? = null,
)

internal interface TrustedDeviceStore {
    suspend fun initialize()
    suspend fun listTrustedDevices(): List<RustTrustedDevice>
    suspend fun deleteTrustedDevice(deviceId: String): Boolean
    suspend fun close()
}

private class AndroidTrustedDeviceStore(
    application: Application,
) : TrustedDeviceStore {
    private val store = AndroidRustDomainStore(application)

    override suspend fun initialize() {
        store.initialize()
    }

    override suspend fun listTrustedDevices(): List<RustTrustedDevice> =
        store.listTrustedDevices()

    override suspend fun deleteTrustedDevice(deviceId: String): Boolean =
        store.deleteTrustedDevice(deviceId)

    override suspend fun close() {
        store.close()
    }
}

class TrustedDevicesViewModel(
    application: Application,
) : AndroidViewModel(application) {
    private var store: TrustedDeviceStore = AndroidTrustedDeviceStore(application)
    private val logger = AppLogger()
    private val _uiState = MutableStateFlow(TrustedDevicesUiState())
    val uiState: StateFlow<TrustedDevicesUiState> = _uiState.asStateFlow()

    internal constructor(
        application: Application,
        trustedDeviceStore: TrustedDeviceStore,
    ) : this(application) {
        store = trustedDeviceStore
    }

    fun refresh() {
        if (_uiState.value.isLoading || _uiState.value.deletingDeviceId != null) return
        _uiState.update {
            it.copy(
                isLoading = true,
                error = null,
                message = null,
            )
        }
        viewModelScope.launch {
            try {
                store.initialize()
                val devices = store.listTrustedDevices()
                _uiState.value = TrustedDevicesUiState(devices = devices)
            } catch (error: Throwable) {
                logger.e("trusted-devices.load", "Could not load approved devices", error)
                _uiState.update {
                    it.copy(
                        isLoading = false,
                        error = "Approved devices could not be loaded. Try again.",
                    )
                }
            }
        }
    }

    fun deleteDevice(deviceId: String) {
        require(deviceId.isNotBlank()) { "Trusted device ID must not be blank" }
        if (_uiState.value.isLoading || _uiState.value.deletingDeviceId != null) return
        _uiState.update {
            it.copy(
                deletingDeviceId = deviceId,
                error = null,
                message = null,
            )
        }
        viewModelScope.launch {
            try {
                store.initialize()
                val deleted = store.deleteTrustedDevice(deviceId)
                val devices = store.listTrustedDevices()
                _uiState.value = TrustedDevicesUiState(
                    devices = devices,
                    message = if (deleted) {
                        "This phone will need approval before joining a future session."
                    } else {
                        "That phone was already removed. The list has been refreshed."
                    },
                )
            } catch (error: Throwable) {
                logger.e("trusted-devices.delete", "Could not remove approved device", error)
                _uiState.update {
                    it.copy(
                        deletingDeviceId = null,
                        error = "The approved device could not be removed. Try again.",
                    )
                }
            }
        }
    }

    fun clearMessage() {
        _uiState.update { it.copy(message = null) }
    }

    override fun onCleared() {
        runBlocking(Dispatchers.IO) {
            runCatching { store.close() }
                .onFailure { error ->
                    logger.e("trusted-devices.close", "Could not close approved-device store", error)
                }
        }
        super.onCleared()
    }
}
