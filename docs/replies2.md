# Silent Disco — replies2.md

These replies address `responses2(27).md` for the FIX3 hardening pass. Treat this file as the tie-breaker where the spec/TODO wording was ambiguous.

---

## Q1 — File naming

Yes. **`FIX3` is the canonical name for this pass.**

Use these names consistently:

- `docs/SILENT_DISCO_FIX3_SPEC.md`
- `docs/SILENT_DISCO_FIX3_TODO.md`
- `docs/replies2.md`

Do **not** rename this pass to `SILENT_DISCO_CODE_REVIEW3_*` unless Phillip explicitly asks for that later. This is a continuation of the CODE_REVIEW2 hardening work, but `FIX3` is the intended working name.

---

## Q2 — “Styling” vs hardening

Proceed with **FIX3 as a pure correctness / state-machine / silent-failure hardening pass**.

There is no separate styling/UI-polish spec for this pass. If the original wording mentioned “styling,” treat that as imprecise wording, not as a requirement to add typography, colors, spacing, animations, visual polish, or layout redesign.

The only UI work in scope is UI that prevents the app from lying to the user, such as:

- disabling buttons that cannot safely work,
- showing helper text explaining why an action is disabled,
- replacing misleading diagnostics labels,
- surfacing real error states instead of log-only failures,
- ensuring loading/scanning/progress indicators match real state.

Do **not** add cosmetic styling changes unless they are directly required to make a state or failure visible.

---

## Q3 — `ControlMessage.StreamStart` fields

In the reviewed code, `ControlMessage.StreamStart` already has these fields:

```kotlin
val sampleRate: Int,
val channels: Int,
val samplesPerPacket: Int,
```

So you do **not** need to add `sampleRate` or `channels` to the protocol model for FIX3 if your local tree matches the reviewed zip.

Use them when starting real listener playback:

```kotlin
private fun handleRemoteStreamStart(message: ControlMessage.StreamStart) {
    currentStreamId = message.streamId
    pendingTransportPackets.clear()
    listenerScheduler = null

    val playbackFormat = AudioFormatSpec(
        sampleRate = message.sampleRate,
        channelCount = message.channels,
    )

    startTransportListenerPlayback(
        streamId = message.streamId,
        format = playbackFormat,
    )
}
```

If your branch somehow lacks those fields, add them to `StreamStart` and update every construction/serialization path in the same commit. But based on the submitted code, they already exist.

---

## Q4 — `refreshHostDiagnostics(streamState = ...)`

In the reviewed code, `refreshHostDiagnostics()` already accepts a named `streamState` parameter:

```kotlin
private fun refreshHostDiagnostics(
    streamState: PlaybackState = _uiState.value.hostPlaybackState,
    sessionId: String = _uiState.value.hostDiagnostics.sessionId,
) {
    diagnosticsStore.updateHost {
        it.copy(
            sessionId = sessionId,
            listenerCount = _uiState.value.approvedListeners.size + _uiState.value.pendingJoinRequests.size,
            pendingJoinCount = _uiState.value.pendingJoinRequests.size,
            connectedListenerCount = _uiState.value.approvedListeners.count {
                it.connectionState == TransportConnectionState.CONNECTING ||
                    it.connectionState == TransportConnectionState.CONNECTED
            },
            desyncedListenerCount = _uiState.value.approvedListeners.count {
                it.listenerState == ListenerLifecycleState.DESYNCED ||
                    it.syncQuality == SyncQualityBadge.POOR
            } + if (_uiState.value.listenerState == ListenerLifecycleState.DESYNCED) 1 else 0,
            streamState = streamState,
            lastContactElapsedMs = wifiDirectService.snapshot.value.lastContactElapsedMs,
            metricsSummary = summarizeMetrics(),
            packetBudgetSummary = it.packetBudgetSummary,
            lastError = _uiState.value.lastError,
        )
    }
    _uiState.value = _uiState.value.copy(hostDiagnostics = diagnosticsStore.hostDiagnostics.value)
}
```

