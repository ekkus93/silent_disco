from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one match, found {count}: {old[:140]!r}")
    file_path.write_text(text.replace(old, new), encoding="utf-8")


Path("app/src/main/java/com/ekkus/silentdisco/core/persistence/AndroidStorageRepository.kt").write_text(r'''package com.ekkus.silentdisco.core.persistence

import android.content.Context
import com.ekkus.silentdisco.core.rust.RustLegacyImportOutcome
import com.ekkus.silentdisco.core.rust.RustStorageBridge
import com.ekkus.silentdisco.core.rust.RustStorageSession
import com.ekkus.silentdisco.core.rust.RustStoredSettings
import com.ekkus.silentdisco.core.rust.RustTrustedDevice

private const val LEGACY_PREFERENCES_NAME = "silent-disco"

data class AndroidStorageSnapshot(
    val schemaVersion: Int,
    val settings: RustStoredSettings,
    val trustedDevices: List<RustTrustedDevice>,
    val legacyImport: RustLegacyImportOutcome,
)

interface AppStorageRepository : AutoCloseable {
    fun initialize(): AndroidStorageSnapshot
    fun saveSettings(settings: RustStoredSettings): RustStoredSettings
    fun upsertTrustedDevice(device: RustTrustedDevice): RustTrustedDevice
    fun deleteTrustedDevice(deviceId: String): Boolean
    fun listTrustedDevices(): List<RustTrustedDevice>
}

/**
 * Android control-plane owner for one process-lifetime Rust storage session.
 *
 * This class chooses the app-private path and reads the fixed legacy Android
 * key contract. All validation, migrations, SQL, transactions, and durable
 * domain persistence remain Rust-owned. Initialization has no SharedPreferences
 * fallback: any failure closes the candidate Rust worker and is rethrown.
 */
class AndroidStorageRepository(
    private val context: Context,
    private val clock: () -> Long = System::currentTimeMillis,
    private val legacyReader: LegacyAndroidImportReader = LegacyAndroidImportReader(clock),
) : AppStorageRepository {
    private var session: RustStorageSession? = null
    private var committedImportOutcome: RustLegacyImportOutcome? = null

    @Synchronized
    override fun initialize(): AndroidStorageSnapshot {
        session?.let { existing ->
            val outcome = committedImportOutcome ?: throw AndroidStorageInitializationException(
                "Rust storage session exists without its committed import outcome",
            )
            return readCurrentSnapshot(existing, outcome)
        }
        val databaseFile = AndroidDatabasePathProvider.resolve(context)
        val candidate = RustStorageBridge.open(databaseFile.absolutePath)
        try {
            val preferences = context.getSharedPreferences(
                LEGACY_PREFERENCES_NAME,
                Context.MODE_PRIVATE,
            )
            val legacySnapshot = legacyReader.read(preferences)
            val importOutcome = candidate.importLegacy(legacySnapshot.request)
            legacyReader.deleteCommittedKeys(preferences, legacySnapshot)
            val persistedSettings = candidate.loadSettings() ?: RustStoredSettings.defaults(clock()).also {
                candidate.saveSettings(it)
            }
            val snapshot = AndroidStorageSnapshot(
                schemaVersion = candidate.schemaVersion,
                settings = persistedSettings,
                trustedDevices = candidate.listTrustedDevices(),
                legacyImport = importOutcome,
            )
            session = candidate
            committedImportOutcome = importOutcome
            return snapshot
        } catch (error: Throwable) {
            runCatching(candidate::close).exceptionOrNull()?.let(error::addSuppressed)
            throw error
        }
    }

    @Synchronized
    override fun saveSettings(settings: RustStoredSettings): RustStoredSettings =
        requireSession().saveSettings(settings)

    @Synchronized
    override fun upsertTrustedDevice(device: RustTrustedDevice): RustTrustedDevice =
        requireSession().upsertTrustedDevice(device)

    @Synchronized
    override fun deleteTrustedDevice(deviceId: String): Boolean =
        requireSession().deleteTrustedDevice(deviceId)

    @Synchronized
    override fun listTrustedDevices(): List<RustTrustedDevice> =
        requireSession().listTrustedDevices()

    @Synchronized
    override fun close() {
        val current = session ?: return
        current.close()
        session = null
        committedImportOutcome = null
    }

    private fun readCurrentSnapshot(
        current: RustStorageSession,
        importOutcome: RustLegacyImportOutcome,
    ): AndroidStorageSnapshot {
        val settings = current.loadSettings() ?: throw AndroidStorageInitializationException(
            "Rust storage was initialized without the required settings row",
        )
        return AndroidStorageSnapshot(
            schemaVersion = current.schemaVersion,
            settings = settings,
            trustedDevices = current.listTrustedDevices(),
            legacyImport = importOutcome,
        )
    }

    private fun requireSession(): RustStorageSession = session ?: throw AndroidStorageInitializationException(
        "Rust storage has not completed initialization",
    )
}

class AndroidStorageInitializationException(
    message: String,
) : IllegalStateException(message)
''', encoding="utf-8")

