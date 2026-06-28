# Silent Disco — Fix 3 Hardening TODO

Source reviewed: `silent_disco-master_2606281224.zip`  
Previous baseline: `SILENT_DISCO_CODE_REVIEW2_TODO(1).md`

## Priority legend

- **P0**: correctness / silent failure / app can lie to user / crash after success state
- **P1**: important state-machine, diagnostics, transport, or UX correctness
- **P2**: polish, naming cleanup, test hardening, future-proofing

## General implementation rules

1. Do **not** solve failures with broad `try/catch` that only logs.
2. Do **not** report success unless the production operation succeeded.
3. If something is local-only, demo-only, debug-only, simulated, or diagnostic-only, say so in the UI/diagnostics or gate it behind `BuildConfig.DEBUG`.
4. Keep helpers in production code and test those helpers. Avoid tests that duplicate constants and prove nothing.
5. Prefer small explicit result types over methods that call `fail(...)` internally and return `Unit`.

---

# P0 — Fix playback engine crash/silent failure after `write()` hardening

## P0.1 Rename or wrap `OboePlaybackEngine` as `AudioTrackPlaybackEngine`

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/core/audio/PlaybackScheduling.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- tests under `app/src/test/java/com/ekkus/silentdisco/core/audio/`

Current problem: `PlaybackScheduling.kt` still defines `class OboePlaybackEngine`, but the implementation is Android `AudioTrack` output. This is misleading and makes diagnostics dishonest.

Preferred low-churn change:

```kotlin
/**
 * Android AudioTrack-backed streaming playback engine.
 *
 * The native bridge currently exposes diagnostics only. Do not describe this as
 * native Oboe playback unless PCM output is actually routed through Oboe.
 */
class AudioTrackPlaybackEngine {
    private var audioTrack: AudioTrack? = null
    private var sampleRate: Int = 48_000
    private var writeCount: Long = 0
    private var volume: Float = 1.0f

    fun start(format: AudioFormatSpec = AudioFormatSpec()): String {
        sampleRate = format.sampleRate
        if (audioTrack == null) {
            val channelMask = if (format.channelCount == 1) {
                AudioFormat.CHANNEL_OUT_MONO
            } else {
                AudioFormat.CHANNEL_OUT_STEREO
            }
            val minBufferSize = AudioTrack.getMinBufferSize(
                format.sampleRate,
                channelMask,
                AudioFormat.ENCODING_PCM_16BIT,
            ).coerceAtLeast(format.sampleRate / 5 * format.bytesPerFrame)
            audioTrack = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(format.sampleRate)
                        .setChannelMask(channelMask)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(minBufferSize)
                .build()
                .also {
                    it.setVolume(volume)
                    it.play()
                }
        }
        return "Android AudioTrack"
    }

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
        writeCount += 1
        return written.toLong()
    }

    fun setVolume(value: Float) {
        volume = value.coerceIn(0f, 1f)
        audioTrack?.setVolume(volume)
    }

    fun playbackPositionMs(frame: PlaybackFrame): Long {
        val headPosition = audioTrack?.playbackHeadPosition?.toLong() ?: return frame.localDeadlineMs
        return (headPosition * 1_000L) / sampleRate
    }

    fun stop() {
        audioTrack?.pause()
        audioTrack?.flush()
        audioTrack?.release()
        audioTrack = null
    }
}

@Deprecated("Use AudioTrackPlaybackEngine; native Oboe is diagnostics-only for now.")
typealias OboePlaybackEngine = AudioTrackPlaybackEngine
```

Acceptance:

- `MainViewModel` imports/uses `AudioTrackPlaybackEngine` directly, or the typealias is used only temporarily.
- `start()` returns `Android AudioTrack`, not `Oboe + AudioTrack`.
- Diagnostics no longer implies Oboe output.

## P0.2 Add playback failure helpers in `MainViewModel`

