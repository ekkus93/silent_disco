# Silent Disco — Fix 4 Hardening TODO

Source reviewed: `silent_disco-master_2606281313.zip`  
Previous pass reviewed: `SILENT_DISCO_FIX3_TODO(1).md`

## Priority legend

- **P0**: correctness / silent failure / app can lie to user / real join or host control broken
- **P1**: important state-machine, diagnostics, transport, or UX correctness
- **P2**: test hardening, cleanup, future-proofing

## General implementation rules

1. Do **not** solve failures with broad `try/catch` that only logs.
2. Do **not** report success unless the production operation succeeded.
3. If a result object exists, consume it at the call site.
4. Treat zero recipients as not delivered, not as success.
5. Keep demo/local behavior behind `BuildConfig.DEBUG` and disclose it in the message.
6. Add tests against production helpers or ViewModel state, not copied local validators.

---

# P0 — Fix listener sync flow and remove host misuse of `manualResync()`

## [x] P0.1 Restore manual resync availability for initial sync states

**File:** `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`

Current problem: `canManualResync()` is too narrow. It only allows `SYNCING_CLOCK`, `BUFFERING`, and `PLAYING`, but `handleJoinApprovalMessage()` calls sync immediately after setting the listener to `APPROVED`. That makes the initial sync probe fail with a misleading error.

Replace `canManualResync()` with the broader production rule:

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

Acceptance:

- `APPROVED`, `CONNECTING`, `SYNCING_CLOCK`, `BUFFERING`, `PLAYING`, `RECONNECTING`, and `DESYNCED` allow manual resync when `selectedSession != null`.
- `IDLE`, `SCANNING`, `SESSION_SELECTED`, `DISCONNECTED`, and `ERROR` reject manual resync.

## [x] P0.2 Split listener sync request logic out of `manualResync()`

