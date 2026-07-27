package com.ekkus.silentdisco.app

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.identity.HostIdentityManager
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.rust.P2RecentSession
import com.ekkus.silentdisco.core.rust.P2RustBridge
import com.ekkus.silentdisco.core.rust.P2SessionOutcome
import com.ekkus.silentdisco.core.rust.P2SessionRole
import com.ekkus.silentdisco.core.rust.P2TrustedHost
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import com.ekkus.silentdisco.platform.persistence.AndroidP2Store
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

enum class P2StorageState {
    INITIALIZING,
    READY,
    ERROR,
}

enum class RecentAvailability {
    IDLE,
    CHECKING,
    AVAILABLE,
    UNAVAILABLE,
}

data class P2UiState(
    val storageState: P2StorageState = P2StorageState.INITIALIZING,
    val recentSessions: List<P2RecentSession> = emptyList(),
    val trustedHosts: List<P2TrustedHost> = emptyList(),
    val rejoinTarget: P2RecentSession? = null,
    val recentAvailability: RecentAvailability = RecentAvailability.IDLE,
    val validatedInvitation: P2ValidatedInvitation? = null,
    val hostQrPayload: String? = null,
    val lastMessage: String? = null,
    val lastError: String? = null,
) {
    val mostRecentSession: P2RecentSession? get() = recentSessions.firstOrNull()

    fun validatedHostIsTrusted(): Boolean {
        val invitation = validatedInvitation ?: return false
        return trustedHosts.any {
            it.fingerprint == invitation.hostFingerprint &&
                it.publicKeyDer.contentEquals(invitation.hostPublicKeyDer)
        }
    }
}