**File:** `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Add helpers near the existing error helpers.

```kotlin
private fun handleListenerPlaybackEngineFailure(error: Throwable) {
    val message = error.message ?: "Playback engine failed"
    logger.e("playback.listener", message, error)
    playbackJob?.cancel()
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

private fun handleHostPlaybackEngineFailure(error: Throwable) {
    val message = error.message ?: "Host playback engine failed"
    logger.e("playback.host", message, error)
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
}
```

Acceptance:

- Playback write/start exceptions never kill a coroutine without updating UI and diagnostics.
- Listener error uses listener diagnostics.
- Host error uses host diagnostics.

## P0.3 Start the playback engine before listener simulation writes

**File:** `MainViewModel.kt`

Current problem: `startListenerPlaybackSimulation()` calls `playbackEngine.write(frame)` but never starts the playback engine. After Code Review 2 changed write-before-start to throw, this loop can crash.

Inside `startListenerPlaybackSimulation(sessionId)`, after packets are selected and before the playback job starts, determine the format and start the engine.

Use the real packet format if available. If generated packets do not carry a format, use the app default.

```kotlin
val playbackFormat = latestDecodedAudio?.format ?: AudioFormatSpec()
val backend = runCatching {
    playbackEngine.start(playbackFormat)
}.getOrElse { error ->
    handleListenerPlaybackEngineFailure(error)
    return
}
logger.i("playback.listener", "Started listener simulation playback via $backend")
```

Then replace the direct write inside the loop:

```kotlin
val frame = listenerScheduler?.poll() ?: break
runCatching {
    playbackEngine.write(frame)
}.onFailure { error ->
    handleListenerPlaybackEngineFailure(error)
    return@launch
}
```

Acceptance:

- Simulation path starts the engine before first write.
- Write failure transitions to listener error state.
- No bare `playbackEngine.write(frame)` remains in listener simulation.

## P0.4 Start the playback engine before real transport listener writes

**File:** `MainViewModel.kt`

Current problem: `startTransportListenerPlayback()` sets UI to `PLAYING` and then calls `playbackEngine.write(frame)` directly. It also does not set `buffered = true` when playback starts.

Add a format parameter to `startTransportListenerPlayback()` so it can start the engine with the stream format.

Change function signature:

```kotlin
private fun startTransportListenerPlayback(
    sessionId: SessionId,
    streamId: StreamId,
    format: AudioFormatSpec,
) {
```

Update the call site in the stream-start handler. Wherever `ControlMessage.StreamStart` is handled, create:

```kotlin
val streamFormat = AudioFormatSpec(
    sampleRate = message.sampleRate,
    channelCount = message.channels,
)
startTransportListenerPlayback(message.sessionId, message.streamId, streamFormat)
```

Inside `startTransportListenerPlayback()`, before creating the playback job:

```kotlin
val backend = runCatching {
    playbackEngine.start(format)
}.getOrElse { error ->
    handleListenerPlaybackEngineFailure(error)
    return
}
logger.i("playback.listener", "Started transport listener playback via $backend")
```

When scheduler can start, set `buffered = true`:

```kotlin
_uiState.value = _uiState.value.copy(
    listenerState = ListenerLifecycleState.PLAYING,
    listenerPlaybackState = PlaybackState.PLAYING,
    connectionProgress = _uiState.value.connectionProgress.copy(
        currentState = ListenerLifecycleState.PLAYING,
        connected = true,
        approved = true,
        synced = true,
        buffered = true,
        playing = true,
    ),
)
```

Replace direct write:

```kotlin
val frame = scheduler.poll()
if (frame == null) {
    delay(10)
    continue
}
runCatching {
    playbackEngine.write(frame)
}.onFailure { error ->
    handleListenerPlaybackEngineFailure(error)
    return@launch
}
```

Acceptance:

- Real transport listener path starts the playback engine before first write.
- `buffered = true` is set before/when `playing = true`.
- No bare `playbackEngine.write(frame)` remains in listener transport playback.

## P0.5 Catch host preview `AudioTrack` write failures in the stream loop

**File:** `MainViewModel.kt`

Current problem: `startHostStreamingLoop()` directly writes preview audio with no catch.

Replace:

```kotlin
playbackEngine.write(
    PlaybackFrame(
        packet = packet,
        localDeadlineMs = packet.hostPresentationTimeMs,
    ),
)
```

With:

```kotlin
runCatching {
    playbackEngine.write(
        PlaybackFrame(
            packet = packet,
            localDeadlineMs = packet.hostPresentationTimeMs,
        ),
    )
}.onFailure { error ->
    handleHostPlaybackEngineFailure(error)
    return@launch
}
```

Acceptance:

- Host preview write failure transitions host to error.
- No bare `playbackEngine.write(...)` remains in the host streaming loop.

---

# P0 — Fix scan lifecycle and join gating still not wired into UI

## P0.6 Make Discover screen use `uiState.isScanning`

**File:** `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`

Replace:

```kotlin
val isScanning = uiState.listenerState == ListenerLifecycleState.SCANNING
```

With:

```kotlin
val isScanning = uiState.isScanning
```

Remove the `ListenerLifecycleState` import if unused.

Acceptance:

- Scan button enabled/disabled state follows `AppUiState.isScanning`.
- Grep should not find `listenerState == ListenerLifecycleState.SCANNING` in `DiscoverSessionsScreen.kt`.

## P0.7 Disable Join buttons while another join is active

**File:** `DiscoverSessionsScreen.kt`

Add imports:

```kotlin
import com.ekkus.silentdisco.app.canSelectSession
import com.ekkus.silentdisco.app.isJoinInProgress
```

Replace the session-card Join button block with:

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

Acceptance:

- Different-session Join buttons are disabled during active join states.
- Helper text explains why.
- The currently selected active session may remain enabled only if intended by `canSelectSession()`.

## P0.8 Add and use `clearScanState()` in listener reset paths

**File:** `MainViewModel.kt`

Add helper near other private helpers:

```kotlin
private fun clearScanState() {
    scanJob?.cancel()
    scanJob = null
    if (_uiState.value.isScanning) {
        _uiState.value = _uiState.value.copy(isScanning = false)
    }
}
```

Use it in:

- `cancelJoin()`
- `leaveSession()`
- any listener-flow reset path that can run while scanning
- `handleListenerConnectionFailure(...)` if the failure can occur during scan/join
- `handleListenerDisconnect(...)` if it can interrupt scan/join

Example:

```kotlin
fun cancelJoin() {
    clearScanState()
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.DISCONNECTED,
        connectionProgress = ConnectionProgressState(currentState = ListenerLifecycleState.DISCONNECTED),
        lastError = "Join cancelled",
    )
}
```

Example:

```kotlin
fun leaveSession() {
    clearScanState()
    hostStreamJob?.cancel()
    playbackJob?.cancel()
    resyncJob?.cancel()
    playbackEngine.stop()
    listenerScheduler = null
    pendingTransportPackets.clear()
    pendingSyncCorrelationId = null
    pendingJoinRequestMessage = null
    logger.i("listener.disconnect", "Listener left session")
    _uiState.value = _uiState.value.copy(
        listenerState = ListenerLifecycleState.IDLE,
        listenerPlaybackState = PlaybackState.STOPPED,
        selectedSession = null,
        isScanning = false,
        connectionProgress = ConnectionProgressState(),
    )
    diagnosticsStore.updateListener { ListenerDiagnosticsSnapshot() }
    refreshListenerDiagnostics()
}
```

Acceptance:

- `cancelJoin()` and `leaveSession()` always clear `isScanning`.
- Scan job is cancelled on listener reset.

---

# P0 — Prevent navigation after Wi-Fi Direct host startup failure

## P0.9 Add a transport operation result type

**File:** `app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt`

Add:

```kotlin
data class TransportOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        val Started = TransportOperationResult(started = true)
        fun failed(message: String) = TransportOperationResult(started = false, message = message)
    }
}
```

Change the interface:

```kotlin
fun startHost(session: SessionInfo): TransportOperationResult
```

Acceptance:

- The app can synchronously know whether host startup failed before navigation.

## P0.10 Make `WifiDirectTransportService.startHost()` return failure instead of only calling `fail()`

**File:** `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`

Replace the start-host shape with:

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

- Missing manager/channel returns `started = false`.
- Missing Wi-Fi permission returns `started = false`.
- Synchronous socket/start exceptions return `started = false`.

## P0.11 Use the Wi-Fi Direct result in `createHostSession()`

**File:** `MainViewModel.kt`

Replace:

```kotlin
return runCatching {
    wifiDirectService.startHost(session)
}.onSuccess {
    ...
}.onFailure { ... }.isSuccess
```

With explicit result handling:

```kotlin
val wifiStartResult = wifiDirectService.startHost(session)
if (!wifiStartResult.started) {
    val message = wifiStartResult.message ?: "Wi-Fi Direct host could not start"
    logger.w("transport.host", message)
    bleService.stopAdvertising()
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.ERROR,
        lastError = message,
    )
    diagnosticsStore.updateHost {
        it.copy(
            sessionId = session.id,
            streamState = PlaybackState.ERROR,
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR, sessionId = session.id)
    return false
}

