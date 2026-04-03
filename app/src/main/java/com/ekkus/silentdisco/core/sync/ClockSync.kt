package com.ekkus.silentdisco.core.sync

import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.SyncState
import kotlin.math.abs

data class ClockSyncSample(
    val t1: Long,
    val t2: Long,
    val t3: Long,
    val t4: Long,
) {
    val rttMs: Double = (t4 - t1 - (t3 - t2)).toDouble()
    val offsetMs: Double = ((t2 - t1) + (t3 - t4)) / 2.0
}

class ClockSyncEstimator(
    private val maxSamples: Int = 12,
    private val maxAcceptedRttMs: Double = 200.0,
) {
    private val samples = ArrayDeque<ClockSyncSample>()

    fun observe(sample: ClockSyncSample): SyncState {
        if (sample.rttMs in 0.0..maxAcceptedRttMs) {
            if (samples.size == maxSamples) {
                samples.removeFirst()
            }
            samples.addLast(sample)
        }
        return snapshot()
    }

    fun snapshot(): SyncState {
        if (samples.isEmpty()) {
            return SyncState()
        }
        val sorted = samples.sortedBy { it.rttMs }
        val bestHalf = sorted.take((sorted.size / 2).coerceAtLeast(1))
        val offset = bestHalf.map { it.offsetMs }.average()
        val rtt = bestHalf.map { it.rttMs }.average()
        val jitter = if (bestHalf.size == 1) 0.0 else {
            val mean = bestHalf.map { it.offsetMs }.average()
            bestHalf.map { abs(it.offsetMs - mean) }.average()
        }
        return SyncState(
            offsetMs = offset,
            rttMs = rtt,
            jitterMs = jitter,
            confidence = when {
                rtt <= 20 && jitter <= 2 -> SyncQualityBadge.EXCELLENT
                rtt <= 50 && jitter <= 5 -> SyncQualityBadge.GOOD
                rtt <= 90 && jitter <= 12 -> SyncQualityBadge.FAIR
                else -> SyncQualityBadge.POOR
            },
        )
    }
}

data class HostTimeMapper(
    val offsetMs: Double,
    val skewPpm: Double = 0.0,
) {
    fun hostToLocal(hostElapsedMs: Long): Long {
        val skewAdjusted = hostElapsedMs + (hostElapsedMs * (skewPpm / 1_000_000.0))
        return (skewAdjusted - offsetMs).toLong()
    }

    fun localToHost(localElapsedMs: Long): Long {
        val adjusted = localElapsedMs + offsetMs
        return adjusted.toLong()
    }
}
