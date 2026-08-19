package com.ekkus.silentdisco.core.rust

import android.os.SystemClock
import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** Durable listener-side diagnostics, beside the debug PCM captures. */
private const val DIAGNOSTICS_LOG_FILE_NAME = "manual-listener-diagnostics.log"

private const val DIAGNOSTICS_SAMPLE_CADENCE_MS = 1_000L

/**
 * Samples playback diagnostics once a second for the life of one stream.
 *
 * The end-of-stream summary reports only totals, which cannot distinguish
 * a defect concentrated in the first second of playback from the same
 * count spread evenly across the whole stream -- and the two have
 * completely different causes. The debug WAV cannot settle it either: it
 * captures frames on their way *into* the render ring, so silence the
 * real-time callback substitutes on an empty ring is inaudible to it.
 * Per-second deltas are the only view that locates a hiccup in time.
 */
internal fun ManualListenerTransportController.startDiagnosticsSampler(
    scope: CoroutineScope,
    runtime: FfiListenerPlaybackHandle,
) {
    diagnosticsSampleJob?.cancel()
    diagnosticsSampleJob = scope.launch(Dispatchers.IO) {
        var previousUnderruns = 0uL
        var previousSilenceFrames = 0uL
        var previousConcealed = 0uL
        var previousEmitted = 0uL
        var previousDroppedBeforeSync = 0uL
        while (isActive) {
            delay(DIAGNOSTICS_SAMPLE_CADENCE_MS)
            val diagnostics = playbackRuntime?.takeIf { it === runtime }?.diagnostics() ?: break
            appendDiagnosticsLine(
                "sample emitted=+${diagnostics.packetsEmitted - previousEmitted} " +
                    "concealed=+${diagnostics.concealedPackets - previousConcealed} " +
                    "underruns=+${diagnostics.ringUnderruns - previousUnderruns} " +
                    "silenceFrames=+${diagnostics.ringSilenceFilledFrames - previousSilenceFrames} " +
                    "ringQueued=${diagnostics.ringQueuedFrames} " +
                    "bufferedMs=${diagnostics.bufferedSpanMs} phase=${diagnostics.phase}",
            )
            logger.i(
                "manual.audio.sample",
                "emitted=+${diagnostics.packetsEmitted - previousEmitted} " +
                    "concealed=+${diagnostics.concealedPackets - previousConcealed} " +
                    "underruns=+${diagnostics.ringUnderruns - previousUnderruns} " +
                    "silenceFrames=+${diagnostics.ringSilenceFilledFrames - previousSilenceFrames} " +
                    "ringQueued=${diagnostics.ringQueuedFrames} " +
                    "bufferedMs=${diagnostics.bufferedSpanMs} phase=${diagnostics.phase}",
            )
            if (!diagnostics.syncLocked && diagnostics.droppedBeforeSync > previousDroppedBeforeSync) {
                val warning =
                    "Waiting for clock sync; ${diagnostics.droppedBeforeSync} audio packets were dropped before lock"
                logger.w("manual.audio.pre_sync_drop", warning)
                appendDiagnosticsLine("warning $warning")
                val current = _connectState.value
                if (current is ManualConnectUiState.Streaming) {
                    _connectState.value = current.copy(syncStatus = warning)
                }
            }
            previousUnderruns = diagnostics.ringUnderruns
            previousSilenceFrames = diagnostics.ringSilenceFilledFrames
            previousConcealed = diagnostics.concealedPackets
            previousEmitted = diagnostics.packetsEmitted
            previousDroppedBeforeSync = diagnostics.droppedBeforeSync
        }
    }
}

/**
 * Appends one diagnostic line to a durable file beside the debug PCM
 * capture, when a recording directory is configured.
 *
 * `AppLogger` reaches only logcat, which is entirely unavailable on some
 * real devices (confirmed on an Android 8.0 phone: every buffer empty,
 * `ro.logdumpd.enabled=0`). Physical-device validation is exactly where
 * these numbers matter most, and the in-app diagnostics screen cannot be
 * reached mid-stream without tearing the session down -- so without a
 * file sink a real run's listener-side counters are simply unobservable.
 */
internal fun ManualListenerTransportController.appendDiagnosticsLine(line: String) {
    val directory = debugRecordingDirectory ?: return
    runCatching {
        File(directory, DIAGNOSTICS_LOG_FILE_NAME)
            .appendText("[t=${SystemClock.elapsedRealtime()}] $line\n")
    }
}

/**
 * Logs the stream's final accounting from the Rust runtime.
 *
 * Every counter here is produced by the component that owns the
 * behaviour it describes, so the summary cannot disagree with what
 * actually happened the way an independently maintained tally could.
 */
internal fun ManualListenerTransportController.logPlaybackSummary(runtime: FfiListenerPlaybackHandle) {
    val diagnostics = runtime.finalDiagnostics() ?: runtime.diagnostics()
    appendDiagnosticsLine(
        "summary streamId=$currentStreamId " +
            "forwarded=${runCatching { handleRef.get()?.forwardedAudioCount() }.getOrNull()} " +
            "received=$receivedCount " +
            "accepted=${diagnostics.packetsAccepted} emitted=${diagnostics.packetsEmitted} " +
            "concealed=${diagnostics.concealedPackets} late=${diagnostics.lateRejections} " +
            "skipped=${diagnostics.sequencesSkipped} " +
            "droppedBeforeSync=${diagnostics.droppedBeforeSync} " +
            // A4.4: `hardResyncs` is the sum of both rebuffer causes --
            // the concealment/offset breakdown is what tells them apart.
            "hardResyncs=${diagnostics.hardResyncSignals} " +
            "(concealment=${diagnostics.concealmentDrivenRebuffers} " +
            "offset=${diagnostics.offsetDrivenRebuffers}) " +
            "ringUnderruns=${diagnostics.ringUnderruns} " +
            "ringSilenceFilled=${diagnostics.ringSilenceFilledFrames} " +
            "ringFullEvents=${diagnostics.ringFullEvents} " +
            "ringPeakFrames=${diagnostics.ringPeakQueuedFrames} " +
            "prefillFrames=${diagnostics.prefillFrames} phase=${diagnostics.phase} " +
            "oboe=${OboeBridge.lastOpenSummary()}",
    )
    logger.i(
        "manual.audio.summary",
        "received=$receivedCount accepted=${diagnostics.packetsAccepted} " +
            "emitted=${diagnostics.packetsEmitted} skipped=${diagnostics.sequencesSkipped} " +
            "concealed=${diagnostics.concealedPackets} late=${diagnostics.lateRejections} " +
            "duplicate=${diagnostics.duplicateRejections} " +
            "reorderWindow=${diagnostics.reorderWindowRejections} " +
            "hardResyncs=${diagnostics.hardResyncSignals} " +
            "(concealment=${diagnostics.concealmentDrivenRebuffers} " +
            "offset=${diagnostics.offsetDrivenRebuffers}) " +
            "resyncs=${diagnostics.resynchronisations} " +
            "droppedBeforeSync=${diagnostics.droppedBeforeSync} " +
            "ringPeakFrames=${diagnostics.ringPeakQueuedFrames} " +
            "prefillFrames=${diagnostics.prefillFrames} " +
            "ringUnderruns=${diagnostics.ringUnderruns} " +
            "ringSilenceFilled=${diagnostics.ringSilenceFilledFrames} " +
            "ringFullEvents=${diagnostics.ringFullEvents} phase=${diagnostics.phase}",
    )
    runtime.debugCaptureError()?.let { error ->
        logger.w("manual.audio.recording_error", error)
    }
}
