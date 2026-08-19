# Silent Disco — Code Review 2 Hardening TODO

Source review: `silent_disco-master_2606280240.zip`

Priorities:

- **P0**: correctness / silent failure / app can lie to user
- **P1**: important UX or state-machine correctness
- **P2**: polish, diagnostics quality, naming cleanup

General rule for this pass: **do not add a fallback unless the UI/diagnostics clearly disclose it.** Prefer explicit error states over silent returns.

---

## P0 — Fix scan lifecycle so the app cannot get stuck scanning forever

### P0.1 Add explicit scan state to `AppUiState`

File: `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`

Add `isScanning` and a production helper for join-in-progress gating.

```kotlin
data class AppUiState(
    val selectedRole: AppRole? = null,
    val permissions: List<PermissionState> = emptyList(),
    val hostForm: HostFormState = HostFormState(),
    val hostState: HostLifecycleState = HostLifecycleState.IDLE,
    val listenerState: ListenerLifecycleState = ListenerLifecycleState.IDLE,
    val connectionProgress: ConnectionProgressState = ConnectionProgressState(),
    val discoveredSessions: List<SessionInfo> = emptyList(),
    val selectedSession: SessionInfo? = null,
    val pendingJoinRequests: List<JoinRequest> = emptyList(),
    val approvedListeners: List<ListenerInfo> = emptyList(),
    val hostPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerPlaybackState: PlaybackState = PlaybackState.STOPPED,
    val listenerSyncState: SyncState = SyncState(),
    val tuningSettings: TuningSettings = TuningSettings(),
    val hostDiagnostics: HostDiagnosticsSnapshot = HostDiagnosticsSnapshot(),
    val listenerDiagnostics: ListenerDiagnosticsSnapshot = ListenerDiagnosticsSnapshot(),
    val localVolume: Float = 1.0f,
    val isScanning: Boolean = false,
    val lastMessage: String? = null,
    val lastError: String? = null,
)

fun AppUiState.isJoinInProgress(): Boolean = listenerState in setOf(
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.AWAITING_APPROVAL,
    ListenerLifecycleState.APPROVED,
    ListenerLifecycleState.CONNECTING,
    ListenerLifecycleState.SYNCING_CLOCK,
    ListenerLifecycleState.BUFFERING,
    ListenerLifecycleState.PLAYING,
    ListenerLifecycleState.RECONNECTING,
    ListenerLifecycleState.DESYNCED,
)

fun AppUiState.canSelectSession(session: SessionInfo): Boolean =
    !isJoinInProgress() || selectedSession?.id == session.id
```

### P0.2 Replace `DiscoverSessionsScreen` derived scanning state

File: `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`

Replace:

```kotlin
val isScanning = uiState.listenerState == ListenerLifecycleState.SCANNING
```

with:

```kotlin
val isScanning = uiState.isScanning
```

Remove the now-unused `ListenerLifecycleState` import if it becomes unused.

### P0.3 Disable Join buttons while another join is active

File: `DiscoverSessionsScreen.kt`

Import and use the helper:

```kotlin
import com.ekkus.silentdisco.app.canSelectSession
import com.ekkus.silentdisco.app.isJoinInProgress
```

Update each session card button:

```kotlin
val canJoinThisSession = uiState.canSelectSession(session)
Button(
    onClick = { onSelectSession(session) },
    enabled = canJoinThisSession,
) {
    Text("Join")
}
if (!canJoinThisSession && uiState.isJoinInProgress()) {
    Text(
        "Finish or cancel the current join before joining another session.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
```

### P0.4 Make `scanForSessions()` explicitly complete or fail

