package com.ekkus.silentdisco.core.persistence

/**
 * Stable keys used by the pre-Rust Android SharedPreferences implementation.
 *
 * The Rust migration fixtures depend on these values to preserve existing user
 * settings and trust records when SQLite becomes authoritative.
 */
internal object LegacyPreferencesContract {
    const val SYNC_SAMPLE_WINDOW = "tuning:syncSampleWindow"
    const val SYNC_CADENCE_MS = "tuning:syncCadenceMs"
    const val STARTUP_BUFFER_MS = "tuning:startupBufferMs"
    const val LATE_PACKET_THRESHOLD_MS = "tuning:latePacketThresholdMs"
    const val HARD_RESYNC_THRESHOLD_MS = "tuning:hardResyncThresholdMs"
    const val SYNC_DRIFT_THRESHOLD_BITS = "tuning:syncDriftThresholdBits"

    fun trustedDeviceKey(deviceId: String): String = "trusted:$deviceId"
}
