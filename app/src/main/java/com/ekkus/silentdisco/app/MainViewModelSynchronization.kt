package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.rust.RustCoreBridge
import com.ekkus.silentdisco.core.rust.RustSyncConfidence
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Protocol version for the pre-runtime NTP-style sync request/response exchange. */
private const val LISTENER_SYNC_REQUEST_VERSION = 1

/** Mirrors [MainViewModelRustListener.kt]'s `FfiSyncConfidence.toAppSyncQuality()` for the standalone estimator. */
internal fun RustSyncConfidence.toAppSyncQuality(): SyncQualityBadge = when (this) {
    RustSyncConfidence.UNKNOWN -> SyncQualityBadge.UNKNOWN
    RustSyncConfidence.POOR -> SyncQualityBadge.POOR
    RustSyncConfidence.FAIR -> SyncQualityBadge.FAIR
    RustSyncConfidence.GOOD -> SyncQualityBadge.GOOD
    RustSyncConfidence.EXCELLENT -> SyncQualityBadge.EXCELLENT
}

/**
 * Preserves the pre-Rust drift-threshold gate from
 * `ListenerSyncController.shouldResync`. Its other half (re-probing once
 * `cadenceMs` has elapsed since the last sample) is dropped deliberately, not
 * lost: it was only ever evaluated synchronously right after a fresh sample
 * was recorded, so elapsed time at that call site was always ~0 and the
 * check was already an inert no-op in production; the periodic probe cadence
 * itself is owned by [startPeriodicListenerResync], unrelated to this
 * per-sample gate. Extracted as a pure function so it stays covered by a JVM
 * unit test even though `RustSyncEstimator` itself cannot load in one (see
 * `SyncEstimateMappingTest`).
 */
