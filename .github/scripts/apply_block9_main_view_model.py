from pathlib import Path

path = Path("app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt")
text = path.read_text()


def replace_once(old: str, new: str, label: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    text = text.replace(old, new)


replace_once("import android.content.Context\n", "", "Context import")
replace_once("import androidx.core.content.edit\n", "", "preferences edit import")
replace_once(
    "import com.ekkus.silentdisco.core.persistence.LegacyPreferencesContract\n",
    "import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings\n",
    "legacy preferences import",
)
replace_once(
    "import com.ekkus.silentdisco.core.transport.WifiDirectTransportService\n",
    "import com.ekkus.silentdisco.core.transport.WifiDirectTransportService\n"
    "import com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore\n",
    "domain store import",
)
replace_once(
    "import kotlinx.coroutines.Job\n",
    "import kotlinx.coroutines.Dispatchers\nimport kotlinx.coroutines.Job\n",
    "Dispatchers import",
)
replace_once(
    "import kotlinx.coroutines.launch\n",
    "import kotlinx.coroutines.launch\nimport kotlinx.coroutines.runBlocking\n",
    "runBlocking import",
)
replace_once(
    "    private val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),\n"
    ") : AndroidViewModel(application) {",
    "    private val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),\n"
    "    private val domainStore: AndroidRustDomainStore = AndroidRustDomainStore(application),\n"
    ") : AndroidViewModel(application) {",
    "constructor persistence dependency",
)
replace_once(
    "    private val hostTimingService = HostTimingService()\n"
    "    private val preferences = application.getSharedPreferences(\"silent-disco\", Context.MODE_PRIVATE)\n",
    "    private val hostTimingService = HostTimingService()\n",
    "preferences field",
)
replace_once(
    "            tuningSettings = loadTuningSettings(),\n",
    "            tuningSettings = TuningSettings(),\n",
    "initial tuning fallback",
)
replace_once(
    "    private val localListenerDeviceId = \"listener-device\"\n\n"
    "    init {\n"
    "        observeTransport()",
    "    private val localListenerDeviceId = \"listener-device\"\n"
    "    @Volatile\n"
    "    private var persistenceReady = false\n\n"
    "    init {\n"
    "        initializeDomainPersistence()\n"
    "        observeTransport()",
    "persistence initialization",
)
replace_once(
    "    fun createHostSession(): Boolean {\n",
    "    fun createHostSession(): Boolean {\n"
    "        if (!requirePersistenceReady(\"start a host session\")) return false\n",
    "host persistence gate",
)
replace_once(
    "    fun scanForSessions() {\n",
    "    fun scanForSessions() {\n"
    "        if (!requirePersistenceReady(\"scan for sessions\")) return\n",
    "scan persistence gate",
)
replace_once(
    "    fun requestJoin() {\n",
    "    fun requestJoin() {\n"
    "        if (!requirePersistenceReady(\"join a session\")) return\n",
    "join persistence gate",
)
replace_once(
    "            if (_uiState.value.hostForm.rememberApprovedDevices) {\n"
    "                trustListener(request.listenerId)\n"
    "            }\n"
    "            _uiState.value = _uiState.value.copy(\n"
    "                pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },\n"
    "                approvedListeners = (_uiState.value.approvedListeners + request.toListenerInfo()).distinctBy { it.deviceId },\n"
    "                lastMessage = \"${request.listenerName} approved\",\n"
    "                lastError = null,\n"
    "            )",
    "            val trustPersisted = if (_uiState.value.hostForm.rememberApprovedDevices) {\n"
    "                persistTrustedListener(request.listenerId, request.listenerName)\n"
    "            } else {\n"
    "                true\n"
    "            }\n"
    "            _uiState.value = _uiState.value.copy(\n"
    "                pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },\n"
    "                approvedListeners = (_uiState.value.approvedListeners + request.toListenerInfo()).distinctBy { it.deviceId },\n"
    "                lastMessage = \"${request.listenerName} approved\",\n"
    "                lastError = if (trustPersisted) null else _uiState.value.lastError,\n"
    "            )",
    "approval trust persistence",
)
replace_once(
    "    fun trustListener(listenerId: String) {\n"
    "        preferences.edit { putBoolean(LegacyPreferencesContract.trustedDeviceKey(listenerId), true) }\n"
    "        _uiState.value = _uiState.value.copy(\n"
    "            approvedListeners = _uiState.value.approvedListeners.map {\n"
    "                if (it.deviceId == listenerId) it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER) else it\n"
    "            },\n"
    "            lastMessage = \"Trusted listener ${listenerId.take(6)}\",\n"
    "        )\n"
    "        refreshHostDiagnostics()\n"
    "    }",
    "    fun trustListener(listenerId: String) {\n"
    "        val displayName = _uiState.value.approvedListeners\n"
    "            .firstOrNull { it.deviceId == listenerId }\n"
    "            ?.displayName\n"
    "            ?: listenerId\n"
    "        viewModelScope.launch {\n"
    "            persistTrustedListener(listenerId, displayName)\n"
    "        }\n"
    "    }\n\n"
    "    private suspend fun persistTrustedListener(\n"
    "        listenerId: String,\n"
    "        displayName: String,\n"
    "    ): Boolean {\n"
    "        if (!requirePersistenceReady(\"trust a listener\")) return false\n"
    "        return runCatching {\n"
    "            domainStore.trustDevice(listenerId, displayName)\n"
    "        }.fold(\n"
    "            onSuccess = {\n"
    "                _uiState.value = _uiState.value.copy(\n"
    "                    approvedListeners = _uiState.value.approvedListeners.map {\n"
    "                        if (it.deviceId == listenerId) {\n"
    "                            it.copy(trustState = TrustState.TRUSTED_PLACEHOLDER)\n"
    "                        } else {\n"
    "                            it\n"
    "                        }\n"
    "                    },\n"
    "                    lastMessage = \"Trusted listener ${listenerId.take(6)}\",\n"
    "                    lastError = null,\n"
    "                )\n"
    "                refreshHostDiagnostics()\n"
    "                true\n"
    "            },\n"
    "            onFailure = { error ->\n"
    "                val message = error.message ?: \"Failed to persist trusted listener\"\n"
    "                logger.e(\"storage.trust\", message, error)\n"
    "                _uiState.value = _uiState.value.copy(lastError = message)\n"
    "                refreshHostDiagnostics()\n"
    "                false\n"
    "            },\n"
    "        )\n"
    "    }",
    "trust listener replacement",
)
replace_once(
    "    fun adjustTuning(field: TuningField, direction: Int) {\n"
    "        val updated = _uiState.value.tuningSettings.adjust(field, direction)\n"
    "        persistTuningSettings(updated)\n"
    "        listenerSyncController = _uiState.value.selectedSession?.let { createSyncController(SessionId(it.id)) }\n"
    "        _uiState.value = _uiState.value.copy(\n"
    "            tuningSettings = updated,\n"
    "            lastMessage = \"Updated tuning: ${updated.summary()}\",\n"
    "            lastError = null,\n"
    "        )\n"
    "        if (resyncJob?.isActive == true) {\n"
    "            startPeriodicListenerResync()\n"
    "        }\n"
    "    }",
    "    fun adjustTuning(field: TuningField, direction: Int) {\n"
    "        if (!requirePersistenceReady(\"update tuning\")) return\n"
    "        val updated = _uiState.value.tuningSettings.adjust(field, direction)\n"
    "        viewModelScope.launch {\n"
    "            runCatching {\n"
    "                domainStore.saveTuning(updated.toRustStoredSettings())\n"
    "            }.onSuccess {\n"
    "                listenerSyncController = _uiState.value.selectedSession\n"
    "                    ?.let { createSyncController(SessionId(it.id)) }\n"
    "                _uiState.value = _uiState.value.copy(\n"
    "                    tuningSettings = updated,\n"
    "                    lastMessage = \"Updated tuning: ${updated.summary()}\",\n"
    "                    lastError = null,\n"
    "                )\n"
    "                if (resyncJob?.isActive == true) {\n"
    "                    startPeriodicListenerResync()\n"
    "                }\n"
    "            }.onFailure { error ->\n"
    "                val message = error.message ?: \"Failed to persist tuning settings\"\n"
    "                logger.e(\"storage.settings\", message, error)\n"
    "                _uiState.value = _uiState.value.copy(lastError = message)\n"
    "            }\n"
    "        }\n"
    "    }",
    "tuning persistence replacement",
)
replace_once(
    "        if (message.trustedForFuture) {\n"
    "            preferences.edit { putBoolean(\"trusted:${message.listenerId}\", true) }\n"
    "        }",
    "        if (message.trustedForFuture) {\n"
    "            viewModelScope.launch {\n"
    "                persistTrustedListener(message.listenerId, \"This Android Listener\")\n"
    "            }\n"
    "        }",
    "listener approval trust replacement",
)
old_helpers = '''    private fun loadTuningSettings(): TuningSettings = TuningSettings(
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
new_helpers = '''    private fun initializeDomainPersistence() {
        viewModelScope.launch {
            runCatching {
                domainStore.initialize()
            }.onSuccess { stored ->
                persistenceReady = true
                val tuning = stored.toAppTuningSettings()
                _uiState.value = _uiState.value.copy(
                    tuningSettings = tuning,
                    lastMessage = "Persistent settings loaded",
                    lastError = null,
                )
            }.onFailure { error ->
                persistenceReady = false
                val message = error.message ?: "Rust domain persistence failed to initialize"
                logger.e("storage.initialize", message, error)
                _uiState.value = _uiState.value.copy(
                    lastError = "Persistent storage unavailable: $message",
                )
            }
        }
    }

    private fun requirePersistenceReady(action: String): Boolean {
        if (persistenceReady) return true
        val message = "Persistent storage is not ready; cannot $action."
        _uiState.value = _uiState.value.copy(lastError = message)
        return false
    }
'''
replace_once(old_helpers, new_helpers, "legacy tuning helpers")
replace_once(
    "    override fun onCleared() {\n"
    "        bleService.stop()\n"
    "        wifiDirectService.stop()\n"
    "        super.onCleared()\n"
    "    }\n"
    "}\n\n"
    "internal fun requireHostSessionForPlayback",
    "    override fun onCleared() {\n"
    "        bleService.stop()\n"
    "        wifiDirectService.stop()\n"
    "        runCatching {\n"
    "            runBlocking(Dispatchers.IO) { domainStore.close() }\n"
    "        }.onFailure { error ->\n"
    "            logger.e(\"storage.close\", error.message ?: \"Failed to close Rust database\", error)\n"
    "        }\n"
    "        super.onCleared()\n"
    "    }\n"
    "}\n\n"
    "private fun RustStoredTuningSettings.toAppTuningSettings(): TuningSettings = TuningSettings(\n"
    "    syncSampleWindow = syncSampleWindow,\n"
    "    syncCadenceMs = syncCadenceMs,\n"
    "    startupBufferMs = startupBufferMs,\n"
    "    latePacketThresholdMs = latePacketThresholdMs,\n"
    "    hardResyncThresholdMs = hardResyncThresholdMs,\n"
    "    syncDriftThresholdMs = syncDriftThresholdMs,\n"
    "    scanWindowMs = scanWindowMs,\n"
    ")\n\n"
    "private fun TuningSettings.toRustStoredSettings(): RustStoredTuningSettings =\n"
    "    RustStoredTuningSettings(\n"
    "        syncSampleWindow = syncSampleWindow,\n"
    "        syncCadenceMs = syncCadenceMs,\n"
    "        startupBufferMs = startupBufferMs,\n"
    "        latePacketThresholdMs = latePacketThresholdMs,\n"
    "        hardResyncThresholdMs = hardResyncThresholdMs,\n"
    "        syncDriftThresholdMs = syncDriftThresholdMs,\n"
    "        scanWindowMs = scanWindowMs,\n"
    "        updatedAtMs = System.currentTimeMillis(),\n"
    "    )\n\n"
    "internal fun requireHostSessionForPlayback",
    "close and settings mappings",
)

if "preferences." in text or "LegacyPreferencesContract" in text:
    raise SystemExit("direct legacy preference access remains in MainViewModel")

path.write_text(text)