**File:** `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Current problem: `manualResync()` is both a UI action and internal join-sync implementation. That makes it easy for host code to call it incorrectly and for initial sync to be blocked by UI gating.

Add a private helper that sends a listener sync probe. Keep all real logic here. Let `manualResync()` become a thin user-action wrapper.

```kotlin
private fun requestListenerSyncProbe(source: String) {
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

    val transportConnected = wifiDirectService.snapshot.value.state == TransportConnectionState.CONNECTED
    if (transportConnected) {
        _uiState.value = _uiState.value.copy(
            listenerState = ListenerLifecycleState.SYNCING_CLOCK,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = ListenerLifecycleState.SYNCING_CLOCK,
                requested = true,
                approved = true,
                connected = true,
            ),
            lastMessage = "$source sync probe sent",
            lastError = null,
        )
        viewModelScope.launch {
            runCatching {
                wifiDirectService.sendSyncRequestToHost(request)
            }.onSuccess {
                _uiState.value = _uiState.value.copy(lastMessage = "$source sync probe sent", lastError = null)
            }.onFailure { error ->
                handleSyncFailure(error.message ?: "Failed to send sync probe")
            }
        }
        return
    }

    val isDemoSession = BuildConfig.DEBUG && session.id.startsWith("demo-session-")
    if (isDemoSession) {
        applySyncResponse(hostTimingService.createResponse(request))
        _uiState.value = _uiState.value.copy(
            lastMessage = "$source sync applied locally for demo session",
            lastError = null,
        )
        return
    }

    val message = "Manual resync requires an active host connection"
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateListener { it.copy(lastError = message) }
    refreshListenerDiagnostics()
}
```

Then replace `manualResync()` with:

```kotlin
fun manualResync() {
    if (!_uiState.value.canManualResync()) {
        val message = "Join a session before requesting manual resync"
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
        return
    }
    requestListenerSyncProbe(source = "Manual resync")
}
```

Acceptance:

- Direct user manual resync still sets an error if there is no selected/active listener session.
- Real disconnected sessions do not fake success.
- Debug demo sessions may use local sync response and must disclose it.

## [x] P0.3 Use listener sync helper after join approval

**File:** `MainViewModel.kt`

In `handleJoinApprovalMessage()`, do not call `manualResync()` directly. Use the private helper so the initial join sync is not blocked by UI-facing manual-resync validation.

Replace:

```kotlin
manualResync()
```

With:

```kotlin
requestListenerSyncProbe(source = "Initial clock sync")
```

Acceptance:

- Listener approval starts clock sync from `APPROVED` or `CONNECTING` state.
- The initial sync path does not set `lastError = "Join a session before requesting manual resync"`.

## [x] P0.4 Remove host call path to listener `manualResync()`

**File:** `MainViewModel.kt`

Current problem: `startHostPlayback()` calls `startPeriodicResync()`, and `startPeriodicResync()` calls `manualResync()`. That is wrong because host playback is not a listener session and normally has no `selectedSession`.

Do one of these:

### Preferred minimum fix

Remove the call from host playback:

```kotlin
// In startHostPlayback(), remove this:
// startPeriodicResync()
```

Then delete `startPeriodicResync()` if it is no longer used, or rename it to `startPeriodicListenerResync()` and call it only from listener-side playback setup.

### If listener periodic resync is desired

Rename and restrict it:

```kotlin
private fun startPeriodicListenerResync() {
    resyncJob?.cancel()
    resyncJob = viewModelScope.launch {
        while (isActive) {
            delay(5_000)
            if (_uiState.value.canManualResync()) {
                requestListenerSyncProbe(source = "Periodic listener resync")
            }
        }
    }
}
```

Call it only from listener approval/playback flow, never from host playback.

Acceptance:

- Host playback never calls `manualResync()`.
- Host playback never emits listener error `Join a session before requesting manual resync`.
- Listener periodic resync, if kept, only runs when listener state allows it.

---

# P0 — Consume `SendAllResult` everywhere it matters

## [x] P0.5 Add host broadcast delivery helper

**File:** `MainViewModel.kt`

Add a single helper to surface zero-peer and partial-delivery results consistently.

```kotlin
private fun reportHostBroadcastDelivery(
    action: String,
    result: SendAllResult,
    requireAnyPeer: Boolean = true,
): Boolean {
    if (result.peerCount == 0) {
        val message = "$action was not delivered: no connected listeners"
        logger.w("transport.control", message)
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateHost {
            it.copy(
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshHostDiagnostics()
        return !requireAnyPeer
    }

    if (result.failureCount > 0) {
        val message = "$action delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
        logger.w("transport.control", message)
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateHost {
            it.copy(
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshHostDiagnostics()
        return false
    }

    return true
}
```

Acceptance:

- The helper is used by host control and sync broadcast callers.
- Zero peers and partial sends update host diagnostics immediately.

## [x] P0.6 Use delivery helper in `approveJoinRequest()` and move success state after delivery

**File:** `MainViewModel.kt`

Current problem: approval errors are handled only when an exception is thrown. `SendAllResult` can say zero peers or partial failure, but the UI may still claim approval.

In `approveJoinRequest()`, send approval first, inspect the result, and only then remove pending request / add approved listener.

Shape:

```kotlin
fun approveJoinRequest(request: JoinRequest) {
    val sessionId = currentSessionId ?: run {
        _uiState.value = _uiState.value.copy(lastError = "No active host session")
        return
    }

    viewModelScope.launch {
        val delivered = runCatching {
            wifiDirectService.broadcastControl(
                ControlMessage.JoinApproval(
                    version = 1,
                    sessionId = sessionId,
                    listenerId = request.listenerId,
                    streamId = currentStreamId,
                    hostDevice = localDeviceDescriptor(),
                    approvedAtMs = SystemClock.elapsedRealtime(),
                ),
            )
        }.map { result ->
            reportHostBroadcastDelivery("send join approval", result, requireAnyPeer = true)
        }.getOrElse { error ->
            handleHostControlFailure("send join approval", error)
            false
        }

        if (!delivered) return@launch

        val approved = ListenerInfo(
            id = request.listenerId,
            name = request.listenerName,
            connectedAtMs = SystemClock.elapsedRealtime(),
            syncState = SyncState(),
        )
        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },
            approvedListeners = _uiState.value.approvedListeners + approved,
            lastMessage = "Approved ${request.listenerName}",
            lastError = null,
        )
        refreshHostDiagnostics()
    }
}
```

Adjust field names to match current `ControlMessage.JoinApproval` / `ListenerInfo` constructors.

Acceptance:

- Pending request is not removed if approval was delivered to nobody.
- Approved listener is not added if approval delivery failed.
- Approval delivery stats are visible on zero-peer/partial failure.

## [x] P0.7 Use delivery helper in rejection, pause, stop, stream-start, stream-stop, and end-session broadcasts

**File:** `MainViewModel.kt`

For every host-side call to `wifiDirectService.broadcastControl(...)`:

- handle thrown failure with `handleHostControlFailure(...)`;
- inspect successful `SendAllResult` with `reportHostBroadcastDelivery(...)`.

Example for pause:

```kotlin
viewModelScope.launch {
    runCatching {
        wifiDirectService.broadcastControl(
            ControlMessage.Pause(
                version = 1,
                sessionId = sessionId,
                streamId = streamId,
                hostPauseTimeMs = SystemClock.elapsedRealtime(),
            ),
        )
    }.onSuccess { result ->
        reportHostBroadcastDelivery("broadcast pause", result, requireAnyPeer = false)
    }.onFailure { error ->
        handleHostControlFailure("broadcast pause", error)
    }
}
```

Example for join rejection inside `handleJoinRequestMessage()`:

```kotlin
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
    }.onSuccess { result ->
        reportHostBroadcastDelivery("send join rejection", result, requireAnyPeer = true)
    }.onFailure { error ->
        handleHostControlFailure("send join rejection", error)
    }
}
```

Acceptance:

- Grep all `broadcastControl(` calls in `MainViewModel.kt`; every returned result is used or intentionally documented as safe to ignore.
- Join rejection send failure is not log-only.

## [x] P0.8 Use delivery helper in host sync-response broadcast

**File:** `MainViewModel.kt`

Current problem: answering sync probes can fail with only a log message.

Replace the sync-response failure shape with:

```kotlin
viewModelScope.launch {
    runCatching {
        wifiDirectService.broadcastSyncResponse(hostTimingService.createResponse(request))
    }.onSuccess { result ->
        reportHostBroadcastDelivery("broadcast sync response", result, requireAnyPeer = true)
    }.onFailure { error ->
        handleHostControlFailure("broadcast sync response", error)
    }
}
```

Acceptance:

- Host diagnostics show sync-response delivery failure.
- Zero-peer sync response is not considered successful delivery.

---

# P0 — Fix host audio broadcast truthfulness

## [x] P0.9 Disclose zero listeners and partial audio delivery immediately

**File:** `MainViewModel.kt`

Inside `startHostStreamingLoop()`, update the `broadcastAudio(packet)` handling.

Use this shape:

```kotlin
var consecutiveAudioSendFailures = 0
var zeroPeerBroadcastCount = 0
```

Broadcast handling:

```kotlin
runCatching {
    wifiDirectService.broadcastAudio(packet)
}.onSuccess { result ->
    when {
        result.peerCount == 0 -> {
            zeroPeerBroadcastCount += 1
            consecutiveAudioSendFailures = 0
            val message = "No connected listeners for audio broadcast"
            diagnosticsStore.updateHost {
                it.copy(
                    lastError = message,
                    metricsSummary = summarizeMetrics(),
                )
            }
            // Keep host preview alive. Zero peers is disclosed, not fatal.
            refreshHostDiagnostics()
        }

        result.failureCount > 0 -> {
            consecutiveAudioSendFailures += 1
            val message = "Audio packet delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
            logger.w("transport.audio", message)
            _uiState.value = _uiState.value.copy(lastError = message)
            diagnosticsStore.updateHost {
                it.copy(
                    lastError = message,
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshHostDiagnostics()
        }

        else -> {
            consecutiveAudioSendFailures = 0
            diagnosticsStore.updateHost { it.copy(lastError = null) }
        }
    }
}.onFailure { error ->
    consecutiveAudioSendFailures += 1
    val message = error.message ?: "Failed to send audio packet"
    logger.w("transport.audio", "Failed to send packet ${packet.sequenceNumber}: $message")
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics()
}

if (consecutiveAudioSendFailures >= 10) {
    val message = "Audio transport failed repeatedly; stream stopped"
    hostStreamJob?.cancel()
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.ERROR,
        hostPlaybackState = PlaybackState.ERROR,
        lastError = message,
    )
    diagnosticsStore.updateHost {
        it.copy(
            streamState = PlaybackState.ERROR,
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
    return@launch
}
```

Acceptance:

- `peerCount == 0` is disclosed but does not stop local host preview.
- Partial delivery updates host diagnostics immediately.
- Repeated failed/partial delivery stops the host stream at the threshold.

---

# P0 — Make Wi-Fi Direct startup result honest

## [x] P0.10 Check Wi-Fi Direct permission inside `startHost()` before returning `Started`

**File:** `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`

Current problem: `MainViewModel` checks permissions, but the service contract itself can still return `Started` before permission-dependent work fails.

Add this check before `startHostSockets()` / `recreateGroup()`:

```kotlin
if (!hasWifiDirectPermission()) {
    val message = "Missing nearby Wi-Fi permission"
    fail(message, retryable = true)
    return TransportOperationResult.failed(message)
}
```

`startHost()` should have this shape:

```kotlin
override fun startHost(session: SessionInfo): TransportOperationResult {
    activeSession = session
    pendingConnectSession = null
    hosting = true
    stopClientChannels()
    ensureReceiver()

    if (manager == null || channel == null) {
        val message = "Wi-Fi Direct manager unavailable on this device"
        fail(message, retryable = false)
        return TransportOperationResult.failed(message)
    }

    if (!hasWifiDirectPermission()) {
        val message = "Missing nearby Wi-Fi permission"
        fail(message, retryable = true)
        return TransportOperationResult.failed(message)
    }

    return runCatching {
        startHostSockets()
        recreateGroup()
        updateSnapshot(
            state = TransportConnectionState.ADVERTISING,
            peers = emptyList(),
            lastError = null,
            hostAddressHint = null,
        )
        TransportOperationResult.Started
    }.getOrElse { error ->
        val message = error.message ?: "Failed to start Wi-Fi Direct host"
        fail(message, retryable = true)
        TransportOperationResult.failed(message)
    }
}
```

Acceptance:

- Missing Wi-Fi permission returns `started = false`.
- Synchronous socket/group-start exceptions return `started = false`.

## [x] P0.11 Surface async Wi-Fi Direct host startup failures to host UI

**Files:**

- `WifiDirectTransportService.kt`
- `MainViewModel.kt`

If `createGroup()` or related async callbacks call `fail(...)`, make sure the failure is visible in host UI. If `fail(...)` already updates `snapshot.state = ERROR`, collect that state in `MainViewModel` and map it to host/listener state depending on current role.

Add or update transport snapshot observer logic:

```kotlin
private fun handleTransportSnapshot(snapshot: TransportSnapshot) {
    if (snapshot.state == TransportConnectionState.ERROR && snapshot.lastError != null) {
        val hosting = _uiState.value.hostState in setOf(
            HostLifecycleState.CREATING_SESSION,
            HostLifecycleState.WAITING_FOR_LISTENERS,
            HostLifecycleState.READY,
            HostLifecycleState.STREAMING,
        )
        if (hosting) {
            _uiState.value = _uiState.value.copy(
                hostState = HostLifecycleState.ERROR,
                hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {
                    PlaybackState.ERROR
                } else {
                    _uiState.value.hostPlaybackState
                },
                lastError = snapshot.lastError,
            )
            diagnosticsStore.updateHost {
                it.copy(lastError = snapshot.lastError, metricsSummary = summarizeMetrics())
            }
            refreshHostDiagnostics()
        }
    }
}
```

Wire it into the existing `wifiDirectService.snapshot` collection rather than creating duplicate collectors.

Acceptance:

- Async Wi-Fi Direct group creation failure produces host error state and diagnostics.
- If Host Control screen has already opened, it visibly shows the failure.

---

# P1 — Enforce join/session state rules in the ViewModel

## [x] P1.1 Guard `selectDiscoveredSession()` with `canSelectSession()`

**File:** `MainViewModel.kt`

Current problem: the UI disables illegal join choices, but the ViewModel still accepts them.

Add import if needed:

```kotlin
import com.ekkus.silentdisco.app.canSelectSession
```

Update method:

```kotlin
fun selectDiscoveredSession(session: SessionInfo) {
    if (!_uiState.value.canSelectSession(session)) {
        val message = "Finish or cancel the current join before joining another session."
        _uiState.value = _uiState.value.copy(lastError = message)
        diagnosticsStore.updateListener { it.copy(lastError = message) }
        refreshListenerDiagnostics()
        return
    }

    _uiState.value = _uiState.value.copy(
        selectedSession = session,
        listenerState = ListenerLifecycleState.SESSION_SELECTED,
        connectionProgress = _uiState.value.connectionProgress.copy(
            currentState = ListenerLifecycleState.SESSION_SELECTED,
            discovered = true,
            inviteCode = "",
        ),
        lastMessage = "Selected ${session.name}",
        lastError = null,
    )
}
```

Adjust fields to match the current implementation.

Acceptance:

- ViewModel rejects switching to another session during active join.
- Rejection sets visible listener error/diagnostic.

## [x] P1.2 Clear scan state in listener error/disconnect paths

**File:** `MainViewModel.kt`

`clearScanState()` exists. Use it in every listener path that can interrupt scanning/joining:

- `handleListenerConnectionFailure(...)`
- `handleListenerDisconnect(...)`
- BLE scan failure collector from P1.5
- any listener reset path besides `cancelJoin()` and `leaveSession()`

Example:

```kotlin
private fun handleListenerConnectionFailure(message: String) {
    clearScanState()
    playbackJob?.cancel()
    resyncJob?.cancel()
    playbackEngine.stop()
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.ERROR,
        listenerPlaybackState = PlaybackState.ERROR,
        connectionProgress = _uiState.value.connectionProgress.copy(
            buffered = false,
            playing = false,
        ),
        lastError = message,
    )
    diagnosticsStore.updateListener { it.copy(playbackState = PlaybackState.ERROR, lastError = message) }
    refreshListenerDiagnostics()
}
```

Acceptance:

- `isScanning` is false after listener error/disconnect.
- `scanJob` is cancelled after listener error/disconnect.

---

# P1 — Fix host playback identity and missing-session failure

## [x] P1.3 Make missing host session a host error with diagnostics

**File:** `MainViewModel.kt`

Current problem: starting playback without `currentSessionId` only sets `lastError`. It should be a host error because the app cannot play a production host stream without a session.

Replace missing-session handling with:

```kotlin
val sessionId = currentSessionId
if (sessionId == null) {
    val message = "Start a host session before starting playback"
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.ERROR,
        hostPlaybackState = PlaybackState.ERROR,
        lastError = message,
    )
    diagnosticsStore.updateHost {
        it.copy(
            streamState = PlaybackState.ERROR,
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
    return
}
```

Acceptance:

- Starting host playback with no active host session fails visibly.
- Host diagnostics show the failure.

## [x] P1.4 Store generated stream ID in `currentStreamId`

**File:** `MainViewModel.kt`

Current problem: a stream ID may be generated locally but not assigned to `currentStreamId`, so pause/stop/end stream commands may not reference the active stream.

Use:

```kotlin
val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}").also {
    currentStreamId = it
}
```

Acceptance:

- If host playback creates a stream ID, `currentStreamId` is non-null afterward.
- Pause/stop/end stream broadcasts use the same stream ID.

---

# P1 — Surface BLE async failures

## [x] P1.5 Add BLE async failure flow

**File:** `app/src/main/java/com/ekkus/silentdisco/core/transport/BleDiscoveryService.kt`

Add models:

```kotlin
enum class BleOperation {
    ADVERTISE,
    SCAN,
}

data class BleOperationFailure(
    val operation: BleOperation,
    val message: String,
)
```

Add flow fields:

```kotlin
private val _failures = MutableSharedFlow<BleOperationFailure>(extraBufferCapacity = 8)
val failures: SharedFlow<BleOperationFailure> = _failures.asSharedFlow()
```

In advertise callback:

```kotlin
override fun onStartFailure(errorCode: Int) {
    val message = "BLE advertise failed with code=$errorCode"
    logger.w("ble.advertise", message)
    _failures.tryEmit(BleOperationFailure(BleOperation.ADVERTISE, message))
}
```

In scan callback:

```kotlin
override fun onScanFailed(errorCode: Int) {
    val message = "BLE scan failed with code=$errorCode"
    logger.w("ble.scan", message)
    _discoveredSessions.value = emptyList()
    _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
}
```

Add imports:

```kotlin
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
```

Acceptance:

- Async scan/advertise failures are observable outside the BLE service.
- The callbacks do not only log.

## [x] P1.6 Collect BLE failures in `MainViewModel`

**File:** `MainViewModel.kt`

In initialization/observer setup, collect `bleService.failures`.

```kotlin
private fun observeBleFailures() {
    viewModelScope.launch {
        bleService.failures.collect { failure ->
            when (failure.operation) {
                BleOperation.SCAN -> handleBleScanFailure(failure.message)
                BleOperation.ADVERTISE -> handleBleAdvertiseFailure(failure.message)
            }
        }
    }
}
```

Add handlers:

```kotlin
private fun handleBleScanFailure(message: String) {
    clearScanState()
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.ERROR,
        isScanning = false,
        lastError = message,
    )
    diagnosticsStore.updateListener { it.copy(lastError = message) }
    refreshListenerDiagnostics()
}

private fun handleBleAdvertiseFailure(message: String) {
    val hosting = _uiState.value.hostState in setOf(
        HostLifecycleState.CREATING_SESSION,
        HostLifecycleState.WAITING_FOR_LISTENERS,
        HostLifecycleState.READY,
        HostLifecycleState.STREAMING,
    )
    if (!hosting) return

    wifiDirectService.stop()
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.ERROR,
        hostPlaybackState = if (_uiState.value.hostPlaybackState == PlaybackState.PLAYING) {
            PlaybackState.ERROR
        } else {
            _uiState.value.hostPlaybackState
        },
        lastError = message,
    )
    diagnosticsStore.updateHost {
        it.copy(lastError = message, metricsSummary = summarizeMetrics())
    }
    refreshHostDiagnostics()
}
```

Call `observeBleFailures()` from `init` or the existing observer setup.

Acceptance:

- BLE scan failure clears scan UI and listener diagnostics.
- BLE advertise failure during hosting visibly errors the host state.

---

# P1 — Preserve AudioTrack write error codes

## [x] P1.7 Do not coerce negative `AudioTrack.write()` results to zero

**File:** `app/src/main/java/com/ekkus/silentdisco/core/audio/PlaybackScheduling.kt`

Current problem: if `write()` does something like `.coerceAtLeast(0)`, it loses the actual negative platform error code.

Use:

```kotlin
val written = track.write(
    frame.packet.payload,
    0,
    frame.packet.payload.size,
    AudioTrack.WRITE_NON_BLOCKING,
)
if (written <= 0) {
    error("AudioTrack write failed with result=$written")
}
writeCount += 1
return written.toLong()
```

Acceptance:

- Negative AudioTrack error codes appear in the thrown error message.
- Tests expecting write-before-start failure still pass.

---

# P2 — Fix confusing copy and diagnostics polish

## [x] P2.1 Make invite-code field label dynamic

**File:** `app/src/main/java/com/ekkus/silentdisco/feature/host/HostSetupScreen.kt`

Current issue: the invite code field can still say something like `Optional invite code` even when invite-code mode requires it.

Use dynamic label/helper text:

```kotlin
val inviteCodeRequired = uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE
OutlinedTextField(
    value = uiState.hostForm.inviteCode,
    onValueChange = { onHostFormChanged(uiState.hostForm.copy(inviteCode = it)) },
    label = { Text(if (inviteCodeRequired) "Invite code" else "Optional invite code") },
    supportingText = {
        Text(
            if (inviteCodeRequired) {
                "Listeners must enter this invite code to request approval."
            } else {
                "Optional code shown to listeners."
            },
        )
    },
    isError = inviteCodeRequired && uiState.hostForm.inviteCode.isBlank(),
)
```

Adjust callback names to current screen signature.

Acceptance:

- UI does not call a required invite code optional.

## [x] P2.2 Clarify native diagnostics labels if needed

**File:** `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

