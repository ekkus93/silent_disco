# Silent Disco — Fix 5 Hardening TODO

Source reviewed: `silent_disco-master_2606281350.zip`
Previous pass reviewed: `SILENT_DISCO_FIX4_TODO(1).md`

## Priority legend

- **P0**: correctness / false success / broken real join or playback state
- **P1**: important diagnostics, state-machine cleanup, and ViewModel enforcement
- **P2**: test hardening and validation cleanup

## General implementation rules

1. Do **not** solve failures with broad `try/catch` that only logs.
2. Do **not** report success unless the real operation succeeded.
3. If a method returns `SendAllResult`, consume it or document why it is intentionally irrelevant.
4. Treat zero recipients as not delivered, not as success.
5. Keep demo/local behavior behind `BuildConfig.DEBUG` and disclose it in `lastMessage` or diagnostics.
6. Do not add tests that only copy production constants or duplicate production validation locally.

---

# P0 — Fix periodic resync corrupting active listener state

## P0.1 Make `requestListenerSyncProbe()` preserve active playback state

**File:** `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Current problem: `requestListenerSyncProbe(source)` always sets `listenerState = SYNCING_CLOCK` when transport is connected. That is correct for initial sync, but wrong for periodic/manual sync while already `PLAYING`, `BUFFERING`, `DESYNCED`, or `RECONNECTING`.

Update the connected branch so it only enters `SYNCING_CLOCK` for initial sync states.

Use this shape:

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
        val currentState = _uiState.value.listenerState
        val shouldEnterSyncingState = currentState in setOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
        )
        val nextState = if (shouldEnterSyncingState) {
            ListenerLifecycleState.SYNCING_CLOCK
        } else {
            currentState
        }
        val nextProgressState = if (shouldEnterSyncingState) {
            ListenerLifecycleState.SYNCING_CLOCK
        } else {
            _uiState.value.connectionProgress.currentState
        }

        _uiState.value = _uiState.value.copy(
            listenerState = nextState,
            connectionProgress = _uiState.value.connectionProgress.copy(
                currentState = nextProgressState,
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

Acceptance:

- Initial sync from `APPROVED`, `CONNECTING`, or `SYNCING_CLOCK` still shows `SYNCING_CLOCK`.
- Periodic/manual resync from `BUFFERING`, `PLAYING`, `RECONNECTING`, or `DESYNCED` preserves the current listener state.
- Periodic sync while playing does not make the Join Progress UI regress to clock-sync.

## P0.2 Add tests for sync-state preservation

**File:** `app/src/test/java/com/ekkus/silentdisco/app/ManualResyncStateTest.kt` or a new production-helper test file.

If `requestListenerSyncProbe()` is hard to test directly, extract a small production helper:

```kotlin
internal fun nextStateForSyncProbe(currentState: ListenerLifecycleState): ListenerLifecycleState =
    if (currentState in setOf(
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.SYNCING_CLOCK,
        )
    ) {
        ListenerLifecycleState.SYNCING_CLOCK
    } else {
        currentState
    }
```

Tests:

```kotlin
@Test
fun syncProbe_entersSyncingForInitialStates() {
    listOf(
        ListenerLifecycleState.APPROVED,
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.SYNCING_CLOCK,
    ).forEach { state ->
        assertEquals(ListenerLifecycleState.SYNCING_CLOCK, nextStateForSyncProbe(state), "state=$state")
    }
}