class P2ViewModel @JvmOverloads constructor(
    application: Application,
    private val store: AndroidP2Store = AndroidP2Store(application),
    private val hostIdentity: HostIdentityManager = HostIdentityManager(),
) : AndroidViewModel(application) {
    private val _uiState = MutableStateFlow(P2UiState())
    val uiState: StateFlow<P2UiState> = _uiState.asStateFlow()

    private var activeHost: ObservedSession? = null
    private var activeListener: ObservedSession? = null
    private var rejoinScanObserved = false

    init {
        viewModelScope.launch {
            runCatching { store.initialize() }
                .onSuccess { (recent, trusted) ->
                    _uiState.value = _uiState.value.copy(
                        storageState = P2StorageState.READY,
                        recentSessions = recent,
                        trustedHosts = trusted,
                        lastError = null,
                    )
                }
                .onFailure { error ->
                    _uiState.value = _uiState.value.copy(
                        storageState = P2StorageState.ERROR,
                        lastError = error.message ?: "Could not open optional P2 data",
                    )
                }
        }
    }

    fun observeAppState(appState: AppUiState) {
        observeHostSession(appState)
        observeListenerSession(appState)
        observeDiscovery(appState.discoveredSessions, appState.isScanning)
    }

    fun requestRecentSessionCheck(session: P2RecentSession) {
        rejoinScanObserved = false
        _uiState.value = _uiState.value.copy(
            rejoinTarget = session,
            recentAvailability = RecentAvailability.CHECKING,
            lastMessage = "Checking whether ${session.sessionName} is nearby…",
            lastError = null,
        )
    }

    fun cancelRecentSessionCheck() {
        rejoinScanObserved = false
        _uiState.value = _uiState.value.copy(
            rejoinTarget = null,
            recentAvailability = RecentAvailability.IDLE,
        )
    }

    fun prepareHostQr(appState: AppUiState) {
        val sessionId = appState.hostDiagnostics.sessionId.takeIf(String::isNotBlank)
            ?: run {
                _uiState.value = _uiState.value.copy(
                    lastError = "Start a host session before creating a QR invitation",
                )
                return
            }
        val session = appState.discoveredSessions.firstOrNull { it.id == sessionId }
            ?: SessionInfo(
                id = sessionId,
                name = appState.hostForm.sessionName,
                hostDeviceName = "This Android Host",
                approvalMode = appState.hostForm.approvalMode,
                inviteCodeRequired = appState.hostForm.approvalMode == ApprovalMode.INVITE_CODE,
            )
        viewModelScope.launch {
            runCatching {
                val now = System.currentTimeMillis()
                val publicKey = hostIdentity.publicKeyDer()
                val unsigned = P2RustBridge.prepareUnsignedQr(
                    sessionId = session.id,
                    sessionName = session.name,
                    hostName = session.hostDeviceName,
                    publicKeyDer = publicKey,
                    approvalMode = session.approvalMode.qrWireName(),
                    inviteCode = appState.hostForm.inviteCode
                        .takeIf { session.approvalMode == ApprovalMode.INVITE_CODE && it.isNotBlank() },
                    issuedAtMs = now,
                    expiresAtMs = now + QR_INVITATION_LIFETIME_MS,
                    nonce = UUID.randomUUID().toString(),
                )
                P2RustBridge.finalizeQr(
                    unsignedJson = unsigned,
                    signatureBase64url = hostIdentity.signBase64Url(unsigned.encodeToByteArray()),
                )
            }.onSuccess { payload ->
                _uiState.value = _uiState.value.copy(
                    hostQrPayload = payload,
                    lastMessage = "Created a signed QR invitation",
                    lastError = null,
                )
            }.onFailure { error ->
                _uiState.value = _uiState.value.copy(
                    hostQrPayload = null,
                    lastError = error.message ?: "Could not create a QR invitation",
                )
            }
        }
    }

    fun clearHostQr() {
        _uiState.value = _uiState.value.copy(hostQrPayload = null)
    }

    fun validateScannedQr(payload: String) {
        if (_uiState.value.storageState != P2StorageState.READY) {
            _uiState.value = _uiState.value.copy(lastError = "Optional P2 storage is not ready")
            return
        }
        viewModelScope.launch {
            runCatching { store.validateInvitation(payload) }
                .onSuccess { invitation ->
                    _uiState.value = _uiState.value.copy(
                        validatedInvitation = invitation,
                        lastMessage = "Verified QR signature from ${invitation.hostName}",
                        lastError = null,
                    )
                }
                .onFailure { error ->
                    _uiState.value = _uiState.value.copy(
                        validatedInvitation = null,
                        lastError = error.message ?: "QR invitation could not be verified",
                    )
                }
        }
    }

    fun trustValidatedHost() {
        val invitation = _uiState.value.validatedInvitation ?: run {
            _uiState.value = _uiState.value.copy(lastError = "Verify a host QR invitation first")
            return
        }
        viewModelScope.launch {
            runCatching { store.trustValidatedHost() }
                .onSuccess { trusted ->
                    _uiState.value = _uiState.value.copy(
                        trustedHosts = trusted,
                        lastMessage = "${invitation.hostName} is now a trusted host",
                        lastError = null,
                    )
                }
                .onFailure { error ->
                    _uiState.value = _uiState.value.copy(
                        lastError = error.message ?: "Could not persist trusted host",
                    )
                }
        }
    }

    fun deleteTrustedHost(fingerprint: String) {
        viewModelScope.launch {
            runCatching { store.deleteTrustedHost(fingerprint) }
                .onSuccess { (deleted, trusted) ->
                    _uiState.value = _uiState.value.copy(
                        trustedHosts = trusted,
                        lastMessage = if (deleted) {
                            "Trusted host removed"
                        } else {
                            "Trusted host was already absent"
                        },
                        lastError = null,
                    )
                }
                .onFailure { error ->
                    _uiState.value = _uiState.value.copy(
                        lastError = error.message ?: "Could not remove trusted host",
                    )
                }
        }
    }

    fun dismissValidatedInvitation() {
        _uiState.value = _uiState.value.copy(validatedInvitation = null)
    }

    fun consumeMessage() {
        _uiState.value = _uiState.value.copy(lastMessage = null)
    }

    fun consumeError() {
        _uiState.value = _uiState.value.copy(lastError = null)
    }

    private fun observeHostSession(appState: AppUiState) {
        val sessionId = appState.hostDiagnostics.sessionId.takeIf(String::isNotBlank)
        val active = appState.hostState in ACTIVE_HOST_STATES && sessionId != null
        if (active && activeHost?.sessionId != sessionId) {
            activeHost?.let { finishSession(it, P2SessionOutcome.FAILED) }
            activeHost = ObservedSession(
                sessionId = sessionId,
                role = P2SessionRole.HOST,
                sessionName = appState.hostForm.sessionName.ifBlank { "Silent Disco session" },
                hostName = "This Android Host",
                hostFingerprint = runCatching {
                    P2RustBridge.fingerprint(hostIdentity.publicKeyDer())
                }.getOrNull(),
                startedAtMs = System.currentTimeMillis(),
            )
        }
        val previous = activeHost ?: return
        val terminal = appState.hostState == HostLifecycleState.IDLE ||
            appState.hostState == HostLifecycleState.ERROR ||
            sessionId != previous.sessionId
        if (terminal) {
            val outcome = if (appState.hostState == HostLifecycleState.ERROR) {
                P2SessionOutcome.FAILED
            } else {
                P2SessionOutcome.COMPLETED
            }
            activeHost = null
            finishSession(previous, outcome)
        }
    }

    private fun observeListenerSession(appState: AppUiState) {
        val selected = appState.selectedSession
        if (
            appState.listenerState == ListenerLifecycleState.PLAYING &&
            selected != null &&
            activeListener?.sessionId != selected.id
        ) {
            activeListener?.let { finishSession(it, P2SessionOutcome.FAILED) }
            activeListener = ObservedSession(
                sessionId = selected.id,
                role = P2SessionRole.LISTENER,
                sessionName = selected.name,
                hostName = selected.hostDeviceName,
                hostFingerprint = _uiState.value.validatedInvitation
                    ?.takeIf { it.sessionId == selected.id }
                    ?.hostFingerprint,
                startedAtMs = System.currentTimeMillis(),
            )
        }
        val previous = activeListener ?: return
        val terminalState = appState.listenerState in setOf(
            ListenerLifecycleState.IDLE,
            ListenerLifecycleState.DISCONNECTED,
            ListenerLifecycleState.ERROR,
        )
        val selectedChanged = selected?.id != previous.sessionId
        if (terminalState || selectedChanged) {
            val outcome = when (appState.listenerState) {
                ListenerLifecycleState.ERROR,
                ListenerLifecycleState.DISCONNECTED,
                -> P2SessionOutcome.FAILED
                else -> P2SessionOutcome.COMPLETED
            }
            activeListener = null
            finishSession(previous, outcome)
        }
    }

    private fun observeDiscovery(sessions: List<SessionInfo>, scanning: Boolean) {
        val target = _uiState.value.rejoinTarget ?: return
        if (scanning) {
            rejoinScanObserved = true
            return
        }
        if (!rejoinScanObserved || _uiState.value.recentAvailability != RecentAvailability.CHECKING) {
            return
        }
        rejoinScanObserved = false
        val available = sessions.any { it.id == target.sessionId }
        _uiState.value = if (available) {
            _uiState.value.copy(
                recentAvailability = RecentAvailability.AVAILABLE,
                lastMessage = "${target.sessionName} is available nearby",
                lastError = null,
            )
        } else {
            _uiState.value.copy(
                recentAvailability = RecentAvailability.UNAVAILABLE,
                lastMessage = null,
                lastError = "${target.sessionName} is not currently nearby",
            )
        }
    }

    private fun finishSession(session: ObservedSession, outcome: P2SessionOutcome) {
        if (_uiState.value.storageState != P2StorageState.READY) return
        val endedAt = System.currentTimeMillis().coerceAtLeast(session.startedAtMs)
        viewModelScope.launch {
            runCatching {
                store.recordSession(
                    sessionId = session.sessionId,
                    role = session.role,
                    sessionName = session.sessionName,
                    hostName = session.hostName,
                    hostFingerprint = session.hostFingerprint,
                    startedAtMs = session.startedAtMs,
                    endedAtMs = endedAt,
                    outcome = outcome,
                )
            }.onSuccess { recent ->
                _uiState.value = _uiState.value.copy(recentSessions = recent, lastError = null)
            }.onFailure { error ->
                _uiState.value = _uiState.value.copy(
                    lastError = error.message ?: "Could not record recent session",
                )
            }
        }
    }

    override fun onCleared() {
        runBlocking(Dispatchers.IO) { store.close() }
        super.onCleared()
    }

    private data class ObservedSession(
        val sessionId: String,
        val role: P2SessionRole,
        val sessionName: String,
        val hostName: String,
        val hostFingerprint: String?,
        val startedAtMs: Long,
    )

    companion object {
        private const val QR_INVITATION_LIFETIME_MS = 5L * 60L * 1_000L
        private val ACTIVE_HOST_STATES = setOf(
            HostLifecycleState.WAITING_FOR_LISTENERS,
            HostLifecycleState.READY,
            HostLifecycleState.STREAMING,
            HostLifecycleState.PAUSED,
            HostLifecycleState.ENDING_SESSION,
        )
    }
}

internal fun ApprovalMode.qrWireName(): String = when (this) {
    ApprovalMode.MANUAL -> "manual"
    ApprovalMode.INVITE_CODE -> "invite_code"
    ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER -> "approved_devices"
}

internal fun P2ValidatedInvitation.toSessionInfo(): SessionInfo = SessionInfo(
    id = sessionId,
    name = sessionName,
    hostDeviceName = hostName,
    approvalMode = when (approvalMode) {
        "manual" -> ApprovalMode.MANUAL
        "invite_code" -> ApprovalMode.INVITE_CODE
        "approved_devices" -> ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
        else -> error("Rust returned an unsupported approval mode: $approvalMode")
    },
    inviteCodeRequired = approvalMode == "invite_code",
)