logger.i("transport.host", "Started host session ${session.id}")
diagnosticsStore.updateHost {
    it.copy(
        sessionId = session.id,
        streamState = PlaybackState.STOPPED,
        lastContactElapsedMs = SystemClock.elapsedRealtime(),
        lastError = null,
    )
}
_uiState.value = _uiState.value.copy(
    hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
    discoveredSessions = listOf(session),
    lastMessage = "Hosting ${session.name}",
    lastError = null,
)
refreshHostDiagnostics()
return true
```

Acceptance:

- `createHostSession()` returns `false` when Wi-Fi Direct cannot start.
- Host Setup does not navigate to Host Control on immediate Wi-Fi Direct failure.
- BLE advertising is stopped/cleaned up if Wi-Fi Direct fails after BLE start.

---

# P0 — Surface host audio broadcast failures instead of logging forever

## P0.12 Add `SendAllResult`

**File:** `TransportModels.kt` or `TcpTransport.kt`

If it is used outside `TcpTransport.kt`, put it in `TransportModels.kt`.

```kotlin
data class SendAllResult(
    val peerCount: Int,
    val successCount: Int,
    val failureCount: Int,
) {
    val deliveredToAnyPeer: Boolean get() = successCount > 0

    companion object {
        val NoServer = SendAllResult(peerCount = 0, successCount = 0, failureCount = 1)
    }
}
```

## P0.13 Change `TcpServerChannel.sendAll()` to return stats

**File:** `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`

Replace:

```kotlin
suspend fun sendAll(message: T) {
    val snapshot = peers.values.toList()
    snapshot.forEach { peer -> peer.send(message) }
}
```

With:

```kotlin
suspend fun sendAll(message: T): SendAllResult {
    val snapshot = peers.values.toList()
    var success = 0
    var failure = 0
    snapshot.forEach { peer ->
        runCatching { peer.send(message) }
            .onSuccess { success += 1 }
            .onFailure { error ->
                failure += 1
                logger.w("transport.$channelName.send", "Failed to send to peer: ${error.message}")
            }
    }
    return SendAllResult(
        peerCount = snapshot.size,
        successCount = success,
        failureCount = failure,
    )
}
```

Acceptance:

- One peer failure does not hide the delivery stats for other peers.
- Callers can tell zero peers from failed peers.

## P0.14 Return `SendAllResult` from broadcast methods

**Files:**

- `TransportModels.kt`
- `WifiDirectTransportService.kt`
- call sites in `MainViewModel.kt`

Change interface methods that broadcast to host-connected listeners:

```kotlin
suspend fun broadcastControl(message: ControlMessage): SendAllResult
suspend fun broadcastSyncResponse(packet: SyncResponsePacket): SendAllResult
suspend fun broadcastAudio(packet: AudioPacket): SendAllResult
```

Update implementation:

```kotlin
override suspend fun broadcastAudio(packet: AudioPacket): SendAllResult {
    val result = audioServer?.sendAll(packet) ?: error("Audio server is not active")
    recordHeartbeat()
    updateByteCounts()
    return result
}
```

Do the same for `broadcastControl()` and `broadcastSyncResponse()`.

Acceptance:

- Broadcast callers receive delivery stats.
- No production caller ignores important failure counts.

## P0.15 Stop host stream after repeated audio delivery failures

**File:** `MainViewModel.kt`

Inside `startHostStreamingLoop()` add local counters:

```kotlin
var consecutiveAudioSendFailures = 0
var zeroPeerBroadcastCount = 0
```

Replace the audio broadcast block with:

```kotlin
runCatching {
    wifiDirectService.broadcastAudio(packet)
}.onSuccess { result ->
    if (result.peerCount == 0) {
        zeroPeerBroadcastCount += 1
        diagnosticsStore.updateHost {
            it.copy(
                lastError = "No connected listeners for audio broadcast",
                metricsSummary = summarizeMetrics(),
            )
        }
        // Do not count zero peers as a transport failure that kills host preview.
        consecutiveAudioSendFailures = 0
    } else if (result.failureCount > 0) {
        consecutiveAudioSendFailures += 1
        val message = "Audio packet delivered to ${result.successCount}/${result.peerCount} listeners; ${result.failureCount} failed"
        logger.w("transport.audio", message)
        diagnosticsStore.updateHost {
            it.copy(
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
    } else {
        consecutiveAudioSendFailures = 0
        diagnosticsStore.updateHost { it.copy(lastError = null) }
    }
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
}

if (consecutiveAudioSendFailures >= 10) {
    val message = "Audio transport failed repeatedly; stream stopped"
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

- Partial delivery is visible.
- Repeated failure stops the stream.
- Zero listeners is disclosed but does not kill host preview.

---

# P1 — Fix host control broadcast failure handling

## P1.1 Add a host control failure helper

**File:** `MainViewModel.kt`

```kotlin
private fun handleHostControlFailure(action: String, error: Throwable) {
    val message = error.message ?: "Failed to $action"
    logger.w("transport.control", "$action failed: $message")
    _uiState.value = _uiState.value.copy(lastError = "$action failed: $message")
    diagnosticsStore.updateHost {
        it.copy(
            lastError = "$action failed: $message",
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics()
}
```

Acceptance:

- Failed host control messages update host state/diagnostics, not listener diagnostics.

## P1.2 Fix `approveJoinRequest()` failure surface

**File:** `MainViewModel.kt`

Replace:

```kotlin
}.onFailure { error ->
    handleListenerConnectionFailure(error.message ?: "Failed to send join approval")
}
```

With:

```kotlin
}.onFailure { error ->
    handleHostControlFailure("send join approval", error)
}
```

Acceptance:

- Approval broadcast failure is a host-side visible error.

## P1.3 Fix swallowed failures in reject/pause/stop/end-session/stream-start/stream-stop broadcasts

**File:** `MainViewModel.kt`

For each `runCatching { wifiDirectService.broadcastControl(...) }` that currently has no `onFailure`, add a host-control failure handler.

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
    }.onFailure { error ->
        handleHostControlFailure("broadcast pause", error)
    }
}
```

Example for stop:

```kotlin
}.onFailure { error ->
    handleHostControlFailure("broadcast stop", error)
}
```

Example for stream start:

```kotlin
}.onFailure { error ->
    handleHostControlFailure("broadcast stream start", error)
}
```

Example for end session:

```kotlin
}.onFailure { error ->
    handleHostControlFailure("broadcast session end", error)
}
```

Acceptance:

- Grep no longer finds `runCatching { ... broadcastControl ... }` without visible `onFailure` handling.
- Host diagnostics tells the host when listeners may not have received the command.

---

# P1 — Fix buffering/progress state consistency

## P1.4 Clear buffered/playing on stop, pause, leave, disconnect, and error

**File:** `MainViewModel.kt`

`propagateListenerPlaybackState()` currently only updates `playing`. Update it:

```kotlin
private fun propagateListenerPlaybackState(
    playbackState: PlaybackState,
    listenerState: ListenerLifecycleState,
    message: String,
) {
    val isPlaying = playbackState == PlaybackState.PLAYING
    _uiState.value = _uiState.value.copy(
        listenerPlaybackState = playbackState,
        listenerState = listenerState,
        connectionProgress = _uiState.value.connectionProgress.copy(
            buffered = if (isPlaying) true else false,
            playing = isPlaying,
        ),
        lastMessage = message,
    )
    diagnosticsStore.updateListener {
        it.copy(
            playbackState = playbackState,
            metricsSummary = summarizeMetrics(),
            lastError = if (listenerState == ListenerLifecycleState.DESYNCED) {
                "Listener sync trouble detected"
            } else {
                it.lastError
            },
        )
    }
    refreshListenerDiagnostics()
}
```

For error handlers, also clear both:

```kotlin
connectionProgress = _uiState.value.connectionProgress.copy(
    buffered = false,
    playing = false,
)
```

Acceptance:

- The progress UI never shows “Playing” done while “Buffering audio” remains active.
- Stop/error/leave clear progress flags.

---

# P1 — Fix diagnostics honesty

## P1.5 Use playback state label in Diagnostics

**File:** `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