File: `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Add fields near the other jobs:

```kotlin
private var scanJob: Job? = null
private val scanWindowMs = 3_000L
```

Replace the current `scanForSessions()` with a job-backed implementation. This snippet assumes `bleService.startScanning()` is changed in P0.5 to return a result. If P0.5 is not done yet, temporarily wrap it with `runCatching`, but do not leave scan state stuck.

```kotlin
fun scanForSessions() {
    logger.i("listener.scan", "Scanning for nearby sessions")
    scanJob?.cancel()

    if (!hasListenerTransportPermissions()) {
        val message = "Missing nearby connectivity permissions for discovery"
        wifiDirectService.fail(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            isScanning = false,
            listenerState = ListenerLifecycleState.ERROR,
            lastError = message,
        )
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
        return
    }

    _uiState.value = _uiState.value.copy(
        isScanning = true,
        listenerState = ListenerLifecycleState.SCANNING,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.SCANNING,
        ),
        lastError = null,
        lastMessage = "Scanning for nearby sessions…",
    )

    scanJob = viewModelScope.launch {
        val bleStart = bleService.startScanning()
        if (!bleStart.started) {
            val message = bleStart.message ?: "BLE scan could not start"
            wifiDirectService.fail(message, retryable = true)
            _uiState.value = _uiState.value.copy(
                isScanning = false,
                listenerState = ListenerLifecycleState.ERROR,
                lastError = message,
            )
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return@launch
        }

        wifiDirectService.discoverPeers()
        delay(scanWindowMs)
        refreshDiscoveredSessions()

        val discovered = _uiState.value.discoveredSessions
        _uiState.value = _uiState.value.copy(
            isScanning = false,
            listenerState = if (_uiState.value.selectedSession == null) {
                ListenerLifecycleState.IDLE
            } else {
                ListenerLifecycleState.SESSION_SELECTED
            },
            discoveredSessions = discovered,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = if (discovered.isEmpty()) {
                    ListenerLifecycleState.IDLE
                } else {
                    ListenerLifecycleState.SESSION_SELECTED
                },
                discovered = discovered.isNotEmpty(),
            ),
            lastMessage = if (discovered.isEmpty()) "No nearby sessions found" else "Found ${discovered.size} session(s)",
            lastError = null,
        )
        diagnosticsStore.updateListener { it.copy(lastError = null) }
        refreshListenerDiagnostics()
    }
}
```

Also clear scan state in `leaveSession()`, `cancelJoin()`, and any listener-flow reset:

```kotlin
scanJob?.cancel()
_uiState.value = _uiState.value.copy(isScanning = false, ...)
```

### P0.5 Make BLE scan start return an explicit result

File: `app/src/main/java/com/ekkus/silentdisco/core/transport/BleDiscoveryService.kt`

Add:

```kotlin
data class BleOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        val Started = BleOperationResult(started = true)
        fun failed(message: String) = BleOperationResult(started = false, message = message)
    }
}
```

Change `startScanning()` from `Unit` to `BleOperationResult`:

```kotlin
@SuppressLint("MissingPermission")
fun startScanning(): BleOperationResult {
    if (!hasScanPermission()) {
        val message = "Missing Bluetooth scan permission"
        logger.w("ble.scan", message)
        _discoveredSessions.value = emptyList()
        return BleOperationResult.failed(message)
    }
    val scanner = scanner ?: run {
        val message = "BLE scanner unavailable on this device"
        logger.w("ble.scan", message)
        _discoveredSessions.value = emptyList()
        return BleOperationResult.failed(message)
    }
    stopScanning()
    seenSessions.clear()
    _discoveredSessions.value = emptyList()

    val callback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            // keep existing parsing logic
        }

        override fun onScanFailed(errorCode: Int) {
            logger.w("ble.scan", "BLE scan failed with code=$errorCode")
            _discoveredSessions.value = emptyList()
            // P1 follow-up: expose this async failure through StateFlow/SharedFlow.
        }
    }
    scanCallback = callback

    return runCatching {
        scanner.startScan(
            listOf(ScanFilter.Builder().setServiceUuid(BleAdvertisementCodec.serviceUuid).build()),
            ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build(),
            callback,
        )
    }.fold(
        onSuccess = {
            logger.i("ble.scan", "Started BLE scanning")
            BleOperationResult.Started
        },
        onFailure = { error ->
            val message = error.message ?: "BLE scan start failed"
            logger.w("ble.scan", message)
            BleOperationResult.failed(message)
        },
    )
}
```

Do not leave only `logger.w(...)` for failures that prevent scanning.

---

## P0 — Remove fake audio write success and wire volume to real playback

### P0.6 Rename or clarify `OboePlaybackEngine`

File: `app/src/main/java/com/ekkus/silentdisco/core/audio/PlaybackScheduling.kt`

Preferred: rename `OboePlaybackEngine` to `AudioTrackPlaybackEngine`. If renaming causes too much churn, at least add a KDoc comment explaining that this is currently AudioTrack-backed.

```kotlin
/**
 * Android AudioTrack-backed streaming playback engine.
 *
 * The native Oboe bridge currently exposes diagnostics only. Do not report this
 * as native Oboe playback unless audio output is actually routed through Oboe.
 */