replace_exact(
    "app/src/main/java/com/ekkus/silentdisco/app/AppState.kt",
    '''enum class TuningField {
''',
    '''enum class StorageInitializationState {
    INITIALIZING,
    READY,
    FAILED,
}

enum class TuningField {
''',
)
replace_exact(
    "app/src/main/java/com/ekkus/silentdisco/app/AppState.kt",
    '''data class AppUiState(
    val selectedRole: AppRole? = null,
''',
    '''data class AppUiState(
    val storageState: StorageInitializationState = StorageInitializationState.INITIALIZING,
    val storageSchemaVersion: Int? = null,
    val selectedRole: AppRole? = null,
''',
)

main_path = "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt"
replace_exact(main_path, "import android.content.Context\n", "")
replace_exact(main_path, "import androidx.core.content.edit\n", "")
replace_exact(main_path, "import com.ekkus.silentdisco.core.persistence.LegacyPreferencesContract\n", '''import com.ekkus.silentdisco.core.persistence.AndroidStorageRepository
import com.ekkus.silentdisco.core.persistence.AppStorageRepository
import com.ekkus.silentdisco.core.rust.RustStoredSettings
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
''')
replace_exact(main_path, "import kotlinx.coroutines.Job\n", '''import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
''')
replace_exact(main_path, "import kotlinx.coroutines.launch\n", '''import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
''')
replace_exact(
    main_path,
    '''class MainViewModel @JvmOverloads constructor(
    application: Application,
    private val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),
) : AndroidViewModel(application) {
''',
    '''class MainViewModel @JvmOverloads constructor(
    application: Application,
    private val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),
    storageRepositoryOverride: AppStorageRepository? = null,
) : AndroidViewModel(application) {
''',
)
replace_exact(
    main_path,
    '''    private val hostTimingService = HostTimingService()
    private val preferences = application.getSharedPreferences("silent-disco", Context.MODE_PRIVATE)
''',
    '''    private val hostTimingService = HostTimingService()
    private val storageRepository: AppStorageRepository = storageRepositoryOverride
        ?: (application as? SilentDiscoApplication)?.storageRepository
        ?: AndroidStorageRepository(application)
    private val trustedDeviceIds = mutableSetOf<String>()
    private val tuningUpdateMutex = Mutex()
''',
)
replace_exact(
    main_path,
    '''            discoveredSessions = emptyList(),
            tuningSettings = loadTuningSettings(),
''',
    '''            discoveredSessions = emptyList(),
            tuningSettings = TuningSettings(),
            lastMessage = "Opening persistent storage…",
''',
)
replace_exact(
    main_path,
    '''    init {
        observeTransport()
''',
    '''    init {
        initializeStorage()
        observeTransport()
''',
)
replace_exact(
    main_path,
    '''    fun createHostSession(): Boolean {
        val validationError = validateHostForm(_uiState.value)
''',
    '''    fun createHostSession(): Boolean {
        if (!requireStorageReady("Hosting a session")) return false
        val validationError = validateHostForm(_uiState.value)
''',
)
replace_exact(
    main_path,
    '''    fun scanForSessions() {
        logger.i("listener.scan", "Scanning for nearby sessions")
''',
    '''    fun scanForSessions() {
        if (!requireStorageReady("Scanning for sessions")) return
        logger.i("listener.scan", "Scanning for nearby sessions")
''',
)
replace_exact(
    main_path,
    '''    fun requestJoin() {
        val session = _uiState.value.selectedSession ?: run {
''',
    '''    fun requestJoin() {
        if (!requireStorageReady("Joining a session")) return
        val session = _uiState.value.selectedSession ?: run {
''',
)