internal fun shouldResyncForOffset(offsetMs: Double, driftThresholdMs: Double): Boolean =
    kotlin.math.abs(offsetMs) > driftThresholdMs

    internal fun MainViewModel.requestListenerSyncProbe(source: String) {
        val session = _uiState.value.selectedSession
        if (session == null) {
            val message = "Join a session before requesting manual resync"
            _uiState.value = _uiState.value.copy(lastError = message)
            diagnosticsStore.updateListener { it.copy(lastError = message) }
            refreshListenerDiagnostics()
            return
        }

        // Once playback exists, the runtime owns the estimate, so the probe
        // must be registered with it and stamped from its clock -- a
        // timestamp from any other epoch would make the estimate and the
        // playback timeline disagree.
        val runtime = listenerPlayback
        if (runtime != null) {
            sendRuntimeSyncProbe(runtime, source)
            return
        }
        // Kotlin still owns correlation-id/timestamp generation for this
        // pre-runtime probe, exactly like sendRuntimeSyncProbe does for the
        // runtime-owned path -- only the *estimate* computed from the
        // matching response belongs to Rust (see applySyncResponse). The
        // Rust-owned RustSyncEstimator has no notion of a wire correlation
        // id at all (it takes four already-correlated timestamps and
        // internally tracks its own private probe bookkeeping), so there is
        // nothing for Kotlin to hand off here.
        val request = newListenerSyncProbeRequest(SessionId(session.id))
        pendingSyncCorrelationId = request.correlationId

        if (wifiDirectService.snapshot.value.state == TransportConnectionState.CONNECTED) {
            val nextState = nextStateForSyncProbe(_uiState.value.listenerState)
            val nextProgressState = if (nextState == ListenerLifecycleState.SYNCING_CLOCK) {
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
                    listenerTransportController.sendSyncRequest(
                        request.correlationId.toULong(),
                        request.t1ListenerSendElapsedMs.toULong(),
                    )
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

    /**
     * Registers one probe with the playback runtime and sends it with the
     * same timestamp the runtime recorded.
     */
    private fun MainViewModel.sendRuntimeSyncProbe(runtime: FfiListenerPlaybackHandle, source: String) {
        val correlationId = nextSyncCorrelationId++
        val sendTimeMs = runtime.nowMs()
        runCatching { runtime.beginSyncProbe(correlationId.toULong(), sendTimeMs) }
            .onFailure { error ->
                handleSyncFailure(error.message ?: "Failed to register sync probe")
                return
            }
        viewModelScope.launch {
            runCatching {
                listenerTransportController.sendSyncRequest(correlationId.toULong(), sendTimeMs)
            }.onSuccess {
                _uiState.value = _uiState.value.copy(lastMessage = "$source sync probe sent", lastError = null)
            }.onFailure { error ->
                handleSyncFailure(error.message ?: "Failed to send sync probe")
            }
        }
    }

    /**
     * Builds one pre-runtime sync probe request. Mirrors [sendRuntimeSyncProbe]'s
     * ownership split: Kotlin generates the correlation id and stamps t1 from
     * its own monotonic clock (`SystemClock.elapsedRealtime()`, the same
     * clock [applySyncResponse] stamps t4 from below); the Rust-owned
     * estimator only ever sees the resulting four-timestamp exchange, never
     * this id.
     */
    private fun MainViewModel.newListenerSyncProbeRequest(sessionId: SessionId): SyncRequestPacket {
        val correlationId = nextSyncCorrelationId++
        val sendTimeMs = SystemClock.elapsedRealtime()
        return SyncRequestPacket(
            version = LISTENER_SYNC_REQUEST_VERSION,
            sessionId = sessionId,
            correlationId = correlationId,
            t1ListenerSendElapsedMs = sendTimeMs,
        )
    }

    /**
     * Feeds one completed four-timestamp exchange to the Rust-owned
     * standalone estimator ([RustCoreBridge.openSyncEstimator]) and maps its
     * result onto the same [SyncState]/diagnostics fields the pre-Rust local
     * estimator used to populate -- this is the pre-runtime sibling of
     * [MainViewModelRustListener.applyRuntimeSyncOutcome], which does the
     * equivalent mapping for the runtime-owned estimator once playback
     * exists.
     */
    internal fun MainViewModel.applySyncResponse(response: SyncResponsePacket) {
        if (_uiState.value.selectedSession?.id != response.sessionId.value) return
        val expectedCorrelationId = pendingSyncCorrelationId
        if (expectedCorrelationId != null && response.correlationId != expectedCorrelationId) return
        pendingSyncCorrelationId = null

        val localReceiveTimeMs = SystemClock.elapsedRealtime()
        val observation = runCatching {
            val estimator = listenerSyncEstimator ?: RustCoreBridge.openSyncEstimator(
                maxSamples = _uiState.value.tuningSettings.syncSampleWindow,
            ).also { listenerSyncEstimator = it }
            estimator.observe(
                t1LocalSendMs = response.t1ListenerSendElapsedMs,
                t2HostReceiveMs = response.t2HostReceiveElapsedMs,
                t3HostSendMs = response.t3HostSendElapsedMs,
                t4LocalReceiveMs = localReceiveTimeMs,
            )
        }.getOrElse { error ->
            // Unlike a merely-rejected sample (too-high RTT, reported as
            // observation.accepted == false below, never an exception), a
            // thrown exception here means the estimator itself could not be
            // created/used at all (native library unavailable, an
            // impossible timestamp ordering, a protocol-level bridge
            // failure) -- a real, reportable failure, not routine noise.
            handleSyncFailure(error.message ?: "Rust synchronization estimator failed to process the sample")
            return
        }

        logger.i(
            "sync.sample",
            "accepted=${observation.accepted} rawRtt=${"%.2f".format(observation.roundTripTimeMs)} " +
                "acquisitionRejected=${observation.acquisitionRejectedSampleCount} " +
                "acquisitionElapsedMs=${observation.acquisitionElapsedMs} " +
                "acquisitionRttLimitMs=${"%.0f".format(observation.acquisitionRttLimitMs)} " +
                "degradedLock=${observation.degradedLock}",
        )
        if (!observation.accepted) {
            val message =
                "Clock sync: ${observation.acquisitionRejectedSampleCount} rejected over " +
                    "${observation.acquisitionElapsedMs}ms (RTT gate " +
                    "${"%.0f".format(observation.acquisitionRttLimitMs)}ms)"
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.SYNCING_CLOCK,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.SYNCING_CLOCK,
                    synced = false,
                    playing = false,
                ),
                lastMessage = message,
                lastError = null,
            )
            diagnosticsStore.updateListener { it.copy(lastError = null) }
            refreshListenerDiagnostics()
            return
        }

        // observation.snapshot is the filtered, multi-sample running
        // estimate (mirrors what the old ClockSyncEstimator.snapshot()
        // returned) -- not the single raw sample, which is noisy and, for a
        // rejected sample, not even folded into the estimate at all.
        val snapshot = observation.snapshot
        val syncState = SyncState(
            offsetMs = snapshot.offsetMs,
            rttMs = snapshot.roundTripTimeMs,
            jitterMs = snapshot.jitterMs,
            skewPpm = snapshot.skewPpm,
            confidence = snapshot.confidence.toAppSyncQuality(),
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        val shouldResync = shouldResyncForOffset(
            offsetMs = syncState.offsetMs,
            driftThresholdMs = _uiState.value.tuningSettings.syncDriftThresholdMs,
        )
        if (shouldResync && !_uiState.value.connectionProgress.synced) {
            handleSyncFailure("Unable to establish a stable sync estimate")
            return
        }
        _uiState.value = _uiState.value.copy(
            listenerSyncState = syncState,
            listenerState = if (shouldResync) ListenerLifecycleState.DESYNCED else _uiState.value.listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(synced = !shouldResync),
            lastMessage = if (!shouldResync && observation.degradedLock) {
                "Clock sync locked after ${observation.acquisitionRejectedSampleCount} rejected samples " +
                    "using a bounded degraded acquisition gate"
            } else {
                _uiState.value.lastMessage
            },
        )
        diagnosticsStore.updateListener {
            it.copy(
                hostOffsetMs = syncState.offsetMs,
                rttMs = syncState.rttMs,
                jitterMs = syncState.jitterMs,
                resyncCount = syncState.resyncCount,
                metricsSummary = summarizeMetrics(),
                lastError = if (shouldResync) "Sync drift exceeded threshold" else null,
            )
        }
        metrics.increment("sync_sample")
        metrics.recordTiming("sync_rtt_ms", syncState.rttMs)
        if (shouldResync) {
            metrics.increment("playback_desync")
            logger.w("playback.desync", "Resync threshold exceeded")
        }
        refreshListenerDiagnostics()
        refreshHostDiagnostics()
    }

    internal fun MainViewModel.handleSyncFailure(message: String) {
        logger.w("sync.error", message)
        metrics.increment("sync_establish_failure")
        // Reported into Rust (rather than written locally) so a later Rust
        // snapshot cannot silently revert this back to an earlier state.
        listenerCoreController?.transportFailed(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = PlaybackState.ERROR,
            connectionProgress = _uiState.value.connectionProgress.copy(
                buffered = false,
                playing = false,
            ),
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

/** Probe interval while the listener clock is still unlocked. */
private const val SYNC_ACQUIRE_CADENCE_MS = 250L

    internal fun MainViewModel.startPeriodicListenerResync() {
        resyncJob?.cancel()
        resyncJob = viewModelScope.launch {
            while (shouldKeepResyncing()) {
                // Until the clock locks, playback produces nothing at all and
                // the estimator discards any sample whose round trip is too
                // long, so the steady cadence would turn a few unlucky probes
                // into seconds of silence. Probe hard until it locks.
                val locked = listenerPlayback?.diagnostics()?.syncLocked == true
                delay(
                    if (locked) {
                        _uiState.value.tuningSettings.syncCadenceMs
                    } else {
                        SYNC_ACQUIRE_CADENCE_MS
                    },
                )
                if (_uiState.value.canManualResync()) {
                    requestListenerSyncProbe(source = "Periodic listener resync")
                }
                wifiDirectService.recordHeartbeat()
                if (wifiDirectService.snapshot.value.state == TransportConnectionState.DISCONNECTED ||
                    wifiDirectService.snapshot.value.state == TransportConnectionState.FAILED
                ) {
                    handleListenerDisconnect("Transport disconnected during playback")
                    return@launch
                }
            }
        }
    }

    /**
     * Closes and clears the Rust-owned pre-runtime sync estimator, if one is
     * open. `RustSyncEstimator` holds a real native handle (unlike the
     * pre-Rust `ListenerSyncController` it replaces, which was a plain
     * Kotlin object the garbage collector could reclaim on its own), so
     * every place a session ends or the estimator's configuration changes
     * must release it explicitly rather than just dropping the reference.
     */
    internal fun MainViewModel.closeListenerSyncEstimator() {
        val estimator = listenerSyncEstimator ?: return
        listenerSyncEstimator = null
        runCatching { estimator.close() }.onFailure { error ->
            logger.w("sync.estimator_close_failed", error.message ?: "sync estimator failed to close cleanly")
        }
    }
