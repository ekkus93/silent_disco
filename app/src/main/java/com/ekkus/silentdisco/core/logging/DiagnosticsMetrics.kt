package com.ekkus.silentdisco.core.logging

class DiagnosticsMetrics {
    private val counters = LinkedHashMap<String, Long>()
    private val timers = LinkedHashMap<String, Double>()

    fun increment(name: String, amount: Long = 1) {
        counters[name] = (counters[name] ?: 0L) + amount
    }

    fun recordTiming(name: String, durationMs: Double) {
        timers[name] = durationMs
    }

    fun snapshotCounters(): Map<String, Long> = counters.toMap()
    fun snapshotTimings(): Map<String, Double> = timers.toMap()
}