Replace:

```kotlin
Text("Playback state: ${uiState.listenerDiagnostics.playbackState}")
```

With:

```kotlin
Text("Playback state: ${uiState.listenerDiagnostics.playbackState.label()}")
```

Acceptance:

```bash
grep -R "Playback state: .*playbackState}" app/src/main/java
```

returns no matches.

## P1.6 Split playback output from native bridge diagnostics

**File:** `DiagnosticsScreen.kt`

Replace the Audio engine card with:

```kotlin
Card(modifier = Modifier.fillMaxWidth()) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text("Audio engine", style = MaterialTheme.typography.titleMedium)
        Text("Playback output: Android AudioTrack")
        Text("Native bridge: ${OboeBridge.statusSummary()}")
        Text("Native backend diagnostics: ${OboeBridge.backendSummary()}")
    }
}
```

Acceptance:

- UI does not say or imply native Oboe playback is active.

## P1.7 Replace swallowed native load with structured availability state

**File:** `app/src/main/java/com/ekkus/silentdisco/core/audio/OboeBridge.kt`

Replace with:

```kotlin
package com.ekkus.silentdisco.core.audio

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
        val reason = loadResult.exceptionOrNull()?.message
        if (reason.isNullOrBlank()) "Native library not loaded" else "Native library not loaded: $reason"
    }
}
```

