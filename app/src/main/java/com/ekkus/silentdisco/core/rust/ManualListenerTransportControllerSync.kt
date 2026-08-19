package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.model.ManualConnectUiState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Interval between clock-sync probes once the clock is locked.
 *
 * Before it locks, probes go out at [SYNC_PROBE_ACQUIRE_CADENCE_MS] instead:
 * playback produces nothing at all until a sample is accepted, and the
 * estimator rejects any sample whose round trip exceeds its bound, so a
 * steady 2s cadence turns three unlucky probes into six seconds of silence.
 */
private const val SYNC_PROBE_CADENCE_MS = 2_000L
private const val SYNC_PROBE_ACQUIRE_CADENCE_MS = 250L

/**
 * Drives clock-sync probes for the life of one stream.
 *
 * The first probe goes out immediately rather than after a cadence delay:
 * playback cannot start until a sample is genuinely accepted, so the
 * round trip is on the critical path to hearing anything.
 */
internal fun ManualListenerTransportController.startSyncProbeLoop(
    scope: CoroutineScope,
    handle: FfiListenerTransportHandle,
) {
    syncProbeJob?.cancel()
    syncProbeJob = scope.launch(Dispatchers.IO) {
        var correlationId = 1L
        while (isActive) {
            val runtime = playbackRuntime ?: break
            val sendTimeMs = runtime.nowMs()
            // Register before sending: a response that beats its own
            // registration has nothing to correlate against.
            runCatching {
                runtime.beginSyncProbe(correlationId.toULong(), sendTimeMs)
            }.getOrElse { error ->
                logger.w("manual.audio.sync_probe_failed", error.message ?: "sync probe registration failed")
                handlePlaybackEngineFailure(error)
                break
            }
            runCatching {
                handle.sendSyncRequest(correlationId.toULong(), sendTimeMs)
            }.getOrElse { error ->
                logger.w("manual.audio.sync_send_failed", error.message ?: "sync request send failed")
                stopPlayback()?.let(error::addSuppressed)
                _connectState.value = ManualConnectUiState.Failed(
                    error.message ?: "sync request send failed",
                )
                break
            }
            correlationId += 1
            val locked = playbackRuntime?.diagnostics()?.syncLocked == true
            delay(if (locked) SYNC_PROBE_CADENCE_MS else SYNC_PROBE_ACQUIRE_CADENCE_MS)
        }
    }
}

/**
 * Translates a [FfiListenerTransportEvent.SyncResponseReceived.receivedAtElapsedMs]
 * (the transport's own clock) onto [ManualListenerTransportController.playbackRuntime]'s
 * timeline, using the one-time delta captured in
 * [ManualListenerTransportController.transportClockOriginMs].
 *
 * Both clocks are `Instant`-backed with different origins, so their
 * *deltas* -- not raw readings -- are what is comparable. `elapsedTransport
 * - originTransport` is the elapsed time since the runtime's own t=0, in
 * transport-clock units; since both clocks tick at the same real rate,
 * that is exactly the runtime-clock elapsed time too. Falls back to the
 * live `nowMs()` (the pre-fix behaviour, dispatch delay included) if no
 * origin was captured -- still correct, just not as tight.
 */
internal fun translateTransportElapsedToPumpClock(
    transportClockOriginMs: ULong?,
    elapsedTransportMs: ULong,
    fallbackNowMs: () -> ULong,
): ULong {
    val origin = transportClockOriginMs ?: return fallbackNowMs()
    return if (elapsedTransportMs >= origin) elapsedTransportMs - origin else 0uL
}

internal fun ManualListenerTransportController.translateToPumpClock(
    runtime: FfiListenerPlaybackHandle,
    elapsedTransportMs: ULong,
): ULong = translateTransportElapsedToPumpClock(
    transportClockOriginMs = transportClockOriginMs,
    elapsedTransportMs = elapsedTransportMs,
    fallbackNowMs = runtime::nowMs,
)