old_approve = '''    fun approveJoinRequest(request: JoinRequest) {
        val sessionId = currentSessionId ?: run {
            _uiState.value = _uiState.value.copy(lastError = "No active host session")
            return
        }
        logger.i("approval.approve", "Approving ${request.listenerName}")
        viewModelScope.launch {
            val delivered = runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.JoinApproval(
                        version = 1,
                        sessionId = sessionId,
                        listenerId = request.listenerId,
                        trustedForFuture = _uiState.value.hostForm.rememberApprovedDevices,
                    ),
                )
            }.map { result ->
                reportHostBroadcastDelivery("send join approval", result, requireAnyPeer = true)
            }.getOrElse { error ->
                handleHostControlFailure("send join approval", error)
                false
            }

            if (!delivered) return@launch

            if (_uiState.value.hostForm.rememberApprovedDevices) {
                trustListener(request.listenerId)
            }
            _uiState.value = _uiState.value.copy(
                pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },
                approvedListeners = (_uiState.value.approvedListeners + request.toListenerInfo()).distinctBy { it.deviceId },
                lastMessage = "${request.listenerName} approved",
                lastError = null,
            )
            refreshHostDiagnostics()
        }
    }
'''
new_approve = '''    fun approveJoinRequest(request: JoinRequest) {
        val sessionId = currentSessionId ?: run {
            _uiState.value = _uiState.value.copy(lastError = "No active host session")
            return
        }
        if (!requireStorageReady("Approving a listener")) return
        logger.i("approval.approve", "Approving ${request.listenerName}")
        viewModelScope.launch {
            val wasAlreadyTrusted = request.listenerId in trustedDeviceIds
            val rememberRequested = _uiState.value.hostForm.rememberApprovedDevices
            var trustFailure: String? = null
            val trustedForFuture = if (rememberRequested || wasAlreadyTrusted) {
                runCatching {
                    withContext(Dispatchers.IO) {
                        storageRepository.upsertTrustedDevice(request.toRustTrustedDevice(System.currentTimeMillis()))
                    }
                }.fold(
                    onSuccess = {
                        trustedDeviceIds += request.listenerId
                        true
                    },
                    onFailure = { error ->
                        trustFailure = "Listener approved for this session, but durable trust was not saved: ${error.visibleMessage()}"
                        logger.e("storage.trust", trustFailure!!, error)
                        false
                    },
                )
            } else {
                false
            }

            val delivered = runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.JoinApproval(
                        version = 1,
                        sessionId = sessionId,
                        listenerId = request.listenerId,
                        trustedForFuture = trustedForFuture,
                    ),
                )
            }.map { result ->
                reportHostBroadcastDelivery("send join approval", result, requireAnyPeer = true)
            }.getOrElse { error ->
                handleHostControlFailure("send join approval", error)
                false
            }

            if (!delivered) return@launch

            val approved = request.toListenerInfo().copy(
                trustState = if (trustedForFuture) TrustState.TRUSTED_PLACEHOLDER else TrustState.SESSION_ONLY,
            )
            _uiState.value = _uiState.value.copy(
                pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },
                approvedListeners = (_uiState.value.approvedListeners + approved).distinctBy { it.deviceId },
                lastMessage = if (trustFailure == null) {
                    "${request.listenerName} approved"
                } else {
                    "${request.listenerName} approved for this session"
                },
                lastError = trustFailure,
            )
            refreshHostDiagnostics()
        }
    }
'''
replace_exact(main_path, old_approve, new_approve)