So if your tree matches the reviewed zip, no overload is needed.

If the named parameter is missing in your local branch, add it exactly like above. The important part is that host failure helpers can force diagnostics to show `PlaybackState.ERROR` even if some other stale host playback state exists.

---

## Q5 — `summarizeMetrics()`

In the reviewed code, `summarizeMetrics()` already exists in `MainViewModel`.

Use the existing helper. Do **not** invent a second metrics summary function.

Current shape:

```kotlin
private fun summarizeMetrics(): String {
    val counters = metrics.snapshotCounters()
    val timings = metrics.snapshotTimings()
    if (counters.isEmpty() && timings.isEmpty()) return "No metrics yet"
    val counterSummary = counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
    val timingSummary = timings.entries.joinToString(", ") { "${it.key}=${"%.1f".format(it.value)}ms" }
    return listOf(counterSummary, timingSummary).filter { it.isNotBlank() }.joinToString(" | ")
}
```

That is enough for FIX3. The goal is not to design a full observability system in this pass. The goal is to ensure that when a user-visible failure happens, diagnostics are refreshed and include the current metrics summary rather than silently logging and continuing.

Do not block P0/P1 work on adding packet-loss/buffer-depth/sync-stat details unless they are already available in the code path you are touching.

---

## Q6 — `PlaybackEngine` interface for testability

Yes, add the `PlaybackEngine` interface as part of FIX3.

Even though it was listed as P2.5, it directly supports testing the P0 requirement: playback write failures must transition to visible error state instead of killing a coroutine or pretending success.

Keep the interface minimal and low-churn:

```kotlin
interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}
```

Then make the renamed engine implement it:

```kotlin
class AudioTrackPlaybackEngine : PlaybackEngine {
    // existing AudioTrack-backed implementation
}
```

For `MainViewModel`, do not break normal Android construction. Use a default parameter with `@JvmOverloads`, or a secondary constructor, so the framework can still instantiate the ViewModel with only `Application`.

Preferred shape:

```kotlin
class MainViewModel @JvmOverloads constructor(
    application: Application,
    private val playbackEngine: PlaybackEngine = AudioTrackPlaybackEngine(),
) : AndroidViewModel(application) {
    // existing implementation
}
```

Then unit tests can inject a fake:

```kotlin
private class FailingPlaybackEngine : PlaybackEngine {
    override fun start(format: AudioFormatSpec): String = "fake"
    override fun write(frame: PlaybackFrame): Long = error("Injected write failure")
    override fun setVolume(value: Float) = Unit
    override fun playbackPositionMs(frame: PlaybackFrame): Long = 0L
    override fun stop() = Unit
}
```

Acceptance criterion: there must be at least one test proving that an injected write failure moves the listener or host playback state to `PlaybackState.ERROR` and sets a visible `lastError`.

If this creates too much constructor churn, stop and implement the runtime behavior first, but the preferred FIX3 solution is to add the interface now.

---

## Q7 — `TcpServerChannel.channelName` and logger

In the reviewed code, `TcpServerChannel` already has both:

```kotlin
private val channelName: String,
private val logger: AppLogger,
```

So do **not** use a generic fallback tag unless your local tree differs.

Implement `SendAllResult` and use the existing `channelName`/`logger` fields:

```kotlin
data class SendAllResult(
    val peerCount: Int,
    val successCount: Int,
    val failureCount: Int,
) {
    val allDelivered: Boolean
        get() = peerCount > 0 && failureCount == 0
}
```

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

Important: a zero-peer result is not a successful delivery. Host preview may continue with zero listeners, but diagnostics must not imply audio was delivered to listeners.

---

## Q8 — `latestDecodedAudio` in P0.3

In the reviewed code, `MainViewModel` already has:

```kotlin
private var latestDecodedAudio: AudioDecodeResult? = null
```

For the **simulation path**, default `AudioFormatSpec()` is acceptable because synthetic packets are generated with the default format.

Use this shape:

