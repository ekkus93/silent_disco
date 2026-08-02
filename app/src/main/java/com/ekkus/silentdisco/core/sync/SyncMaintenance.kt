package com.ekkus.silentdisco.core.sync

import android.os.SystemClock
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.SyncRequestPacket
import com.ekkus.silentdisco.core.protocol.SyncResponsePacket

class HostTimingService {
    fun createResponse(request: SyncRequestPacket): SyncResponsePacket {
        val receiveTime = SystemClock.elapsedRealtime()
        val sendTime = SystemClock.elapsedRealtime()
        return SyncResponsePacket(
            version = request.version,
            sessionId = request.sessionId,
            correlationId = request.correlationId,
            t1ListenerSendElapsedMs = request.t1ListenerSendElapsedMs,
            t2HostReceiveElapsedMs = receiveTime,
            t3HostSendElapsedMs = sendTime,
        )
    }
}

data class SyncMaintenanceConfig(
    val cadenceMs: Long = 2_000,
    val driftThresholdMs: Double = 18.0,
    val sampleHistorySize: Int = 24,
)

class ListenerSyncController(
    private val sessionId: SessionId,
    private val estimator: ClockSyncEstimator = ClockSyncEstimator(),
    private val config: SyncMaintenanceConfig = SyncMaintenanceConfig(),
    private val nowProvider: () -> Long = { SystemClock.elapsedRealtime() },
) {
    private val samples = mutableListOf<Pair<Long, Double>>()
    private var lastSyncAtMs: Long = 0

    fun newProbe(correlationId: Long = SystemClock.elapsedRealtimeNanos()): SyncRequestPacket {
        val sendTime = nowProvider()
        return SyncRequestPacket(
            version = 1,
            sessionId = sessionId,
            correlationId = correlationId,
            t1ListenerSendElapsedMs = sendTime,
        )
    }

    fun onResponse(response: SyncResponsePacket, localReceiveTimeMs: Long = nowProvider()): SyncState {
        val state = estimator.observe(
            ClockSyncSample(
                t1 = response.t1ListenerSendElapsedMs,
                t2 = response.t2HostReceiveElapsedMs,
                t3 = response.t3HostSendElapsedMs,
                t4 = localReceiveTimeMs,
            ),
        )
        // Before any sample is ever accepted, `state.offsetMs` is just the
        // estimator's all-zero default -- not a real measurement. Recording
        // that placeholder here would let the very first real, huge offset
        // (host and listener epochs are unrelated, so realistic offsets are
        // routinely hundreds of millions of ms) enter this regression right
        // next to a run of zeros, producing a spurious near-vertical slope
        // that -- multiplied by 1,000,000 to express as ppm -- explodes into
        // a physically impossible skew (observed: -5e10 ppm; real clocks
        // drift by at most a few hundred ppm), which then corrupts the
        // whole stream's frozen `HostTimeMapper`. Only track real samples.
        if (state.confidence != SyncQualityBadge.UNKNOWN) {
            samples += localReceiveTimeMs to state.offsetMs
            if (samples.size > config.sampleHistorySize) {
                samples.removeAt(0)
            }
        }
        lastSyncAtMs = localReceiveTimeMs
        return state.copy(skewPpm = estimateSkewPpm())
    }

    fun shouldResync(nowMs: Long = nowProvider(), state: SyncState): Boolean {
        val cadenceElapsed = nowMs - lastSyncAtMs >= config.cadenceMs
        val driftExceeded = kotlin.math.abs(state.offsetMs) > config.driftThresholdMs
        return cadenceElapsed || driftExceeded
    }

    private fun estimateSkewPpm(): Double {
        if (samples.size < 3) return 0.0
        val xMean = samples.map { it.first.toDouble() }.average()
        val yMean = samples.map { it.second }.average()
        val numerator = samples.sumOf { (x, y) -> (x - xMean) * (y - yMean) }
        val denominator = samples.sumOf { (x, _) -> (x - xMean) * (x - xMean) }
        if (denominator == 0.0) return 0.0
        val slopeMsPerMs = numerator / denominator
        return slopeMsPerMs * 1_000_000.0
    }
}