Acceptance:

- Native load failure is explicit and testable.
- Diagnostics-only fallbacks are allowed here because they are disclosed as diagnostics.

---

# P1 — Fix invite-code form behavior

## P1.8 Stop auto-generating invite code inside `updateHostForm()`

**File:** `MainViewModel.kt`

Current problem: `updateHostForm()` silently generates an invite code if invite-code mode has a blank code. That conflicts with the UI validation requirement.

Replace:

```kotlin
val resolvedInviteCode = if (approvalMode == ApprovalMode.INVITE_CODE && inviteCode.isBlank()) {
    generateInviteCode()
} else {
    inviteCode
}
```

With:

```kotlin
val resolvedInviteCode = inviteCode
```

Then update state with `inviteCode = resolvedInviteCode`.

Optional: if you want a generated-code action later, add a visible `Generate code` button and explicit handler. Do not silently generate during mode change.

Acceptance:

- Selecting invite-code mode with blank code keeps blank code.
- Validation and UI helper text block Start Hosting until the host enters a code.

## P1.9 Disable Start Hosting when invite code is required but blank

**File:** `app/src/main/java/com/ekkus/silentdisco/feature/host/HostSetupScreen.kt`

Replace the missing-items/can-start logic with:

```kotlin
val isStarting = uiState.hostState == HostLifecycleState.CREATING_SESSION
val nameBlank = uiState.hostForm.sessionName.isBlank()
val noAudio = uiState.hostForm.selectedAudio == null
val inviteCodeMissing = uiState.hostForm.approvalMode == ApprovalMode.INVITE_CODE &&
    uiState.hostForm.inviteCode.isBlank()
val missingItems = buildList {
    if (nameBlank) add("a session name")
    if (noAudio) add("an audio file")
    if (inviteCodeMissing) add("an invite code")
}
if (missingItems.isNotEmpty()) {
    Text(
        text = "Required: ${missingItems.joinToString(" and ")}",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
    )
}
if (inviteCodeMissing) {
    Text(
        "Enter an invite code or choose a different approval mode.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
    )
}
Button(
    onClick = onStartHosting,
    modifier = Modifier.fillMaxWidth(),
    enabled = !isStarting && !nameBlank && !noAudio && !inviteCodeMissing,
) {
    Text(if (isStarting) "Starting…" else "Start Hosting")
}
```