@Test
fun syncProbe_preservesActivePlaybackStates() {
    listOf(
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.PLAYING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DESYNCED,
    ).forEach { state ->
        assertEquals(state, nextStateForSyncProbe(state), "state=$state")
    }
}
```

Acceptance:

- Tests cover the production helper, not copied test-only logic.

---

# P0 — Map listener transport failure to listener error

## P0.3 Handle listener-side failed transport snapshots

**File:** `MainViewModel.kt`

Current problem: host-side transport failure is mapped to host error, but listener-side join/connect failure can leave the listener stuck in `CONNECTING` with only `lastError` set.

Find the existing transport snapshot collection/handler. Extend it so `TransportConnectionState.FAILED` or equivalent failed/error state is mapped by role.

Use this shape, adapting enum names to the current model:

```kotlin
private fun handleTransportSnapshot(snapshot: TransportSnapshot) {
    val errorMessage = snapshot.lastError?.message ?: snapshot.lastError?.toString()
    val isFailed = snapshot.state == TransportConnectionState.FAILED ||
        snapshot.state == TransportConnectionState.ERROR
    if (!isFailed || errorMessage.isNullOrBlank()) return

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
            lastError = errorMessage,
        )
        diagnosticsStore.updateHost {
            it.copy(lastError = errorMessage, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
        return
    }

    val listenerActiveOrJoining = _uiState.value.listenerState in setOf(
        ListenerLifecycleState.SCANNING,
        ListenerLifecycleState.SESSION_SELECTED,
        ListenerLifecycleState.JOIN_REQUESTED,
        ListenerLifecycleState.CONNECTING,
        ListenerLifecycleState.AWAITING_APPROVAL,
        ListenerLifecycleState.APPROVED,
        ListenerLifecycleState.SYNCING_CLOCK,
        ListenerLifecycleState.BUFFERING,
        ListenerLifecycleState.PLAYING,
        ListenerLifecycleState.RECONNECTING,
        ListenerLifecycleState.DESYNCED,
    )

    if (listenerActiveOrJoining) {
        pendingJoinRequestMessage = null
        handleListenerConnectionFailure(errorMessage)
    }
}
```

Wire this into the existing `wifiDirectService.snapshot` collection. Do **not** create a duplicate competing collector if one already exists.

Acceptance:

- Listener transport failure during connect/join transitions listener to `ERROR`.
- `pendingJoinRequestMessage` is cleared.
- `isScanning` is false.
- `connectionProgress.buffered` and `connectionProgress.playing` are false.
- Host transport failure behavior from Fix 4 is preserved.

## P0.4 Add listener transport failure test

**File:** `app/src/test/java/com/ekkus/silentdisco/app/TransportFailureStateTest.kt`

Preferred: ViewModel-level test with a fake Wi-Fi Direct service that can emit a failed snapshot.

Required behavior:

```kotlin
@Test
fun listenerTransportFailureDuringConnectTransitionsToError() = runTest {
    // Arrange a ViewModel in listener CONNECTING state with selectedSession and pending join.
    // Emit TransportSnapshot(state = FAILED, lastError = "Connection failed").
    // Assert listenerState == ERROR.
    // Assert listenerPlaybackState == ERROR.
    // Assert lastError contains "Connection failed".
    // Assert connectionProgress.playing == false.
    // Assert connectionProgress.buffered == false.
}
```

If full ViewModel construction is still too expensive, extract a production helper that classifies a failed transport snapshot as `HOST_FAILURE`, `LISTENER_FAILURE`, or `IGNORE`, and test that helper. Prefer real ViewModel state if practical.

---

# P0 — Make rejection delivery truthful

## P0.5 Move `rejectJoinRequest()` state mutation after delivery succeeds

**File:** `MainViewModel.kt`

Current problem: `rejectJoinRequest()` removes the pending request and says the listener was rejected before the rejection message is delivered.

Replace the method shape with delivery-first logic:

```kotlin
fun rejectJoinRequest(request: JoinRequest) {
    logger.w("approval.reject", "Rejecting ${request.listenerName}")
    viewModelScope.launch {
        val delivered = runCatching {
            wifiDirectService.broadcastControl(
                ControlMessage.JoinRejection(
                    version = 1,
                    sessionId = SessionId(request.sessionId),
                    listenerId = request.listenerId,
                    reason = "Host rejected ${request.listenerName}",
                ),
            )
        }.map { result ->
            reportHostBroadcastDelivery("send join rejection", result, requireAnyPeer = true)
        }.getOrElse { error ->
            handleHostControlFailure("send join rejection", error)
            false
        }

        if (!delivered) return@launch

        _uiState.value = _uiState.value.copy(
            pendingJoinRequests = _uiState.value.pendingJoinRequests.filterNot { it.requestId == request.requestId },
            lastMessage = "Rejected ${request.listenerName}",
            lastError = null,
        )
        diagnosticsStore.updateHost {
            it.copy(lastError = null, metricsSummary = summarizeMetrics())
        }
        refreshHostDiagnostics()
    }
}
```

Acceptance:

- Pending request is not removed if rejection delivery fails or reaches zero peers.
- Host does not show “Rejected X” unless rejection delivery succeeded.
- Delivery failure remains visible in host diagnostics.

## P0.6 Make invite-code rejection diagnostics delivery-aware

**File:** `MainViewModel.kt`

Current problem: wrong invite-code rejection path sends `JoinRejection`, but updates host diagnostics as if the listener was rejected even if delivery failed.

Inside `handleJoinRequestMessage(...)`, in the `rejectionReason != null` branch, move the “Rejected X” diagnostics update inside successful delivery handling.

Use this shape:

```kotlin
if (rejectionReason != null) {
    logger.w("listener.join.reject", rejectionReason)
    viewModelScope.launch {
        val delivered = runCatching {
            wifiDirectService.broadcastControl(
                ControlMessage.JoinRejection(
                    version = 1,
                    sessionId = message.sessionId,
                    listenerId = message.device.deviceId,
                    reason = rejectionReason,
                ),
            )
        }.map { result ->
            reportHostBroadcastDelivery("send join rejection", result, requireAnyPeer = true)
        }.getOrElse { error ->
            handleHostControlFailure("send join rejection", error)
            false
        }

        if (delivered) {
            val hostMessage = "Rejected ${message.device.displayName}: $rejectionReason"
            diagnosticsStore.updateHost {
                it.copy(lastError = hostMessage, metricsSummary = summarizeMetrics())
            }
            _uiState.value = _uiState.value.copy(lastError = hostMessage)
            refreshHostDiagnostics()
        }
    }
    return
}
```

Acceptance:

- Wrong invite-code rejection delivery failure does not get overwritten by “Rejected X”.
- If rejection delivery succeeds, host diagnostics clearly show the rejection reason.

## P0.7 Add rejection delivery tests

**File:** `app/src/test/java/com/ekkus/silentdisco/app/JoinRejectionDeliveryTest.kt`

Required behavior:

- Given a pending request and fake transport returning `SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)`, `rejectJoinRequest()` does not remove the pending request.
- Given fake transport returning full success, `rejectJoinRequest()` removes the pending request and sets `lastMessage = "Rejected ..."`.
- Wrong invite code rejection failure leaves host diagnostics with the delivery failure, not a false rejection-delivered message.

If ViewModel test setup is too heavy, extract production decision helper(s), but do not use copied local validators.

---

# P1 — Make host control delivery messaging honest

## P1.1 Add a helper for local-control result messages

**File:** `MainViewModel.kt`

Pause/stop/end-session can legitimately change local host state even with zero listeners, but the message must disclose delivery problems.

Add helper:

```kotlin
private fun hostControlDeliveryMessage(
    localActionPastTense: String,
    deliveryAction: String,
    result: SendAllResult,
): String? {
    val report = classifyBroadcastDelivery(deliveryAction, result)
    return when (report.severity) {
        BroadcastDeliverySeverity.OK -> null
        BroadcastDeliverySeverity.ZERO_PEERS -> "$localActionPastTense locally; no connected listeners received the command"
        BroadcastDeliverySeverity.PARTIAL_FAILURE -> "$localActionPastTense locally; ${report.message}"
    }
}
```

Acceptance:

- Local host state may pause/stop/end locally.
- Delivery failure warning is not hidden behind a success-only message.

## P1.2 Use the local-control helper in pause/stop/end-session paths

**File:** `MainViewModel.kt`

Example pause pattern:

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
        val warning = hostControlDeliveryMessage(
            localActionPastTense = "Paused",
            deliveryAction = "broadcast pause",
            result = result,
        )
        if (warning != null) {
            _uiState.value = _uiState.value.copy(lastError = warning)
            diagnosticsStore.updateHost { it.copy(lastError = warning, metricsSummary = summarizeMetrics()) }
            refreshHostDiagnostics()
        } else {
            diagnosticsStore.updateHost { it.copy(lastError = null, metricsSummary = summarizeMetrics()) }
            refreshHostDiagnostics()
        }
    }.onFailure { error ->
        handleHostControlFailure("broadcast pause", error)
    }
}
```