class AudioTrackPlaybackEngine {
    // existing implementation, hardened below
}
```

Update imports/usages in `MainViewModel` and tests accordingly. If you keep the old class name for this pass, still apply P0.7/P0.8.

### P0.7 Make write-before-start fail, not fake success

File: `PlaybackScheduling.kt`

Replace `write()` with this shape:

```kotlin
fun write(frame: PlaybackFrame): Long {
    val track = audioTrack ?: error("Playback engine is not started")
    val written = track.write(
        frame.packet.payload,
        0,
        frame.packet.payload.size,
        AudioTrack.WRITE_NON_BLOCKING,
    )
    if (written <= 0) {
        error("AudioTrack write failed with result=$written")
    }
    if (written < frame.packet.payload.size) {
        // Count it and expose through diagnostics in P1. For now, do not pretend it was full success.
        writeCount += 1
        return written.toLong()
    }
    writeCount += 1
    return written.toLong()
}
```

Then update playback loops to catch this and transition to error.

Example helper in `MainViewModel.kt`:

```kotlin
private fun handlePlaybackEngineFailure(error: Throwable) {
    val message = error.message ?: "Playback engine failed"
    logger.e("playback.engine", message, error)
    playbackJob?.cancel()
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.ERROR,
        listenerPlaybackState = PlaybackState.ERROR,
        lastError = message,
    )
    diagnosticsStore.updateListener {
        it.copy(
            playbackState = PlaybackState.ERROR,
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshListenerDiagnostics()
}
```

Use it around `playbackEngine.write(frame)` in both listener playback loops. For host preview playback inside `startHostStreamingLoop()`, use a separate host error helper or update host diagnostics.

```kotlin
runCatching {
    playbackEngine.write(frame)
}.onFailure { error ->
    handlePlaybackEngineFailure(error)
    return@launch
}
```

### P0.8 Add real playback volume support

File: `PlaybackScheduling.kt`

Add a stored volume field and setter:

```kotlin
private var volume: Float = 1.0f

fun setVolume(value: Float) {
    volume = value.coerceIn(0f, 1f)
    audioTrack?.setVolume(volume)
}
```

Apply it when creating a new `AudioTrack`:

```kotlin
audioTrack = AudioTrack.Builder()
    // existing builder calls
    .build()
    .also {
        it.setVolume(volume)
        it.play()
    }
```

File: `MainViewModel.kt`

Update `setLocalVolume()`:

```kotlin
fun setLocalVolume(volume: Float) {
    val normalized = volume.coerceIn(0f, 1f)
    playbackEngine.setVolume(normalized)
    _uiState.value = _uiState.value.copy(localVolume = normalized)
}
```

Add/adjust tests so a volume update is not only a UI-state test. If direct `AudioTrack` testing is hard, extract a tiny interface:

```kotlin
interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}
```

Then inject a fake into `MainViewModel` tests.

---

## P0 — Enforce invite codes in the real host join path

### P0.9 Validate real `JoinRequest` messages against `hostForm.inviteCode`

File: `MainViewModel.kt`

Add a helper:

```kotlin
private fun joinRejectionReason(message: ControlMessage.JoinRequest): String? {
    if (message.sessionId != currentSessionId) return "Session mismatch"
    val form = _uiState.value.hostForm
    if (form.approvalMode == ApprovalMode.INVITE_CODE) {
        val expected = form.inviteCode.trim()
        val actual = message.inviteCode?.trim().orEmpty()
        if (expected.isBlank()) return "Host invite code is not configured"
        if (actual != expected) return "Incorrect invite code"
    }
    return null
}
```

Update `handleJoinRequestMessage()`:

```kotlin
private fun handleJoinRequestMessage(message: ControlMessage.JoinRequest) {
    if (message.sessionId != currentSessionId) return

    val rejectionReason = joinRejectionReason(message)
    if (rejectionReason != null) {
        logger.w("listener.join.reject", rejectionReason)
        viewModelScope.launch {
            runCatching {
                wifiDirectService.broadcastControl(
                    ControlMessage.JoinRejection(
                        version = 1,
                        sessionId = message.sessionId,
                        listenerId = message.device.deviceId,
                        reason = rejectionReason,
                    ),
                )
            }.onFailure { error ->
                logger.w("listener.join.reject", "Failed to send rejection: ${error.message}")
            }
        }
        diagnosticsStore.updateHost {
            it.copy(lastError = "Rejected ${message.device.displayName}: $rejectionReason")
        }
        refreshHostDiagnostics()
        return
    }

    val request = JoinRequest(
        requestId = "${message.device.deviceId}-${message.sessionId.value}",
        sessionId = message.sessionId.value,
        listenerId = message.device.deviceId,
        listenerName = message.device.displayName,
        inviteCode = message.inviteCode,
        requestedAtMs = SystemClock.elapsedRealtime(),
    )
    if (_uiState.value.pendingJoinRequests.any { it.listenerId == request.listenerId }) return
    _uiState.value = _uiState.value.copy(
        pendingJoinRequests = _uiState.value.pendingJoinRequests + request,
        hostState = HostLifecycleState.READY,
        lastMessage = "${request.listenerName} requested to join",
        lastError = null,
    )
    refreshHostDiagnostics()
}
```

### P0.10 Remove hardcoded invite code semantics from non-debug paths

File: `MainViewModel.kt`

The demo path uses `"1234"`. Keep it only if the session is explicitly a demo session and ideally only in debug builds.

```kotlin
val shouldSimulate = BuildConfig.DEBUG && session.id.startsWith("demo-session-")
if (shouldSimulate) {
    val expectedDemoCode = "1234"
    val shouldReject = session.inviteCodeRequired && request.inviteCode != expectedDemoCode
    simulateApprovalAndPlayback(session.id, shouldReject)
    return
}
```

Add `import com.ekkus.silentdisco.BuildConfig` if needed.

---

## P1 — Finish incomplete Code Review 1 UI TODOs

### P1.1 Fix raw enum display in Diagnostics

File: `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