Also replace the helper text:

```kotlin
text = "Listeners must enter this invite code to request approval."
```

Do not display “Auto-generated when hosting” unless there is an explicit generate action.

Acceptance:

- UI blocks blank invite-code hosting.
- No auto-generation copy remains.

---

# P1 — Fix Android SDK-aware permission checks

## P1.10 Make `PermissionCatalogue.requiredPermissions()` SDK-aware

**File:** `app/src/main/java/com/ekkus/silentdisco/core/permissions/Permissions.kt`

Current problem: the catalogue always includes Android 12/13 permissions even on older SDKs. But `SilentDiscoApp.requiredPermissions()` only requests some permissions based on SDK. This can make `hasHostTransportPermissions()` / `hasListenerTransportPermissions()` fail forever on older devices.

Add imports:

```kotlin
import android.os.Build
```

Replace `requiredPermissions()` with:

```kotlin
fun requiredPermissions(sdkInt: Int = Build.VERSION.SDK_INT): List<AppPermission> = buildList {
    add(AppPermission.RecordAudio)
    add(AppPermission.AccessFineLocation)
    if (sdkInt >= Build.VERSION_CODES.TIRAMISU) {
        add(AppPermission.NearbyWifiDevices)
    }
    if (sdkInt >= Build.VERSION_CODES.S) {
        add(AppPermission.BluetoothScan)
        add(AppPermission.BluetoothAdvertise)
        add(AppPermission.BluetoothConnect)
    }
    add(AppPermission.PostNotifications)
}
```

If `POST_NOTIFICATIONS` is Android 13+ only in this project’s manifest, gate that too:

```kotlin
if (sdkInt >= Build.VERSION_CODES.TIRAMISU) {
    add(AppPermission.PostNotifications)
}
```

Use whichever matches the manifest and app behavior.

Acceptance:

- Android 29-30 required list excludes `NEARBY_WIFI_DEVICES` and Bluetooth runtime permissions.
- Android 31-32 includes Bluetooth runtime permissions but excludes `NEARBY_WIFI_DEVICES`.
- Android 33+ includes `NEARBY_WIFI_DEVICES` and Bluetooth runtime permissions.

## P1.11 Make `SilentDiscoApp.requiredPermissions()` use the catalogue

**File:** `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

Replace the private permission array builder with one derived from `PermissionCatalogue`.

```kotlin
private fun requiredPermissions(): Array<String> = PermissionCatalogue
    .requiredPermissions()
    .map { it.permission }
    .toTypedArray()
```

If `AppPermission` exposes the Android permission string under a different property name, use that property.

Acceptance:

- UI permission launcher and ViewModel permission state use the same source of truth.

## P1.12 Make transport permission checks use the SDK-aware catalogue

**File:** `MainViewModel.kt`

Current code checks enum states from `_uiState.value.permissions`. This is acceptable only if the permission list is SDK-aware. After P1.10/P1.11, verify these helpers evaluate only relevant permissions.

Recommended helper:

```kotlin
private fun isPermissionGranted(permission: AppPermission): Boolean =
    _uiState.value.permissions.any { it.permission == permission && it.granted }

