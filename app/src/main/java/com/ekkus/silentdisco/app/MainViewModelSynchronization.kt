package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ListenerSyncController
import com.ekkus.silentdisco.core.sync.SyncMaintenanceConfig
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

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
        val controller = listenerSyncController ?: createSyncController(SessionId(session.id)).also {
            listenerSyncController = it
        }
        val request = controller.newProbe()
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

    internal fun MainViewModel.applySyncResponse(response: SyncResponsePacket) {
        if (_uiState.value.selectedSession?.id != response.sessionId.value) return
        val expectedCorrelationId = pendingSyncCorrelationId
        if (expectedCorrelationId != null && response.correlationId != expectedCorrelationId) return
        pendingSyncCorrelationId = null
        val controller = listenerSyncController ?: ListenerSyncController(response.sessionId).also {
            listenerSyncController = it
        }
        val syncState = controller.onResponse(response).copy(
            resyncCount = _uiState.value.listenerSyncState.resyncCount + 1,
        )
        val shouldResync = controller.shouldResync(state = syncState)
        logger.i(
            "sync.sample",
            "offset=${"%.2f".format(syncState.offsetMs)} rtt=${"%.2f".format(syncState.rttMs)} jitter=${"%.2f".format(syncState.jitterMs)}",
        )
        if (shouldResync && !_uiState.value.connectionProgress.synced) {
            handleSyncFailure("Unable to establish a stable sync estimate")
            return
        }
        _uiState.value = _uiState.value.copy(
            listenerSyncState = syncState,
            listenerState = if (shouldResync) ListenerLifecycleState.DESYNCED else _uiState.value.listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(synced = !shouldResync),
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

    internal fun MainViewModel.createSyncController(sessionId: SessionId): ListenerSyncController {
        val tuning = _uiState.value.tuningSettings
        return ListenerSyncController(
            sessionId = sessionId,
            estimator = ClockSyncEstimator(maxSamples = tuning.syncSampleWindow),
            config = SyncMaintenanceConfig(
                cadenceMs = tuning.syncCadenceMs,
                driftThresholdMs = tuning.syncDriftThresholdMs,
                sampleHistorySize = tuning.syncSampleWindow,
            ),
        )
    }