Replace:

```kotlin
Text("Playback state: ${uiState.listenerDiagnostics.playbackState}")
```

with:

```kotlin
Text("Playback state: ${uiState.listenerDiagnostics.playbackState.label()}")
```

Add a regression test if there is a reasonable text-building helper. At minimum, grep should not find this pattern in UI code:

```bash
grep -R "Playback state: .*playbackState}" app/src/main/java
```

### P1.2 Add buffering step to `ConnectionProgressState`

File: `AppState.kt`

Add field:

```kotlin
data class ConnectionProgressState(
    val currentState: ListenerLifecycleState = ListenerLifecycleState.IDLE,
    val discovered: Boolean = false,
    val requested: Boolean = false,
    val approved: Boolean = false,
    val connected: Boolean = false,
    val synced: Boolean = false,
    val buffered: Boolean = false,
    val playing: Boolean = false,
    val inviteCode: String = "",
)
```

Add helper:

```kotlin
fun ConnectionProgressState.bufferingStep(): StepState = when {
    buffered -> StepState.Done
    synced -> StepState.Active
    else -> StepState.Pending
}

fun ConnectionProgressState.playingStep(): StepState = when {
    playing -> StepState.Done
    buffered -> StepState.Active
    else -> StepState.Pending
}
```

Update existing `playingStep()` so it depends on `buffered`, not directly on `synced`.

### P1.3 Render the buffering step

File: `JoinProgressScreen.kt`

Import:

```kotlin
import com.ekkus.silentdisco.app.bufferingStep
```

Update step rows:

```kotlin
ConnectionStepRow("Discovering session", progress.discoveredStep())
ConnectionStepRow("Sending join request", progress.requestedStep())
ConnectionStepRow("Awaiting host approval", progress.approvedStep())
ConnectionStepRow("Connecting transport", progress.connectedStep())
ConnectionStepRow("Syncing clock", progress.syncedStep())
ConnectionStepRow("Buffering audio", progress.bufferingStep())
ConnectionStepRow("Playing", progress.playingStep())
```