private fun hasHostTransportPermissions(): Boolean {
    val required = PermissionCatalogue.requiredPermissions().toSet()
    fun grantedIfRequired(permission: AppPermission): Boolean =
        permission !in required || isPermissionGranted(permission)

    return grantedIfRequired(AppPermission.NearbyWifiDevices) &&
        grantedIfRequired(AppPermission.BluetoothAdvertise) &&
        grantedIfRequired(AppPermission.BluetoothConnect)
}

private fun hasListenerTransportPermissions(): Boolean {
    val required = PermissionCatalogue.requiredPermissions().toSet()
    fun grantedIfRequired(permission: AppPermission): Boolean =
        permission !in required || isPermissionGranted(permission)

    return grantedIfRequired(AppPermission.NearbyWifiDevices) &&
        grantedIfRequired(AppPermission.BluetoothScan) &&
        grantedIfRequired(AppPermission.BluetoothConnect)
}
```

Acceptance:

- Android 29-30 listener/host transport checks do not require impossible Bluetooth runtime permissions.
- Android 33+ still requires the modern permissions.

---

# P1 — Remove dangerous production fallbacks

## P1.13 Do not invent session/stream IDs in `startHostPlayback()`

**File:** `MainViewModel.kt`

Current problem:

```kotlin
val sessionId = currentSessionId ?: SessionId(_uiState.value.hostDiagnostics.sessionId.ifBlank { UUID.randomUUID().toString() })
val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}")
```

This can make playback appear to start without a real host session.

Replace with:

```kotlin
val sessionId = currentSessionId
if (sessionId == null) {
    val message = "Start a host session before starting playback"
    _uiState.value = _uiState.value.copy(
        hostState = HostLifecycleState.ERROR,
        hostPlaybackState = PlaybackState.ERROR,
        lastError = message,
    )
    diagnosticsStore.updateHost { it.copy(lastError = message, streamState = PlaybackState.ERROR) }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
    return
}

val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}").also {
    currentStreamId = it
}
```

Acceptance:

- No random session ID fallback exists in production host playback.
- Starting playback without a host session fails visibly.

## P1.14 Gate or remove local manual-resync fallback for real sessions

**File:** `MainViewModel.kt`

Current behavior applies a local host timing response when not connected:

```kotlin
applySyncResponse(hostTimingService.createResponse(request))
_uiState.value = _uiState.value.copy(lastMessage = "Manual resync applied locally", lastError = null)
```

For production sessions, this can fake a successful resync.

Replace the disconnected path with:

```kotlin
val selectedSession = _uiState.value.selectedSession
val isDemoSession = BuildConfig.DEBUG && selectedSession?.id?.startsWith("demo-session-") == true
if (isDemoSession) {
    applySyncResponse(hostTimingService.createResponse(request))
    _uiState.value = _uiState.value.copy(lastMessage = "Manual resync applied locally for demo session", lastError = null)
    return
}