Use exact labels:

```kotlin
Text("Playback output: Android AudioTrack")
Text("Native bridge status: ${OboeBridge.statusSummary()}")
Text("Native backend diagnostics: ${OboeBridge.backendSummary()}")
```

Acceptance:

- Diagnostics do not imply native Oboe output.

---

# P2 — Test hardening

## [x] P2.3 Add manual-resync availability tests

**File:** `app/src/test/java/com/ekkus/silentdisco/app/ManualResyncStateTest.kt`

Add tests against the real `AppUiState.canManualResync()` helper.

```kotlin
@Test
fun canManualResync_allowsInitialAndActiveJoinStates() {
    val selected = SessionInfo("s1", "Session", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
    val allowed = listOf(
        ListenerLifecycleState.APPROVED,
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.SYNCING_CLOCK,
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.PLAYING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DESYNCED,
    )
    allowed.forEach { state ->
        assertTrue(
            AppUiState(listenerState = state, selectedSession = selected).canManualResync(),
            "state=$state",
        )
    }
}

@Test
fun canManualResync_rejectsInactiveStates() {
    val selected = SessionInfo("s1", "Session", "Host", ApprovalMode.MANUAL, inviteCodeRequired = false)
    val rejected = listOf(
        ListenerLifecycleState.IDLE,
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
        ListenerLifecycleState.DISCONNECTED,
        ListenerLifecycleState.ERROR,
    )
    rejected.forEach { state ->
        assertFalse(
            AppUiState(listenerState = state, selectedSession = selected).canManualResync(),
            "state=$state",
        )
    }
}
```

