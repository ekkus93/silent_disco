package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.uniffi.FfiAudioPacket
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.mockito.ArgumentCaptor
import org.mockito.Mockito.mock
import org.mockito.Mockito.never
import org.mockito.Mockito.times
import org.mockito.Mockito.verify
import org.mockito.kotlin.any

/**
 * Audio that overtakes its stream's StreamStart is held, not dropped, and
 * handed to that stream's runtime when it opens (startup-counter
 * investigation, 2026-11-19: the A54's first song lost 24 such packets).
 */
class ManualEarlyAudioTest {
    private val controller = ManualListenerTransportController(closeNativeOutput = {})
    private val session = SessionId("session-early")

    private fun audio(stream: String, sequence: Long) = FfiListenerTransportEvent.AudioReceived(
        streamId = stream,
        sequence = sequence.toULong(),
        sampleRate = 48_000u,
        channels = 2u,
        samplesPerPacket = 240u,
        firstSampleIndex = (sequence * 240).toULong(),
        hostPresentationTimeMs = (1_000 + sequence * 5).toULong(),
        payload = ByteArray(960),
    )

    @Test
    fun audioBeforeTheRuntimeIsReplayedInOrderToItsOwnStreamOnly() {
        controller.sessionId = session
        controller.handleAudioReceived(audio("old-stream", 9))
        (0L..2L).forEach { controller.handleAudioReceived(audio("stream-1", it)) }

        val runtime = mock(FfiListenerPlaybackHandle::class.java)
        assertThat(controller.replayEarlyAudio(runtime, session, "stream-1")).isEqualTo(3)
        val packets = ArgumentCaptor.forClass(FfiAudioPacket::class.java)
        verify(runtime, times(3)).submitPacket(packets.capture() ?: audioPacketPlaceholder, any(), any())
        assertThat(packets.allValues.map { it.sequence }).containsExactly(0uL, 1uL, 2uL).inOrder()

        // Replayed once: the hold is empty afterwards.
        assertThat(controller.replayEarlyAudio(runtime, session, "stream-1")).isEqualTo(0)
    }

    @Test
    fun theHoldKeepsTheNewestPacketsWithinItsBound() {
        controller.sessionId = session
        repeat(MAX_EARLY_AUDIO_PACKETS + 10) { controller.handleAudioReceived(audio("stream-1", it.toLong())) }
        assertThat(controller.earlyAudio).hasSize(MAX_EARLY_AUDIO_PACKETS)
        assertThat(controller.earlyAudio.first().sequence).isEqualTo(10uL)
        assertThat(controller.earlyAudioDropped).isEqualTo(10)
    }

    @Test
    fun aTeardownForgetsHeldAudioAndAudioWithoutASessionIsNotHeld() {
        controller.handleAudioReceived(audio("stream-1", 0))
        assertThat(controller.earlyAudio).isEmpty()

        controller.sessionId = session
        controller.handleAudioReceived(audio("stream-1", 1))
        controller.stopPlayback()
        assertThat(controller.earlyAudio).isEmpty()
        val runtime = mock(FfiListenerPlaybackHandle::class.java)
        controller.replayEarlyAudio(runtime, session, "stream-1")
        verify(runtime, never()).submitPacket(any(), any(), any())
    }

    private val audioPacketPlaceholder = FfiAudioPacket(0uL, 48_000u, 2u, 240u, 0uL, 0uL, ByteArray(0))
}