val message = "Manual resync requires an active host connection"
_uiState.value = _uiState.value.copy(lastError = message)
diagnosticsStore.updateListener { it.copy(lastError = message) }
refreshListenerDiagnostics()
```

Acceptance:

- Real sessions do not pretend to resync without a host connection.
- Demo/local fallback is debug-gated and disclosed.

---

# P2 — Reduce memory risk in host packetization

## P2.1 Stop concatenating all decoded chunks with repeated `ByteArray` copies

**File:** `MainViewModel.kt`

Current code:

```kotlin
val combinedBytes = decoded.chunks.fold(ByteArray(0)) { acc, chunk -> acc + chunk.pcm16Le }
```

This repeatedly allocates and copies, and can blow up on full music tracks.

Minimum safer replacement:

```kotlin
val totalBytes = decoded.chunks.sumOf { it.pcm16Le.size }
val combinedBytes = ByteArray(totalBytes)
var offset = 0
decoded.chunks.forEach { chunk ->
    chunk.pcm16Le.copyInto(combinedBytes, destinationOffset = offset)
    offset += chunk.pcm16Le.size
}
```

Better follow-up: packetize chunk-by-chunk without materializing the full track. That may require changing `PcmPacketizer` to preserve sequence state across chunks.

Acceptance:

- No repeated `acc + chunk.pcm16Le` allocation loop remains.
- Add a TODO/comment if full streaming packetization is deferred.

---

# P2 — Strengthen tests

## P2.2 Add scan/join UI-state tests

**Files:**

- `app/src/test/java/com/ekkus/silentdisco/app/ScanLifecycleTest.kt`
- `app/src/test/java/com/ekkus/silentdisco/app/UiStateValidationTest.kt`

Add tests for helpers and reset paths. If ViewModel testing is too heavy, extract pure helpers where practical.

Required assertions:

- `AppUiState(listenerState = SCANNING, isScanning = false)` means UI should not show scanning.
- `canSelectSession(other)` is false during active join.
- `cancelJoin()` clears `isScanning`.
- `leaveSession()` clears `isScanning`.

## P2.3 Add transport result tests

**File:** `app/src/test/java/com/ekkus/silentdisco/core/transport/TcpTransportTest.kt` or equivalent

Required assertions:

- `sendAll()` returns `peerCount = 0` when no peers.
- If one fake peer fails and one succeeds, result is success=1/failure=1.
- Broadcast callers do not treat zero peers as delivered success.

## P2.4 Add host startup failure test

**File:** `app/src/test/java/com/ekkus/silentdisco/app/HostStartupValidationTest.kt`

Use fake BLE and fake transport if needed.

Required assertions:

- BLE start failure returns `false` and host state is `ERROR`.
- Wi-Fi Direct start failure returns `false` and host state is `ERROR`.
- Success path returns `true` and host state is `WAITING_FOR_LISTENERS`.

## P2.5 Add playback-loop error tests

Required assertions:

- `AudioTrackPlaybackEngine.write()` before `start()` throws.
- Listener playback write failure sets `listenerState = ERROR`, `listenerPlaybackState = ERROR`, and `lastError`.
- Host preview write failure sets `hostState = ERROR`, `hostPlaybackState = ERROR`, and `lastError`.

If the existing `MainViewModel` is hard to test because `playbackEngine` is not injectable, introduce an interface:

```kotlin
interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}
```

Then inject `PlaybackEngine` into `MainViewModel` with `AudioTrackPlaybackEngine()` as the default.

## P2.6 Add permission matrix tests

**File:** `app/src/test/java/com/ekkus/silentdisco/core/permissions/PermissionCatalogueTest.kt`

Required assertions:

```kotlin
@Test
fun android30DoesNotRequireNearbyWifiOrBluetoothRuntimePermissions() {
    val required = PermissionCatalogue.requiredPermissions(sdkInt = 30)
    assertFalse(AppPermission.NearbyWifiDevices in required)
    assertFalse(AppPermission.BluetoothScan in required)
    assertFalse(AppPermission.BluetoothAdvertise in required)
    assertFalse(AppPermission.BluetoothConnect in required)
}

@Test
fun android31RequiresBluetoothRuntimeButNotNearbyWifi() {
    val required = PermissionCatalogue.requiredPermissions(sdkInt = 31)
    assertFalse(AppPermission.NearbyWifiDevices in required)
    assertTrue(AppPermission.BluetoothScan in required)
    assertTrue(AppPermission.BluetoothAdvertise in required)
    assertTrue(AppPermission.BluetoothConnect in required)
}

@Test
fun android33RequiresNearbyWifiAndBluetoothRuntimePermissions() {
    val required = PermissionCatalogue.requiredPermissions(sdkInt = 33)
    assertTrue(AppPermission.NearbyWifiDevices in required)
    assertTrue(AppPermission.BluetoothScan in required)
    assertTrue(AppPermission.BluetoothAdvertise in required)
    assertTrue(AppPermission.BluetoothConnect in required)
}
```

---

# Validation checklist before handing back

Run:

```bash
./gradlew test
./gradlew lintDebug
```

Manual/grep checks:

```bash
# Discover UI must not derive scanning from listener state
grep -R "listenerState == ListenerLifecycleState.SCANNING" app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt

# Listener/host playback loops must not write without catch
grep -R "playbackEngine.write" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# Raw enum display should be gone
grep -R "Playback state: .*playbackState}" app/src/main/java

# No swallowed broadcast-control failure blocks
grep -R "runCatching" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
```

Manual app checks:

- Scan indicator stops after scan window.
- Scan button re-enables after empty scan, permission failure, BLE scan failure, cancel join, and leave session.
- Join buttons disable while another join is active.
- Host setup blocks blank invite-code mode.
- Host setup does not navigate to Host Control if BLE advertising fails.
- Host setup does not navigate to Host Control if Wi-Fi Direct startup fails.
- Listener playback does not enter stale `PLAYING` after playback engine failure.
- Host stream stops after repeated audio broadcast failures.
- Pause/stop/end-session broadcast failures are visible in host diagnostics.
- Diagnostics says `Playback output: Android AudioTrack` and does not imply native Oboe playback.
- Android 29-30 devices are not blocked by Android 12/13-only runtime permissions.