Apply equivalent handling to:

- `pauseHostPlayback()`
- `stopHostPlayback()`
- `endSession()`
- any stream-start/stream-stop path where local state changes before delivery is checked

Acceptance:

- Zero peers does not appear as full network success.
- Partial send failure remains visible in host diagnostics.
- Success messages do not immediately clear a delivery warning.

---

# P1 — Disclose zero-listener audio broadcast in UI state

## P1.3 Set `_uiState.lastError` on zero-listener audio broadcast

**File:** `MainViewModel.kt`

Current problem: zero-listener audio broadcast updates host diagnostics but may not update global UI error/message state.

Inside `startHostStreamingLoop()`, in the `result.peerCount == 0` branch, add `_uiState.lastError`:

```kotlin
result.peerCount == 0 -> {
    zeroPeerBroadcastCount += 1
    consecutiveAudioSendFailures = 0
    val message = "No connected listeners for audio broadcast"
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics()
}
```

Acceptance:

- Host sees zero-listener audio broadcast outside diagnostics too.
- Host preview is not stopped for zero peers.

## P1.4 Remove or expose `zeroPeerBroadcastCount`

**File:** `MainViewModel.kt`

Current problem: `zeroPeerBroadcastCount` is incremented but unused.

Choose one:

1. Remove it entirely, or
2. Include it in `summarizeMetrics()` / host diagnostics.