### P1.4 Update buffering/playback transitions

Files: `MainViewModel.kt`

When a stream starts:

```kotlin
connectionProgress = _uiState.value.connectionProgress.copy(
    currentState = ListenerLifecycleState.BUFFERING,
    approved = true,
    connected = true,
    synced = _uiState.value.listenerSyncState.confidence != SyncQualityBadge.UNKNOWN,
    buffered = false,
    playing = false,
)
```

When scheduler can start and playback begins:

```kotlin
connectionProgress = _uiState.value.connectionProgress.copy(
    currentState = ListenerLifecycleState.PLAYING,
    connected = true,
    approved = true,
    synced = true,
    buffered = true,
    playing = true,
)
```

When stopping/leaving/erroring:

```kotlin
connectionProgress = _uiState.value.connectionProgress.copy(
    playing = false,
    buffered = false,
)
```

For full resets, keep using `ConnectionProgressState()`.

### P1.5 Add tests for the buffering step

File: `app/src/test/java/com/ekkus/silentdisco/app/ConnectionProgressStepTest.kt`

Add tests:

```kotlin
@Test
fun bufferingStep_pendingBeforeSync() {
    val p = ConnectionProgressState(synced = false, buffered = false)
    assertEquals(StepState.Pending, p.bufferingStep())
}

@Test
fun bufferingStep_activeAfterSyncBeforeBufferReady() {
    val p = ConnectionProgressState(synced = true, buffered = false)
    assertEquals(StepState.Active, p.bufferingStep())
}

@Test
fun bufferingStep_doneWhenBuffered() {
    val p = ConnectionProgressState(buffered = true)
    assertEquals(StepState.Done, p.bufferingStep())
}

@Test
fun playingStep_activeOnlyAfterBuffering() {
    val p = ConnectionProgressState(buffered = true, playing = false)
    assertEquals(StepState.Active, p.playingStep())
}
```

---

## P1 — Make host startup loading state real

### P1.6 Set `CREATING_SESSION` before side effects

File: `MainViewModel.kt`

In `createHostSession()`, after validation and before generating/starting the session:

```kotlin
_uiState.value = _uiState.value.copy(
    hostState = HostLifecycleState.CREATING_SESSION,
    lastError = null,
    lastMessage = "Starting host session…",
)
```

Keep validation before this state so invalid form fields do not briefly show startup.

### P1.7 Validate invite-code form before hosting

File: `MainViewModel.kt`

Add this in `createHostSession()` before side effects:

```kotlin
if (form.approvalMode == ApprovalMode.INVITE_CODE && form.inviteCode.isBlank()) {
    _uiState.value = _uiState.value.copy(lastError = "Invite code is required for invite-code hosting")
    return false
}
```

File: `HostSetupScreen.kt`

Disable Start Hosting if invite-code mode has blank code:

```kotlin
val inviteCodeMissing = uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE &&
    uiState.hostForm.inviteCode.isBlank()
val canStart = uiState.hostForm.sessionName.isNotBlank() &&
    uiState.hostForm.selectedAudio != null &&
    !inviteCodeMissing &&
    !isStarting
```

Add helper text:

```kotlin
if (inviteCodeMissing) {
    Text(
        "Enter an invite code or choose a different approval mode.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
    )
}
```

### P1.8 Make BLE advertising start return a result

File: `BleDiscoveryService.kt`

Change `startAdvertising()` to return `BleOperationResult` too.