## [x] P2.4 Add broadcast delivery helper tests

**File:** `app/src/test/java/com/ekkus/silentdisco/app/BroadcastDeliveryTest.kt`

If `reportHostBroadcastDelivery()` remains private, extract pure classification to a production helper:

```kotlin
enum class BroadcastDeliverySeverity {
    OK,
    ZERO_PEERS,
    PARTIAL_FAILURE,
}

data class BroadcastDeliveryReport(
    val severity: BroadcastDeliverySeverity,
    val message: String?,
)

fun classifyBroadcastDelivery(action: String, result: SendAllResult): BroadcastDeliveryReport = when {
    result.peerCount == 0 -> BroadcastDeliveryReport(
        BroadcastDeliverySeverity.ZERO_PEERS,
        "$action was not delivered: no connected listeners",
    )
    result.failureCount > 0 -> BroadcastDeliveryReport(
        BroadcastDeliverySeverity.PARTIAL_FAILURE,
        "$action delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed",
    )
    else -> BroadcastDeliveryReport(BroadcastDeliverySeverity.OK, null)
}
```

Tests:

```kotlin
@Test
fun classifyBroadcastDelivery_zeroPeersIsNotSuccess() {
    val report = classifyBroadcastDelivery(
        "broadcast pause",
        SendAllResult(peerCount = 0, successCount = 0, failureCount = 0),
    )
    assertEquals(BroadcastDeliverySeverity.ZERO_PEERS, report.severity)
}

@Test
fun classifyBroadcastDelivery_partialFailureIsWarning() {
    val report = classifyBroadcastDelivery(
        "broadcast pause",
        SendAllResult(peerCount = 3, successCount = 2, failureCount = 1),
    )
    assertEquals(BroadcastDeliverySeverity.PARTIAL_FAILURE, report.severity)
}

@Test
fun classifyBroadcastDelivery_allDeliveredIsOk() {
    val report = classifyBroadcastDelivery(
        "broadcast pause",
        SendAllResult(peerCount = 2, successCount = 2, failureCount = 0),
    )
    assertEquals(BroadcastDeliverySeverity.OK, report.severity)
}
```