old_trust = '''    fun trustListener(listenerId: String) {
        preferences.edit { putBoolean(LegacyPreferencesContract.trustedDeviceKey(listenerId), true) }
        _uiState.value = _uiState.value.copy(
            approvedListeners = _uiState.value.approvedListeners.map {
                if (it.deviceId == listenerId) it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER) else it
            },
            lastMessage = "Trusted listener ${listenerId.take(6)}",
        )
        refreshHostDiagnostics()
    }
'''
new_trust = '''    fun trustListener(listenerId: String) {
        if (!requireStorageReady("Trusting a listener")) return
        val listener = _uiState.value.approvedListeners.firstOrNull { it.deviceId == listenerId }
        val displayName = listener?.displayName ?: listenerId
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    storageRepository.upsertTrustedDevice(
                        trustedDevice(
                            listenerId = listenerId,
                            displayName = displayName,
                            timestampMs = System.currentTimeMillis(),
                        ),
                    )
                }
            }.onSuccess {
                trustedDeviceIds += listenerId
                _uiState.value = _uiState.value.copy(
                    approvedListeners = _uiState.value.approvedListeners.map {
                        if (it.deviceId == listenerId) it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER) else it
                    },
                    lastMessage = "Trusted listener ${listenerId.take(6)}",
                    lastError = null,
                )
                refreshHostDiagnostics()
            }.onFailure { error ->
                val message = "Unable to save trusted listener: ${error.visibleMessage()}"
                logger.e("storage.trust", message, error)
                _uiState.value = _uiState.value.copy(lastError = message)
            }
        }
    }
'''
replace_exact(main_path, old_trust, new_trust)

old_adjust = '''    fun adjustTuning(field: TuningField, direction: Int) {
        val updated = _uiState.value.tuningSettings.adjust(field, direction)
        persistTuningSettings(updated)
        listenerSyncController = _uiState.value.selectedSession?.let { createSyncController(SessionId(it.id)) }
        _uiState.value = _uiState.value.copy(
            tuningSettings = updated,
            lastMessage = "Updated tuning: ${updated.summary()}",
            lastError = null,
        )
        if (resyncJob?.isActive == true) {
            startPeriodicListenerResync()
        }
    }
'''
new_adjust = '''    fun adjustTuning(field: TuningField, direction: Int) {
        if (!requireStorageReady("Changing tuning settings")) return
        viewModelScope.launch {
            tuningUpdateMutex.withLock {
                val requested = _uiState.value.tuningSettings.adjust(field, direction)
                runCatching {
                    withContext(Dispatchers.IO) {
                        storageRepository.saveSettings(
                            requested.toRustStoredSettings(System.currentTimeMillis()),
                        )
                    }
                }.onSuccess { persisted ->
                    val updated = persisted.toTuningSettings()
                    listenerSyncController = _uiState.value.selectedSession?.let {
                        createSyncController(SessionId(it.id))
                    }
                    _uiState.value = _uiState.value.copy(
                        tuningSettings = updated,
                        lastMessage = "Updated tuning: ${updated.summary()}",
                        lastError = null,
                    )
                    if (resyncJob?.isActive == true) {
                        startPeriodicListenerResync()
                    }
                }.onFailure { error ->
                    val message = "Unable to save tuning settings: ${error.visibleMessage()}"
                    logger.e("storage.settings", message, error)
                    _uiState.value = _uiState.value.copy(lastError = message)
                }
            }
        }
    }
'''
replace_exact(main_path, old_adjust, new_adjust)