```kotlin
@SuppressLint("MissingPermission")
fun startAdvertising(advertisement: BleAdvertisement): BleOperationResult {
    _advertisement.value = advertisement
    stopAdvertising()
    if (!hasAdvertisePermission()) {
        val message = "Missing Bluetooth advertise permission"
        logger.w("ble.advertise", message)
        return BleOperationResult.failed(message)
    }
    val advertiser = advertiser ?: run {
        val message = "BLE advertiser unavailable on this device"
        logger.w("ble.advertise", message)
        return BleOperationResult.failed(message)
    }
    val serviceData = BleAdvertisementCodec.encode(advertisement)
    val callback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
            logger.i("ble.advertise", "Advertising session ${advertisement.sessionId.take(8)}")
        }

        override fun onStartFailure(errorCode: Int) {
            logger.w("ble.advertise", "BLE advertise failed with code=$errorCode")
            // P1 follow-up: expose this async failure through StateFlow/SharedFlow.
        }
    }
    advertiseCallback = callback
    return runCatching {
        advertiser.startAdvertising(
            AdvertiseSettings.Builder()
                .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
                .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
                .setConnectable(false)
                .build(),
            AdvertiseData.Builder()
                .addServiceUuid(BleAdvertisementCodec.serviceUuid)
                .build(),
            AdvertiseData.Builder()
                .setIncludeDeviceName(true)
                .addServiceData(BleAdvertisementCodec.serviceUuid, serviceData)
                .build(),
            callback,
        )
    }.fold(
        onSuccess = { BleOperationResult.Started },
        onFailure = { error ->
            val message = error.message ?: "BLE advertise start failed"
            logger.w("ble.advertise", message)
            BleOperationResult.failed(message)
        },
    )
}
```

### P1.9 Use BLE advertising result in `createHostSession()`

File: `MainViewModel.kt`

Inside `runCatching` or before it, do not ignore BLE start result:

```kotlin
val advertiseResult = bleService.startAdvertising(...)
if (!advertiseResult.started) {
    error(advertiseResult.message ?: "BLE advertising could not start")
}
wifiDirectService.startHost(session)
```

If Wi-Fi Direct start can also fail asynchronously via callbacks, at minimum ensure synchronous exceptions and immediate service failure states prevent navigation.

---

## P1 — Manual resync should never silently no-op

### P1.10 Add a helper for manual resync availability

File: `AppState.kt`

```kotlin
fun AppUiState.canManualResync(): Boolean =
    selectedSession != null && listenerState !in setOf(
        ListenerLifecycleState.IDLE,
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
        ListenerLifecycleState.DISCONNECTED,
        ListenerLifecycleState.ERROR,
    )
```

### P1.11 Disable Manual Resync and show reason

File: `DiagnosticsScreen.kt`

Import:

```kotlin
import com.ekkus.silentdisco.app.canManualResync
```

Update button:

```kotlin
val canManualResync = uiState.canManualResync()
Button(
    onClick = onManualResync,
    enabled = canManualResync,
) {
    Icon(Icons.Filled.Sync, contentDescription = null, modifier = Modifier.size(ButtonDefaults.IconSize))
    Spacer(Modifier.size(ButtonDefaults.IconSpacing))
    Text("Manual Resync")
}
if (!canManualResync) {
    Text(
        "Join a session before requesting manual resync.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
```

### P1.12 Make `manualResync()` set an error if called anyway

File: `MainViewModel.kt`

Replace the silent `?: return` path:

```kotlin
fun manualResync() {
    val session = _uiState.value.selectedSession
    if (session == null) {
        val message = "Join a session before requesting manual resync"
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
        return
    }

    val controller = listenerSyncController ?: createSyncController(SessionId(session.id)).also {
        listenerSyncController = it
    }
    val request = controller.newProbe()
    pendingSyncCorrelationId = request.correlationId

    if (wifiDirectService.snapshot.value.state == TransportConnectionState.CONNECTED) {
        viewModelScope.launch {
            runCatching {
                wifiDirectService.sendSyncRequestToHost(request)
            }.onSuccess {
                _uiState.value = _uiState.value.copy(lastMessage = "Manual resync probe sent", lastError = null)
            }.onFailure { error ->
                handleSyncFailure(error.message ?: "Failed to send sync probe")
            }
        }
        return
    }

    applySyncResponse(hostTimingService.createResponse(request))
    _uiState.value = _uiState.value.copy(lastMessage = "Manual resync applied locally", lastError = null)
}
```

---

## P1 — Stop hiding repeated host stream send failures

### P1.13 Make `sendAll()` report delivery stats

File: `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`

Add:

```kotlin
data class SendAllResult(
    val peerCount: Int,
    val successCount: Int,
    val failureCount: Int,
)
```

Change `sendAll()`:

```kotlin
suspend fun sendAll(message: T): SendAllResult {
    val snapshot = peers.values.toList()
    var success = 0
    var failure = 0
    snapshot.forEach { peer ->
        runCatching { peer.send(message) }
            .onSuccess { success += 1 }
            .onFailure { failure += 1 }
    }
    return SendAllResult(
        peerCount = snapshot.size,
        successCount = success,
        failureCount = failure,
    )
}
```