```kotlin
val playbackFormat = latestDecodedAudio?.format ?: AudioFormatSpec()
val backend = playbackEngine.start(playbackFormat)
```

But do not make simulation depend on decoding a real selected audio file. If `latestDecodedAudio` is null in the simulation path, `AudioFormatSpec()` is the right fallback because the simulation packets are synthetic/default-format packets.

For the **real transport path**, use the `StreamStart` format from Q3 instead of `latestDecodedAudio`, because listeners may not have decoded the host's source file locally.

---

## Q9 — `stopAdvertising()` vs `stop()` on `BleDiscoveryService`

Add a public `stopAdvertising()` method.

The reviewed code already has a private `stopAdvertising()`. Promote it to public/internal API and keep `stop()` calling it.

Preferred shape:

```kotlin
fun stop() {
    stopAdvertising()
    stopScanning()
    _advertisement.value = null
    seenSessions.clear()
    _discoveredSessions.value = emptyList()
}

@SuppressLint("MissingPermission")
fun stopAdvertising() {
    val callback = advertiseCallback ?: return
    if (hasAdvertisePermission()) {
        runCatching { advertiser?.stopAdvertising(callback) }
    }
    advertiseCallback = null
    _advertisement.value = null
}
```

Use `bleService.stopAdvertising()` when host startup fails after BLE advertising has already started.

Do not call `bleService.stop()` for this specific cleanup unless you intentionally want to clear scanning/discovery state too. It is probably safe during host startup, but it is broader than necessary. FIX3 should prefer precise cleanup over broad cleanup.

---

## Q10 — `typealias OboePlaybackEngine`

Update all production callers and tests to use `AudioTrackPlaybackEngine` directly.

Keep a deprecated typealias only as a temporary low-churn compatibility marker:

```kotlin
@Deprecated(
    message = "Use AudioTrackPlaybackEngine. Playback output is Android AudioTrack-backed; the native Oboe bridge is diagnostics-only.",
    replaceWith = ReplaceWith("AudioTrackPlaybackEngine"),
)
typealias OboePlaybackEngine = AudioTrackPlaybackEngine
```

Do not keep `OboePlaybackEngine` as the primary name. The point of this task is diagnostics honesty: if playback is AudioTrack-backed, the class used by `MainViewModel` should say `AudioTrackPlaybackEngine`.

Acceptance criteria:

- `MainViewModel` imports/uses `AudioTrackPlaybackEngine` or the `PlaybackEngine` interface, not `OboePlaybackEngine`.
- Test files use `AudioTrackPlaybackEngine` or fake `PlaybackEngine` implementations.
- The only remaining `OboePlaybackEngine` reference should be the deprecated typealias, or none at all if you choose to remove it completely.
- Diagnostics must not claim “Oboe + AudioTrack” as the playback output path.

---

## Additional implementation guidance

### Keep the pass narrow

Do not use FIX3 as an opportunity to redesign the app. The job is to make the existing app honest and failure-aware.

### Do not add broad suppressing wrappers

Avoid this pattern:

```kotlin
runCatching { somethingImportant() }
    .onFailure { logger.w("tag", "failed") }
```

For user-visible operations, failure must update at least one of:

- `lastError`,
- host/listener diagnostics,
- playback state,
- lifecycle state,
- button enabled/disabled state.

### Do not convert real failures into fake local success

Debug/demo paths are allowed, but they must be gated with `BuildConfig.DEBUG` and/or explicit demo-session IDs. Production listener/host paths should not silently simulate success.

### Commit order suggestion

A safe implementation order is:

1. Rename `OboePlaybackEngine` to `AudioTrackPlaybackEngine` and add `PlaybackEngine` interface.
2. Start/catch playback engine writes in listener and host playback loops.
3. Fix scan/join UI wiring.
4. Fix host startup result handling, including precise BLE advertising cleanup.
5. Implement transport `SendAllResult` and repeated audio-send failure escalation.
6. Fix diagnostics honesty labels.
7. Add/repair tests.