replace_exact(
    main_path,
    '''            lastMessage = "Join approved",
            lastError = null,
        )
        if (message.trustedForFuture) {
            preferences.edit { putBoolean("trusted:${message.listenerId}", true) }
        }
''',
    '''            lastMessage = if (message.trustedForFuture) {
                "Join approved; the host saved this device as trusted"
            } else {
                "Join approved"
            },
            lastError = null,
        )
''',
)

old_load_persist = '''    private fun loadTuningSettings(): TuningSettings = TuningSettings(
        syncSampleWindow = preferences.getInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, 12),
        syncCadenceMs = preferences.getLong(LegacyPreferencesContract.SYNC_CADENCE_MS, 2_000L),
        startupBufferMs = preferences.getLong(LegacyPreferencesContract.STARTUP_BUFFER_MS, 400L),
        latePacketThresholdMs = preferences.getLong(LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS, 40L),
        hardResyncThresholdMs = preferences.getLong(LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS, 120L),
        syncDriftThresholdMs = java.lang.Double.longBitsToDouble(
            preferences.getLong(LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS, java.lang.Double.doubleToLongBits(18.0)),
        ),
    )

    private fun persistTuningSettings(settings: TuningSettings) {
        preferences.edit {
            putInt(LegacyPreferencesContract.SYNC_SAMPLE_WINDOW, settings.syncSampleWindow)
            putLong(LegacyPreferencesContract.SYNC_CADENCE_MS, settings.syncCadenceMs)
            putLong(LegacyPreferencesContract.STARTUP_BUFFER_MS, settings.startupBufferMs)
            putLong(LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS, settings.latePacketThresholdMs)
            putLong(LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS, settings.hardResyncThresholdMs)
            putLong(LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS, java.lang.Double.doubleToLongBits(settings.syncDriftThresholdMs))
        }
    }

'''
new_storage_helpers = '''    private fun initializeStorage() {
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    storageRepository.initialize()
                }
            }.onSuccess { snapshot ->
                trustedDeviceIds.clear()
                trustedDeviceIds += snapshot.trustedDevices.map { it.deviceId }
                val settings = snapshot.settings.toTuningSettings()
                _uiState.value = _uiState.value.copy(
                    storageState = StorageInitializationState.READY,
                    storageSchemaVersion = snapshot.schemaVersion,
                    tuningSettings = settings,
                    lastMessage = "Persistent storage ready (schema ${snapshot.schemaVersion})",
                    lastError = null,
                )
                logger.i(
                    "storage.initialize",
                    "Rust storage ready at schema ${snapshot.schemaVersion}; legacy import=${snapshot.legacyImport.disposition}",
                )
            }.onFailure { error ->
                val message = "Storage initialization failed: ${error.visibleMessage()}"
                logger.e("storage.initialize", message, error)
                _uiState.value = _uiState.value.copy(
                    storageState = StorageInitializationState.FAILED,
                    storageSchemaVersion = null,
                    lastMessage = null,
                    lastError = message,
                )
            }
        }
    }

    private fun requireStorageReady(operation: String): Boolean {
        if (_uiState.value.storageState == StorageInitializationState.READY) return true
        val message = when (_uiState.value.storageState) {
            StorageInitializationState.INITIALIZING ->
                "$operation is unavailable while persistent storage initializes"
            StorageInitializationState.FAILED ->
                "$operation is unavailable because persistent storage failed"
            StorageInitializationState.READY -> return true
        }
        _uiState.value = _uiState.value.copy(lastError = message)
        return false
    }

'''
replace_exact(main_path, old_load_persist, new_storage_helpers)