Update callers in `WifiDirectTransportService` to return the result or use it internally.

### P1.14 Surface broadcast failures in host streaming loop

File: `MainViewModel.kt`

Do not only log packet-send failures. Track consecutive failures.

```kotlin
var consecutiveAudioSendFailures = 0
```

Around broadcast:

```kotlin
runCatching {
    wifiDirectService.broadcastAudio(packet)
}.onSuccess {
    consecutiveAudioSendFailures = 0
}.onFailure { error ->
    consecutiveAudioSendFailures += 1
    val message = error.message ?: "Failed to send audio packet"
    logger.w("transport.audio", "Failed to send packet ${packet.sequenceNumber}: $message")
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics()
    if (consecutiveAudioSendFailures >= 10) {
        _uiState.value = _uiState.value.copy(
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = PlaybackState.ERROR,
            lastError = "Audio transport failed repeatedly; stream stopped",
        )
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
        return@launch
    }
}
```

If zero connected peers is normal for host preview, count it separately as `zeroPeerBroadcastCount` or display `0 connected listeners`; do not count it as successful delivery.

---

## P1 — Strengthen tests so they verify production behavior

### P1.15 Replace tautological UI-state tests with helper tests

File: `app/src/test/java/com/ekkus/silentdisco/app/UiStateValidationTest.kt`

Avoid tests that duplicate constants and assert the same expression. Test production helpers:

```kotlin
@Test
fun isJoinInProgress_trueForActiveJoinStates() {
    val activeStates = listOf(
        ListenerLifecycleState.JOIN_REQUESTED,
        ListenerLifecycleState.AWAITING_APPROVAL,
        ListenerLifecycleState.APPROVED,
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.SYNCING_CLOCK,
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.PLAYING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DESYNCED,
    )
    activeStates.forEach { state ->
        assertTrue(AppUiState(listenerState = state).isJoinInProgress(), "state=$state")
    }
}

@Test
fun canSelectSession_falseForDifferentSessionDuringJoin() {
    val selected = SessionInfo("a", "A", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
    val other = SessionInfo("b", "B", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
    val state = AppUiState(
        listenerState = ListenerLifecycleState.CONNECTING,
        selectedSession = selected,
    )
    assertFalse(state.canSelectSession(other))
    assertTrue(state.canSelectSession(selected))
}
```

### P1.16 Add invite-code host validation tests

Expose `joinRejectionReason()` as `internal` if needed and test it directly, or test through a small `JoinRequestValidator` object.

Suggested small object:

```kotlin
object JoinRequestValidator {
    fun rejectionReason(
        mode: ApprovalMode,
        expectedInviteCode: String,
        actualInviteCode: String?,
    ): String? {
        if (mode != ApprovalMode.INVITE_CODE) return null
        val expected = expectedInviteCode.trim()
        val actual = actualInviteCode?.trim().orEmpty()
        if (expected.isBlank()) return "Host invite code is not configured"
        if (actual != expected) return "Incorrect invite code"
        return null
    }
}
```

Tests:

```kotlin
@Test
fun inviteCodeModeRejectsMissingCode() {
    assertEquals(
        "Incorrect invite code",
        JoinRequestValidator.rejectionReason(ApprovalMode.INVITE_CODE, "2468", null),
    )
}

@Test
fun inviteCodeModeAcceptsMatchingCodeIgnoringOuterWhitespace() {
    assertNull(
        JoinRequestValidator.rejectionReason(ApprovalMode.INVITE_CODE, " 2468 ", "2468"),
    )
}

@Test
fun manualModeDoesNotRequireInviteCode() {
    assertNull(
        JoinRequestValidator.rejectionReason(ApprovalMode.MANUAL, "", null),
    )
}
```

### P1.17 Add playback engine tests for no fake success

File: `app/src/test/java/com/ekkus/silentdisco/core/audio/OboePlaybackEngineTest.kt` or renamed engine test

Update old tests that expected fake success. New expected behavior:

