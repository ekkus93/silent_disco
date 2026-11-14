package com.ekkus.silentdisco.feature.settings

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.identity.KeystoreHostTlsSigner
import com.ekkus.silentdisco.core.uniffi.listVerifiedListenersInDatabase
import com.ekkus.silentdisco.core.uniffi.revokeVerifiedListenerInDatabase
import com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/**
 * A listener this host remembered with "Always allow": its key-bound device ID,
 * name and approval time. Read from the host's Rust-owned verified records.
 */
data class ApprovedListener(
    val deviceId: String,
    val displayName: String,
    val approvedAtMs: Long,
)

data class TrustedDevicesUiState(
    val isLoading: Boolean = false,
    val devices: List<ApprovedListener> = emptyList(),
    val deletingDeviceId: String? = null,
    val error: String? = null,
    val message: String? = null,
)

internal interface TrustedDeviceStore {
    suspend fun initialize()
    suspend fun listApprovedListeners(): List<ApprovedListener>

    /** Revokes a remembered listener. Returns false if it was not trusted. */
    suspend fun revokeApproval(deviceId: String): Boolean
    suspend fun close()
}

/**
 * The host's verified listener records for this phone's host TLS key (the key
 * listeners pin). Rust reads and writes the records; Kotlin only lends the
 * database handle and the public key.
 */
private class AndroidTrustedDeviceStore(
    application: Application,
) : TrustedDeviceStore {
    private val store = AndroidRustDomainStore(application)
    private val hostTlsSigner = KeystoreHostTlsSigner()

    override suspend fun initialize() {
        store.initialize()
    }

    // Both run on the store's IO dispatcher: the Keystore read and the Rust
    // database call block.
    override suspend fun listApprovedListeners(): List<ApprovedListener> =
        store.withDatabaseHandle { handle ->
            listVerifiedListenersInDatabase(handle, hostTlsSigner.spkiDer()).map { listener ->
                ApprovedListener(
                    deviceId = listener.deviceId,
                    displayName = listener.displayName,
                    approvedAtMs = listener.enrolledAtMs.toLong(),
                )
            }
        }

    override suspend fun revokeApproval(deviceId: String): Boolean =
        store.withDatabaseHandle { handle ->
            revokeVerifiedListenerInDatabase(
                handle,
                hostTlsSigner.spkiDer(),
                deviceId,
                System.currentTimeMillis().toULong(),
            )
        }

    override suspend fun close() {
        store.close()
    }
}

class TrustedDevicesViewModel internal constructor(
    application: Application,
    private val store: TrustedDeviceStore,
) : AndroidViewModel(application) {
    constructor(application: Application) : this(
        application = application,
        store = AndroidTrustedDeviceStore(application),
    )

    private val logger = AppLogger()
    private val _uiState = MutableStateFlow(TrustedDevicesUiState())
    val uiState: StateFlow<TrustedDevicesUiState> = _uiState.asStateFlow()

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
                val devices = store.listApprovedListeners()
                _uiState.value = TrustedDevicesUiState(devices = devices)
            } catch (cancelled: CancellationException) {
                throw cancelled
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
                val deleted = store.revokeApproval(deviceId)
                val devices = store.listApprovedListeners()
                _uiState.value = TrustedDevicesUiState(
                    devices = devices,
                    message = if (deleted) {
                        "This phone will need your approval next time it joins, " +
                            "including in a session you are hosting now."
                    } else {
                        "That phone was already removed. The list has been refreshed."
                    },
                )
            } catch (cancelled: CancellationException) {
                throw cancelled
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