Minimum cleanup:

```kotlin
// Remove:
var zeroPeerBroadcastCount = 0
zeroPeerBroadcastCount += 1
```

Acceptance:

- No unused local counter remains.
- If retained, it appears in diagnostics/metrics.

---

# P1 — Clear stale listener progress flags

## P1.5 Clear progress flags in `handleSyncFailure()`

**File:** `MainViewModel.kt`

Update `handleSyncFailure(message)` so it clears progress flags:

```kotlin
private fun handleSyncFailure(message: String) {
    logger.w("sync.listener", message)
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.ERROR,
        listenerPlaybackState = PlaybackState.ERROR,
        connectionProgress = _uiState.value.connectionProgress.copy(
            buffered = false,
            playing = false,
        ),
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

Acceptance:

- Sync failure cannot leave “Buffering audio” or “Playing” marked active/done.

## P1.6 Clear progress flags in `handleListenerDisconnect()`

**File:** `MainViewModel.kt`

Update disconnect handler:

```kotlin
private fun handleListenerDisconnect(reason: String) {
    clearScanState()
    playbackJob?.cancel()
    resyncJob?.cancel()
    playbackEngine.stop()
    listenerScheduler = null
    pendingTransportPackets.clear()
    pendingSyncCorrelationId = null
    pendingJoinRequestMessage = null

    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.DISCONNECTED,
        listenerPlaybackState = PlaybackState.STOPPED,
        connectionProgress = _uiState.value.connectionProgress.copy(
            buffered = false,
            playing = false,
        ),
        lastError = reason,
    )
    diagnosticsStore.updateListener {
        it.copy(playbackState = PlaybackState.STOPPED, lastError = reason, metricsSummary = summarizeMetrics())
    }
    refreshListenerDiagnostics()
}
```

Adapt names/parameters to current implementation.

Acceptance:

- Disconnect cannot leave progress flags stale.
- Scan state is cleared.

## P1.7 Verify listener error handlers clear progress flags

**File:** `MainViewModel.kt`

Inspect and update these paths:

- `handleListenerConnectionFailure(...)`
- `handleListenerPlaybackEngineFailure(...)`
- `handleSyncFailure(...)`
- `handleListenerDisconnect(...)`
- `leaveSession()`
- `cancelJoin()` if it can leave an active progress object

Acceptance:

- Grep/review confirms every listener terminal path clears or resets `connectionProgress.buffered` and `connectionProgress.playing`.

---

# P1 — Replace demo simulation `manualResync()` call

## P1.8 Use internal sync helper in demo simulation

**File:** `MainViewModel.kt`

Current problem: demo simulation still calls public `manualResync()`. It is listener-side, so it is not the old host misuse, but it is still the wrong abstraction and confuses validation grep.

Replace:

```kotlin
manualResync()
```

inside demo/simulation approval playback flow with:

```kotlin
requestListenerSyncProbe(source = "Demo clock sync")
```

Acceptance:

- `manualResync()` is only called from UI/user-action paths.
- Demo local sync remains debug-gated and disclosed.

---

# P2 — Replace tautological tests with production behavior tests

## P2.1 Replace copied host validation test with production validation coverage

**File:** `app/src/test/java/com/ekkus/silentdisco/app/HostStartupValidationTest.kt`

Current issue: if the test defines a local `createValidator()` duplicating production logic, it does not verify production behavior.

Preferred fix:

- Extract production validation into a small helper if needed.

Example production helper:

```kotlin
internal object HostSessionValidator {
    fun validate(form: HostFormState): String? = when {
        form.sessionName.isBlank() -> "Session name is required"
        form.selectedAudio == null -> "Choose an audio file before hosting"
        form.approvalMode == ApprovalMode.INVITE_CODE && form.inviteCode.isBlank() ->
            "Invite code is required for invite-code hosting"
        else -> null
    }
}
```

Then `MainViewModel.validateHostForm()` should delegate to this helper, and tests should call `HostSessionValidator.validate(...)`.

Acceptance:

- No local copied validator remains in the test file.
- Tests fail if production validation changes incorrectly.

## P2.2 Replace copied host playback identity test with production helper/ViewModel test

**File:** `app/src/test/java/com/ekkus/silentdisco/app/HostPlaybackIdentityTest.kt`

Current issue: tests may just copy `AppUiState` into the expected error state.

Preferred production helper:

```kotlin
internal fun requireHostSessionForPlayback(currentSessionId: SessionId?): String? =
    if (currentSessionId == null) "Start a host session before starting playback" else null

