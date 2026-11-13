package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.NativeCallbackWavCapture
import com.ekkus.silentdisco.core.diagnostics.DiagnosticsStore
import com.ekkus.silentdisco.core.logging.DiagnosticsMetrics
import com.ekkus.silentdisco.core.model.BridgeQueueCounters
import com.ekkus.silentdisco.core.model.HostDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.ListenerDiagnosticsSnapshot
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** What a host diagnostics refresh needs from the rest of the app. */
internal data class HostDiagnosticsInput(
    val sessionId: String,
    val listenerCount: Int,
    val pendingJoinCount: Int,
    val connectedListenerCount: Int,
    val desyncedListenerCount: Int,
    val streamState: PlaybackState,
    val lastContactElapsedMs: Long?,
    val lastError: String?,
)

/**
 * Diagnostics: the host and listener snapshots, metrics, bridge-queue
 * pressure, and the developer-only output callback capture (God-file
 * refactor spec §4). Other code reports through [updateHost],
 * [updateListener] and [metrics]; nothing else holds the store.
 *
 * @param hostBridgeCounters host transport and core queue drops, read at
 *   each refresh.
 * @param listenerBridgeCounters the same for the listener.
 * @param captureEnabled false outside debug builds: capture requests are
 *   ignored there.
 */
internal class DiagnosticsController(
    parentScope: CoroutineScope,
    private val hostBridgeCounters: () -> BridgeQueueCounters,
    private val listenerBridgeCounters: () -> BridgeQueueCounters,
    private val captureEnabled: Boolean,
) : ViewModelController(parentScope) {
    private val store = DiagnosticsStore()
    val metrics = DiagnosticsMetrics()
    val hostDiagnostics: StateFlow<HostDiagnosticsSnapshot> = store.hostDiagnostics
    val listenerDiagnostics: StateFlow<ListenerDiagnosticsSnapshot> = store.listenerDiagnostics

    private var capture: NativeCallbackWavCapture? = null
    private val _captureStatus = MutableStateFlow("Capture off")
    val captureStatus: StateFlow<String> = _captureStatus.asStateFlow()

    fun updateHost(transform: (HostDiagnosticsSnapshot) -> HostDiagnosticsSnapshot) =
        store.updateHost(transform)

    fun updateListener(transform: (ListenerDiagnosticsSnapshot) -> ListenerDiagnosticsSnapshot) =
        store.updateListener(transform)

    /** Recomputes the host snapshot and returns it. */
    fun refreshHost(input: HostDiagnosticsInput): HostDiagnosticsSnapshot {
        store.updateHost {
            it.copy(
                sessionId = input.sessionId,
                listenerCount = input.listenerCount,
                pendingJoinCount = input.pendingJoinCount,
                connectedListenerCount = input.connectedListenerCount,
                desyncedListenerCount = input.desyncedListenerCount,
                streamState = input.streamState,
                lastContactElapsedMs = input.lastContactElapsedMs,
                metricsSummary = summarizeMetrics(),
                packetBudgetSummary = it.packetBudgetSummary,
                lastError = input.lastError,
                bridgeQueues = hostBridgeCounters(),
            )
        }
        return store.hostDiagnostics.value
    }

    /** Re-reads the listener's bridge pressure and returns the snapshot. */
    fun refreshListener(): ListenerDiagnosticsSnapshot {
        store.updateListener { it.copy(bridgeQueues = listenerBridgeCounters()) }
        return store.listenerDiagnostics.value
    }

    fun summarizeMetrics(): String {
        val counters = metrics.snapshotCounters()
        val timings = metrics.snapshotTimings()
        if (counters.isEmpty() && timings.isEmpty()) return "No metrics yet"
        val counterSummary = counters.entries.joinToString(", ") { "${it.key}=${it.value}" }
        val timingSummary = timings.entries.joinToString(", ") { "${it.key}=${"%.1f".format(it.value)}ms" }
        return listOf(counterSummary, timingSummary).filter { it.isNotBlank() }.joinToString(" | ")
    }

    /**
     * Starts recording the output callback of [playback] into [directory].
     * Developer-only and explicitly started.
     */
    fun startCapture(playback: FfiListenerPlaybackHandle?, directory: File?) {
        if (!captureEnabled) return
        if (capture != null || _captureStatus.value == FINALIZING) return
        if (playback == null) {
            _captureStatus.value = "Connect and start listener playback before recording"
            return
        }
        if (directory == null) {
            _captureStatus.value = "App-private diagnostics directory is unavailable"
            return
        }
        runCatching {
            directory.mkdirs()
            NativeCallbackWavCapture.start(playback, directory, scope)
        }.onSuccess { started ->
            capture = started
            _captureStatus.value = "Recording callback PCM…"
        }.onFailure { error ->
            _captureStatus.value = "Capture could not start: ${error.message}"
        }
    }

    fun stopCapture() {
        if (!captureEnabled) return
        val running = capture ?: return
        capture = null
        _captureStatus.value = FINALIZING
        scope.launch {
            runCatching { running.stopAndReport() }
                .onSuccess { report ->
                    _captureStatus.value = if (report.complete && report.noUnexpectedOutputReopen == true &&
                        report.newUnderrunCallbacks == 0L && report.newSilenceFilledFrames == 0L
                    ) {
                        "Capture saved: ${report.file.name} (${report.wavFrames} frames; " +
                            "${report.callbackCount} callbacks; no reopen/underrun). Location: ${report.file.parent}"
                    } else {
                        "Unqualified capture: ${report.file.name}; ${report.lostFrames} dropped frames, " +
                            "${report.wavFrames} written, ${report.capturedFrames} captured; " +
                            "no output reopen=${report.noUnexpectedOutputReopen}; " +
                            "new underruns=${report.newUnderrunCallbacks}, " +
                            "silence frames=${report.newSilenceFilledFrames}. " +
                            "Do not count this recording as gapless acceptance."
                    }
                }
                .onFailure { error ->
                    _captureStatus.value = "Capture failed: ${error.message}"
                }
        }
    }

    /**
     * Ends a running capture during view-model teardown, blocking until its
     * file is finalized. `MainViewModel.onCleared` calls this at the same
     * point it always did, after asking listener playback to stop.
     */
    fun finishCaptureForTeardown() {
        val running = capture ?: return
        capture = null
        runBlocking(Dispatchers.IO) { runCatching { running.stopAndReport() } }
    }

    override fun release() = finishCaptureForTeardown()

    private companion object {
        const val FINALIZING = "Finalizing capture"
    }
}
