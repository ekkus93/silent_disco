package com.ekkus.silentdisco.core.audio

data class PlaybackThresholds(
    val startupBufferMs: Long = 400,
    val softCorrectionThresholdMs: Long = 40,
    val hardResyncThresholdMs: Long = 120,
)

/**
 * One frame handed to a [PlaybackEngine], with the local time it is due.
 *
 * Production listener and host-monitor playback no longer produce these —
 * the Rust runtime owns scheduling and writes to its render ring directly.
 * This legacy shape remains only for isolated adapter regression tests.
 */
data class PlaybackFrame(
    val packet: com.ekkus.silentdisco.core.protocol.AudioPacket,
    val localDeadlineMs: Long,
    val concealed: Boolean = false,
)

interface PlaybackEngine {
    fun start(format: AudioFormatSpec = AudioFormatSpec()): String
    fun write(frame: PlaybackFrame): Long
    fun setVolume(value: Float)
    fun playbackPositionMs(frame: PlaybackFrame): Long
    fun stop()
}