replace_exact(
    main_path,
    '''internal fun requireHostSessionForPlayback(currentSessionId: SessionId?): String? =
''',
    '''internal fun TuningSettings.toRustStoredSettings(updatedAtMs: Long): RustStoredSettings = RustStoredSettings(
    syncSampleWindow = syncSampleWindow,
    syncCadenceMs = syncCadenceMs,
    startupBufferMs = startupBufferMs,
    latePacketThresholdMs = latePacketThresholdMs,
    hardResyncThresholdMs = hardResyncThresholdMs,
    syncDriftThresholdMs = syncDriftThresholdMs,
    scanWindowMs = scanWindowMs,
    updatedAtMs = updatedAtMs,
)

internal fun RustStoredSettings.toTuningSettings(): TuningSettings = TuningSettings(
    syncSampleWindow = syncSampleWindow,
    syncCadenceMs = syncCadenceMs,
    startupBufferMs = startupBufferMs,
    latePacketThresholdMs = latePacketThresholdMs,
    hardResyncThresholdMs = hardResyncThresholdMs,
    syncDriftThresholdMs = syncDriftThresholdMs,
    scanWindowMs = scanWindowMs,
)

private fun JoinRequest.toRustTrustedDevice(timestampMs: Long): RustTrustedDevice = trustedDevice(
    listenerId = listenerId,
    displayName = listenerName,
    timestampMs = timestampMs,
)

private fun trustedDevice(
    listenerId: String,
    displayName: String,
    timestampMs: Long,
): RustTrustedDevice = RustTrustedDevice(
    deviceId = listenerId,
    displayName = displayName,
    trustState = "trusted",
    firstSeenMs = timestampMs,
    lastSeenMs = timestampMs,
    updatedAtMs = timestampMs,
)

private fun Throwable.visibleMessage(): String = message?.takeIf { it.isNotBlank() }
    ?: this::class.java.simpleName

internal fun requireHostSessionForPlayback(currentSessionId: SessionId?): String? =
''',
)

Path("app/src/main/java/com/ekkus/silentdisco/feature/home/HomeScreen.kt").write_text(r'''package com.ekkus.silentdisco.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.app.permissionSummary

@Composable
fun HomeScreen(
    uiState: AppUiState,
    onRequestPermissions: () -> Unit,
    onHostClick: () -> Unit,
    onJoinClick: () -> Unit,
) {
    val storageReady = uiState.storageState == StorageInitializationState.READY
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Silent Disco PoC", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Play music in sync across multiple phones — no internet required.",
            style = MaterialTheme.typography.bodyLarge,
        )

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Persistent storage", style = MaterialTheme.typography.titleMedium)
                Text(
                    when (uiState.storageState) {
                        StorageInitializationState.INITIALIZING -> "Opening the Rust-owned private database…"
                        StorageInitializationState.READY ->
                            "Ready — schema ${uiState.storageSchemaVersion ?: "unknown"}"
                        StorageInitializationState.FAILED ->
                            uiState.lastError ?: "Persistent storage failed to initialize"
                    },
                )
            }
        }

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Permissions", style = MaterialTheme.typography.titleMedium)
                Text(uiState.permissionSummary())
                Button(onClick = onRequestPermissions) {
                    Text("Grant / Refresh Permissions")
                }
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(
                onClick = onHostClick,
                enabled = storageReady,
                modifier = Modifier.weight(1f),
            ) {
                Text("Host a Session")
            }
            Button(
                onClick = onJoinClick,
                enabled = storageReady,
                modifier = Modifier.weight(1f),
            ) {
                Text("Join a Session")
            }
        }
    }
}
''', encoding="utf-8")

Path("app/src/test/java/com/ekkus/silentdisco/app/StorageMappingTest.kt").write_text(r'''package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class StorageMappingTest {
    @Test
    fun tuningSettingsRoundTripThroughTheRustDtoWithoutDroppingScanWindow() {
        val original = TuningSettings(
            syncSampleWindow = 18,
            syncCadenceMs = 2_750L,
            startupBufferMs = 550L,
            latePacketThresholdMs = 55L,
            hardResyncThresholdMs = 180L,
            syncDriftThresholdMs = 22.5,
            scanWindowMs = 6_500L,
        )

        val stored = original.toRustStoredSettings(updatedAtMs = 99_000L)

        assertThat(stored.updatedAtMs).isEqualTo(99_000L)
        assertThat(stored.toTuningSettings()).isEqualTo(original)
    }
}
''', encoding="utf-8")