internal fun resolveStreamId(currentStreamId: StreamId?, nowMs: Long): Pair<StreamId, Boolean> =
    currentStreamId?.let { it to false } ?: (StreamId("stream-$nowMs") to true)
```

Then use these helpers inside `startHostPlayback()` and test them. If feasible, also add a ViewModel test that calls `startHostPlayback()` without a session and checks UI state.

Acceptance:

- Tests cover production helper or real ViewModel behavior.
- Tests do not merely assert state values they just constructed.

## P2.3 Replace BLE async fake-flow test with production-facing test

**Files:**

- `app/src/test/java/com/ekkus/silentdisco/core/transport/BleDiscoveryServiceTest.kt`
- or `app/src/test/java/com/ekkus/silentdisco/app/BleFailureViewModelTest.kt`

Current issue: testing a standalone `MutableSharedFlow` does not prove `BleDiscoveryService` callbacks or `MainViewModel` mapping work.

Options:

1. Extract internal callback handlers in `BleDiscoveryService`:

```kotlin
internal fun emitAdvertiseFailureForTest(errorCode: Int) {
    val message = "BLE advertise failed with code=$errorCode"
    _failures.tryEmit(BleOperationFailure(BleOperation.ADVERTISE, message))
}

internal fun emitScanFailureForTest(errorCode: Int) {
    val message = "BLE scan failed with code=$errorCode"
    _discoveredSessions.value = emptyList()
    _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
}
```

Then test the real service flow.

2. Preferably, use a fake BLE service interface if one exists and test ViewModel mapping:

- Scan failure -> `listenerState = ERROR`, `isScanning = false`, diagnostics lastError set.
- Advertise failure during hosting -> `hostState = ERROR`, host diagnostics lastError set.

Acceptance:

- At least one test exercises production BLE failure plumbing or ViewModel mapping.
- No test claims BLE async handling is covered by only testing a separate flow object.

## P2.4 Add ViewModel session-selection guard test

**File:** `app/src/test/java/com/ekkus/silentdisco/app/SessionSelectionGuardTest.kt`

Current helper tests are useful but incomplete. Add a ViewModel-level test if feasible:

Required behavior:

```kotlin
@Test
fun selectDiscoveredSession_doesNotSwitchSessionsDuringActiveJoin() = runTest {
    // Arrange ViewModel with selectedSession = sessionA and listenerState = CONNECTING.
    // Act: viewModel.selectDiscoveredSession(sessionB).
    // Assert: selectedSession remains sessionA.
    // Assert: lastError says finish/cancel current join.
}
```

If ViewModel construction is too heavy, extract the exact selection decision into a production helper and test that helper. But do not rely only on Compose UI disabled state.

---

# Validation checklist before handoff

Run:

```bash
./gradlew test
./gradlew lintDebug
```

Grep/manual checks:

```bash
# Demo/internal code should not use user-facing manualResync wrapper
grep -R "manualResync()" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# Every broadcast result should be consumed or documented
grep -R "broadcastControl\|broadcastSyncResponse\|broadcastAudio" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# Failure handlers should not be log-only for user-visible operations
grep -R "onFailure.*logger\.\|logger\.w" app/src/main/java/com/ekkus/silentdisco -n

# Host playback should not invent session IDs
grep -R "UUID.randomUUID" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
```

Manual app checks:

- Listener approval enters initial clock sync without a manual-resync error.
- Periodic listener resync while playing does not change UI state back to `SYNCING_CLOCK`.
- Listener transport failure during connection becomes visible listener `ERROR`.
- Rejecting a listener is not shown as successful if the rejection reaches zero peers.
- Wrong invite-code rejection delivery failure is visible as delivery failure, not false rejection success.
- Pause/stop/end-session delivery problems remain visible after local state changes.
- Zero-listener audio broadcast appears in host UI state/diagnostics and does not kill host preview.
- Sync failure and disconnect clear `buffered` and `playing` progress flags.
- Demo simulation uses `requestListenerSyncProbe("Demo clock sync")` rather than `manualResync()`.