## [x] P2.5 Add ViewModel-level session-selection guard test

**File:** `app/src/test/java/com/ekkus/silentdisco/app/SessionSelectionGuardTest.kt`

Required behavior:

- Given active join to session A, calling `selectDiscoveredSession(sessionB)` does not replace `selectedSession`.
- `lastError` explains that current join must be finished/cancelled.

If full `MainViewModel` is hard to construct, extract the guard decision into a production helper and test that helper, but prefer real ViewModel behavior.

## [x] P2.6 Add host playback identity tests

**File:** `app/src/test/java/com/ekkus/silentdisco/app/HostPlaybackIdentityTest.kt`

Required behavior:

- Starting playback without `currentSessionId` sets `hostState = ERROR`, `hostPlaybackState = ERROR`, and `lastError`.
- Starting playback with session but no stream ID assigns `currentStreamId` before pause/stop paths need it.

If private fields block direct testing, test via public methods and UI state or extract production helper.

## [x] P2.7 Add BLE async failure tests

**Files:**

- `app/src/test/java/com/ekkus/silentdisco/core/transport/BleDiscoveryServiceTest.kt`
- or ViewModel tests with fake BLE service

Required behavior:

- Scan async failure emits `BleOperationFailure(SCAN, ...)`.
- Advertise async failure emits `BleOperationFailure(ADVERTISE, ...)`.
- ViewModel collection maps scan failure to listener error and clears `isScanning`.
- ViewModel collection maps advertise failure during hosting to host error.

---

# Validation checklist before handoff

Run:

```bash
./gradlew test
./gradlew lintDebug
```

Manual/grep checks:

```bash
# Host playback must not call listener manual resync
 grep -R "startPeriodicResync\|manualResync()" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# Every host broadcast result must be consumed
 grep -R "broadcastControl\|broadcastSyncResponse\|broadcastAudio" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# No log-only failures for user-visible operations
 grep -R "onFailure.*logger\.\|logger\.w" app/src/main/java/com/ekkus/silentdisco -n

# No random session IDs in host playback
 grep -R "UUID.randomUUID" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
```

Manual app checks:

- Listener approval proceeds into initial clock sync without setting a manual-resync error.
- Host playback does not create listener manual-resync errors.
- Pause/stop/end-session failures are visible in host diagnostics.
- Approval is not shown as successful if delivered to zero peers.
- Audio broadcast with zero listeners is disclosed but does not kill host preview.
- Partial audio delivery updates host diagnostics immediately.
- Wi-Fi Direct missing permission blocks host startup.
- Wi-Fi Direct async group failure shows host error.
- Selecting a different session during active join is rejected in ViewModel.
- BLE scan failure clears scanning UI.
- BLE advertise failure during host startup/hosting shows host error.