/**
 * Forwards one four-timestamp exchange to the Rust estimator.
 *
 * `t4` is stamped as close to the response's actual socket receipt as
 * this layer can get -- translated from the transport's own clock via
 * [translateToPumpClock] -- rather than at the moment this event happens
 * to be processed. The two used to be conflated: any delay between
 * receipt and processing (dispatch, queued audio ahead of it in the old
 * per-packet event stream) was counted as network round-trip time,
 * which is what pushed marginal samples past the estimator's acceptance
 * gate on a real device. Nothing here interprets the sample: acceptance,
 * offset, and skew are the estimator's to decide, and a listener that
 * computed any of them would be duplicating domain logic it cannot keep
 * consistent.
 *
 * Also relays [outcome]'s confidence/offset/RTT/drift back to the host
 * via [FfiListenerTransportHandle.sendSynchronizationReport] on this
 * same per-exchange cadence -- no separate timer needed. This is the
 * only way the host's per-listener sync diagnostics are ever populated
 * (D2, `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`): the host never sees
 * `t4`, so it cannot compute any of this itself, and would otherwise
 * show "has not yet completed a sync exchange" forever even on a
 * perfectly healthy listener. Sent regardless of `outcome.accepted`,
 * since `outcome` always reflects the estimator's current running
 * state (confidence `Unknown` before the first accepted sample), not
 * just this one exchange -- exactly what the host's diagnostics should
 * mirror. Best-effort: a failed send here must not affect local
 * playback, which is why it is swallowed like the other transport
 * sends on this same path (`sendSyncRequest` above).
 */
internal fun ManualListenerTransportController.handleSyncResponse(
    handle: FfiListenerTransportHandle,
    event: FfiListenerTransportEvent.SyncResponseReceived,
) {
    val runtime = playbackRuntime ?: return
    val outcome = runCatching {
        runtime.observeSyncResponse(
            event.correlationId,
            event.t1ListenerSendElapsedMs,
            event.t2HostReceiveElapsedMs,
            event.t3HostSendElapsedMs,
            translateToPumpClock(runtime, event.receivedAtElapsedMs),
        )
    }.getOrElse { error ->
        logger.w("manual.audio.sync_rejected", error.message ?: "sync response rejected")
        return
    }
    // Teed to the durable file as well as logcat: distinguishing an
    // offset-driven rebuffer from a concealment-driven one needs the
    // per-sample offset series, and the offset path increments no
    // counter of its own (its result is discarded in
    // `observe_sync_response`), so this series is the only way to tell
    // the two causes apart on a device whose logcat is unavailable.
    appendDiagnosticsLine(
        "sync accepted=${outcome.accepted} offsetMs=${outcome.offsetMs} " +
            "rttMs=${outcome.roundTripTimeMs} jitterMs=${outcome.jitterMs} " +
            "samples=${outcome.acceptedSampleCount} locked=${outcome.syncLocked} " +
            "confidence=${outcome.confidence} acquisitionRejected=${outcome.acquisitionRejectedSampleCount} " +
            "acquisitionElapsedMs=${outcome.acquisitionElapsedMs} " +
            "acquisitionRttLimitMs=${outcome.acquisitionRttLimitMs} " +
            "degradedLock=${outcome.degradedLock}",
    )
    logger.i(
        "manual.audio.sync_sample",
        "accepted=${outcome.accepted} offsetMs=${outcome.offsetMs} " +
            "skewPpm=${outcome.skewPpm} rttMs=${outcome.roundTripTimeMs} " +
            "samples=${outcome.acceptedSampleCount} syncLocked=${outcome.syncLocked} " +
            "acquisitionRejected=${outcome.acquisitionRejectedSampleCount} " +
            "acquisitionElapsedMs=${outcome.acquisitionElapsedMs} " +
            "acquisitionRttLimitMs=${outcome.acquisitionRttLimitMs} " +
            "degradedLock=${outcome.degradedLock}",
    )
    runCatching {
        handle.sendSynchronizationReport(
            outcome.confidence,
            outcome.offsetMs,
            outcome.roundTripTimeMs,
            outcome.skewPpm,
        )
    }.onFailure { error ->
        logger.w(
            "manual.audio.sync_report_send_failed",
            error.message ?: "synchronization report send failed",
        )
    }
    val current = _connectState.value
    if (current is ManualConnectUiState.Streaming) {
        val acquisitionStatus = when {
            !outcome.syncLocked ->
                "Clock sync: ${outcome.acquisitionRejectedSampleCount} rejected over " +
                    "${outcome.acquisitionElapsedMs}ms (RTT gate " +
                    "${"%.0f".format(outcome.acquisitionRttLimitMs)}ms)"
            outcome.degradedLock ->
                "Clock sync locked after ${outcome.acquisitionRejectedSampleCount} rejected samples " +
                    "using a bounded degraded acquisition gate"
            else -> null
        }
        val playbackState = if (outcome.syncLocked && current.playbackState == PlaybackState.BUFFERING) {
            PlaybackState.PLAYING
        } else {
            current.playbackState
        }
        _connectState.value = current.copy(
            playbackState = playbackState,
            syncStatus = acquisitionStatus,
        )
    }
}