```kotlin
@Test
fun writeBeforeStartThrows() {
    val engine = AudioTrackPlaybackEngine()
    assertFailsWith<IllegalStateException> {
        engine.write(frame(payloadSize = 3840))
    }
}
```

If JVM unit tests cannot instantiate Android `AudioTrack`, isolate the state check into a fakeable wrapper or use Robolectric/instrumented tests. Do not keep the old fake-success assertion.

---

## P2 — Improve diagnostics honesty and naming

### P2.1 Make audio diagnostics explicitly distinguish native bridge and playback engine

File: `DiagnosticsScreen.kt`

Replace vague strings with explicit labels:

```kotlin
Text("Playback output: Android AudioTrack")
Text("Native bridge: ${OboeBridge.statusSummary()}")
```

If Oboe is not actually used for playback, do not display `"Oboe + AudioTrack"` as if Oboe is part of the output path.

### P2.2 Replace swallowed native bridge load with explicit availability state

File: `OboeBridge.kt`

```kotlin
object OboeBridge {
    private val loadResult: Result<Unit> = runCatching {
        System.loadLibrary("silentdisco")
    }

    val isAvailable: Boolean
        get() = loadResult.isSuccess

    external fun nativeGetAudioBackend(): String
    external fun nativeGetAudioStatus(): String

    fun backendSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioBackend() }.getOrDefault("Native backend query failed")
    } else {
        "Native bridge unavailable"
    }

    fun statusSummary(): String = if (isAvailable) {
        runCatching { nativeGetAudioStatus() }.getOrDefault("Native status query failed")
    } else {
        "Native library not loaded"
    }
}
```

This is acceptable because diagnostics are allowed to report unavailable status. Do not use this pattern for real playback success.

### P2.3 Audit `?: return` in user-triggered functions

Run:

```bash
grep -R "?: return" app/src/main/java/com/ekkus/silentdisco -n
```

For each match, classify it:

- OK: cleanup/idempotent/internal event ignore
- Not OK: user action silently does nothing

Fix every Not OK case by setting `lastError`, disabling the UI action, or both.

### P2.4 Audit log-only failures

Run:

```bash
grep -R "onFailure.*logger\|logger\.w" app/src/main/java/com/ekkus/silentdisco -n
```

For each failure that affects user-visible state, update `lastError` and diagnostics. Logs alone are not enough for user-triggered operations.

---

## Validation checklist

Before handing back the code, Claude Code should run or manually verify:

- [ ] `./gradlew test` passes.
- [ ] `./gradlew lintDebug` passes, or every failure is listed with reason and fix plan.
- [x] Discover screen scan indicator stops after scan window.
- [x] Scan button is re-enabled after scan success, empty scan, permission failure, and scan start failure.
- [x] Join buttons are disabled while another join is active.
- [x] Join progress shows all seven steps, including "Buffering audio".
- [x] Diagnostics shows no raw enum names.
- [x] Manual Resync disabled without active listener session.
- [x] Manual Resync called directly without session sets `lastError`.
- [x] Host setup shows real `CREATING_SESSION` loading state.
- [x] Host setup does not navigate to Host Control on BLE/Wi-Fi start failure.
- [x] Invite-code mode rejects wrong/missing real join request codes.
- [x] The hardcoded demo invite code is debug/demo-only.
- [x] Volume slider changes actual playback engine volume.
- [x] Playback write before start does not report success.
- [x] Playback engine write failures transition to visible error state.
- [x] BLE scan/advertise synchronous failures update UI/diagnostics.
- [x] Repeated audio broadcast failures do not silently continue forever.

Software-only validation was re-audited during the 2026-08-19 non-device
closure pass. The checked behavior items above are backed by current
production helpers/state transitions and their regression tests; the two
Gradle execution gates remain open because this sandbox cannot resolve the
Gradle distribution and are not inferred from source inspection.

## Notes for Claude Code

Do not solve this by adding broad `try/catch` wrappers that suppress failures. The point of this pass is to remove silent failures, not hide them deeper.

Do not add new placeholder text that sounds like a real capability. If something is simulated, debug-only, or not fully implemented, say so in diagnostics or gate it behind `BuildConfig.DEBUG`.

Do not mark this done just because unit tests pass. Several existing tests are weak; add production-helper tests and real state-transition tests as listed above.
