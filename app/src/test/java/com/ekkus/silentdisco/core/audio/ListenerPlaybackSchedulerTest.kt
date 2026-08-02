package com.ekkus.silentdisco.core.audio

import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.HostTimeMapper
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerPlaybackSchedulerTest {
    @Test
    fun `scheduler tracks packet ordering and playback readiness`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )

        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60))
        val telemetry = scheduler.submit(packet(sequence = 1, hostTimeMs = 40))

        assertThat(scheduler.canStart()).isTrue()
        assertThat(telemetry.packetLossCount).isEqualTo(0)
        assertThat(scheduler.poll(nowLocalTimeMs = 25)?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(scheduler.poll(nowLocalTimeMs = 45)?.packet?.sequenceNumber).isEqualTo(1)
        assertThat(scheduler.poll(nowLocalTimeMs = 65)?.packet?.sequenceNumber).isEqualTo(2)
    }

    @Test
    fun `scheduler rejects invalid packet identity and conceals gaps`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )

        val invalidTelemetry = scheduler.submit(
            packet(sequence = 0, hostTimeMs = 20).copy(sessionId = SessionId("other-session")),
        )
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60))

        val first = scheduler.poll(nowLocalTimeMs = 25)
        val concealed = scheduler.poll(nowLocalTimeMs = 65)

        assertThat(invalidTelemetry.invalidPacketCount).isEqualTo(1)
        assertThat(first?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(concealed?.concealed).isTrue()
        assertThat(concealed?.packet?.sequenceNumber).isEqualTo(1)
        assertThat(scheduler.snapshot().concealedPacketCount).isEqualTo(1)
    }

    @Test
    fun `packet loss counts arrival gaps once and reorder backfill compensates`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )

        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        val afterJump = scheduler.submit(packet(sequence = 5, hostTimeMs = 120))
        val afterBackfill = scheduler.submit(packet(sequence = 3, hostTimeMs = 80))
        val afterInOrder = scheduler.submit(packet(sequence = 6, hostTimeMs = 140))
        val afterMoreInOrder = scheduler.submit(packet(sequence = 7, hostTimeMs = 160))

        assertThat(afterJump.packetLossCount).isEqualTo(4)
        assertThat(afterBackfill.packetLossCount).isEqualTo(3)
        // The old head-distance accounting compounded the count on every
        // subsequent in-order submit; arrival-continuity accounting must not.
        assertThat(afterInOrder.packetLossCount).isEqualTo(3)
        assertThat(afterMoreInOrder.packetLossCount).isEqualTo(3)
    }

    @Test
    fun `concealment repeats the last real packet at reduced volume with smooth edges`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60, payload = constantPayload(amplitude)))
        scheduler.poll(nowLocalTimeMs = 25)

        val concealed = scheduler.poll(nowLocalTimeMs = 65)

        assertThat(concealed?.concealed).isTrue()
        val payload = concealed!!.packet.payload
        // Entry continuity: starts exactly at the previously played sample.
        assertThat(readSample(payload, frame = 0, channel = 0)).isEqualTo(amplitude.toInt())
        // Body: the last real packet repeated at half volume, not silence.
        assertThat(readSample(payload, frame = 480, channel = 0)).isEqualTo(amplitude / 2)
        // Entry ramp blends between the two, no step.
        assertThat(readSample(payload, frame = 120, channel = 0)).isEqualTo(6_000)
        // Exit continuity: tail fades to zero for whatever follows.
        assertThat(readSample(payload, frame = 959, channel = 0)).isEqualTo(0)
    }

    @Test
    fun `consecutive concealments decay toward silence and reset after real audio`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 4, hostTimeMs = 100, payload = constantPayload(amplitude)))
        assertThat(scheduler.poll(nowLocalTimeMs = 425)?.packet?.sequenceNumber).isEqualTo(0)

        val firstConcealed = scheduler.poll(nowLocalTimeMs = 425)
        val secondConcealed = scheduler.poll(nowLocalTimeMs = 425)
        val thirdConcealed = scheduler.poll(nowLocalTimeMs = 425)
        val resumed = scheduler.poll(nowLocalTimeMs = 425)
        scheduler.submit(packet(sequence = 6, hostTimeMs = 140, payload = constantPayload(amplitude)))
        val concealedAfterReset = scheduler.poll(nowLocalTimeMs = 425)

        assertThat(readSample(firstConcealed!!.packet.payload, frame = 480, channel = 0)).isEqualTo(4_000)
        assertThat(readSample(secondConcealed!!.packet.payload, frame = 480, channel = 0)).isEqualTo(2_000)
        assertThat(readSample(thirdConcealed!!.packet.payload, frame = 480, channel = 0)).isEqualTo(1_000)
        assertThat(resumed?.concealed).isFalse()
        assertThat(resumed?.packet?.sequenceNumber).isEqualTo(4)
        assertThat(concealedAfterReset?.concealed).isTrue()
        assertThat(readSample(concealedAfterReset!!.packet.payload, frame = 480, channel = 0)).isEqualTo(4_000)
    }

    @Test
    fun `real audio after a concealment gap fades back in from silence`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60, payload = constantPayload(amplitude)))
        scheduler.poll(nowLocalTimeMs = 25)
        scheduler.poll(nowLocalTimeMs = 65)

        val resumed = scheduler.poll(nowLocalTimeMs = 65)

        assertThat(resumed?.concealed).isFalse()
        assertThat(resumed?.packet?.sequenceNumber).isEqualTo(2)
        val payload = resumed!!.packet.payload
        assertThat(readSample(payload, frame = 0, channel = 0)).isEqualTo(0)
        val midRamp = readSample(payload, frame = 120, channel = 0)
        assertThat(midRamp).isGreaterThan(0)
        assertThat(midRamp).isLessThan(amplitude.toInt())
        // Past the ramp the payload is untouched.
        assertThat(readSample(payload, frame = 300, channel = 0)).isEqualTo(amplitude.toInt())
    }

    @Test
    fun `the first delivered frame of a stream fades in`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40, payload = constantPayload(amplitude)))

        val first = scheduler.poll(nowLocalTimeMs = 25)
        val second = scheduler.poll(nowLocalTimeMs = 45)

        assertThat(readSample(first!!.packet.payload, frame = 0, channel = 0)).isEqualTo(0)
        assertThat(readSample(first.packet.payload, frame = 300, channel = 0)).isEqualTo(amplitude.toInt())
        // Only the first frame fades; steady-state frames pass through intact.
        assertThat(readSample(second!!.packet.payload, frame = 0, channel = 0)).isEqualTo(amplitude.toInt())
    }

    @Test
    fun `wide holes are skipped with a fade-in instead of concealed frame-by-frame`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 15, hostTimeMs = 320, payload = constantPayload(amplitude)))
        scheduler.poll(nowLocalTimeMs = 425)

        val resumed = scheduler.poll(nowLocalTimeMs = 425)

        // Concealing 14 slots would queue 280ms of dead air ahead of the
        // resume content; a wide hole must jump straight to the real frame.
        assertThat(resumed?.concealed).isFalse()
        assertThat(resumed?.packet?.sequenceNumber).isEqualTo(15)
        assertThat(readSample(resumed!!.packet.payload, frame = 0, channel = 0)).isEqualTo(0)
        assertThat(scheduler.snapshot().concealedPacketCount).isEqualTo(0)
    }

    @Test
    fun `an arrival outage is bridged with bounded decaying concealment then stops`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40, payload = constantPayload(amplitude)))
        scheduler.poll(nowLocalTimeMs = 25)
        scheduler.poll(nowLocalTimeMs = 45)

        // Nothing else ever arrives. Concealment bridges the outage with
        // decaying repeats, then stops for good instead of looping forever.
        val frames = mutableListOf<PlaybackFrame?>()
        for (step in 1..30) {
            frames += scheduler.poll(nowLocalTimeMs = 45L + step * 20L)
        }

        val concealedFrames = frames.filterNotNull()
        assertThat(concealedFrames).hasSize(25)
        assertThat(concealedFrames.all { it.concealed }).isTrue()
        assertThat(frames.takeLast(5).all { it == null }).isTrue()
        // Decays: half volume, then quarter, and effectively silent well
        // before the bridge bound.
        assertThat(readSample(concealedFrames[0].packet.payload, frame = 480, channel = 0)).isEqualTo(4_000)
        assertThat(readSample(concealedFrames[1].packet.payload, frame = 480, channel = 0)).isEqualTo(2_000)
        assertThat(readSample(concealedFrames[10].packet.payload, frame = 480, channel = 0)).isEqualTo(amplitude / 256)
    }

    @Test
    fun `no phantom concealment while the next packet is buffered but not yet due`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60))
        scheduler.poll(nowLocalTimeMs = 25)

        // Sequence 1 is sitting in the buffer, due in 13ms. Concealing its
        // slot now would play silence for it and then the real packet too.
        val early = scheduler.poll(nowLocalTimeMs = 27)
        val onTime = scheduler.poll(nowLocalTimeMs = 45)

        assertThat(early).isNull()
        assertThat(onTime?.concealed).isFalse()
        assertThat(onTime?.packet?.sequenceNumber).isEqualTo(1)
    }

    @Test
    fun `drain fades sequence-hole edges and the final tail to zero`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 3, hostTimeMs = 80, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 4, hostTimeMs = 100, payload = constantPayload(amplitude)))

        val drained = scheduler.drainRemaining()

        assertThat(drained.map { it.packet.sequenceNumber }).containsExactly(0L, 1L, 3L, 4L).inOrder()
        val lastFrame = 959
        // Contiguous middle edges untouched.
        assertThat(readSample(drained[0].packet.payload, frame = lastFrame, channel = 0)).isEqualTo(amplitude.toInt())
        // Hole between 1 and 3: fade out into it, fade in out of it.
        assertThat(readSample(drained[1].packet.payload, frame = 0, channel = 0)).isEqualTo(amplitude.toInt())
        assertThat(readSample(drained[1].packet.payload, frame = lastFrame, channel = 0)).isEqualTo(0)
        assertThat(readSample(drained[2].packet.payload, frame = 0, channel = 0)).isEqualTo(0)
        assertThat(readSample(drained[2].packet.payload, frame = 300, channel = 0)).isEqualTo(amplitude.toInt())
        // Final frame ends at zero so engine stop never cuts mid-waveform.
        assertThat(readSample(drained[3].packet.payload, frame = 0, channel = 0)).isEqualTo(amplitude.toInt())
        assertThat(readSample(drained[3].packet.payload, frame = lastFrame, channel = 0)).isEqualTo(0)
    }

    @Test
    fun `stale arrivals whose slot was already concealed are dropped, not played out of order`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        scheduler.submit(packet(sequence = 3, hostTimeMs = 80))
        // A write-lead caller polls ahead of real time: slots 0-3 are all
        // emitted (0 and 3 real, 1 and 2 concealed) while wall-clock is
        // still early.
        assertThat(scheduler.poll(nowLocalTimeMs = 425)?.packet?.sequenceNumber).isEqualTo(0)
        assertThat(scheduler.poll(nowLocalTimeMs = 425)?.concealed).isTrue()
        assertThat(scheduler.poll(nowLocalTimeMs = 425)?.concealed).isTrue()
        assertThat(scheduler.poll(nowLocalTimeMs = 425)?.packet?.sequenceNumber).isEqualTo(3)

        // Sequence 1 arrives afterwards, still within its submit window. Its
        // slot already played (concealed), so it must never be delivered --
        // the poll instead moves on (here: the outage bridge concealing the
        // next slot forward, since nothing after 3 ever arrives).
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))
        val next = scheduler.poll(nowLocalTimeMs = 425)

        assertThat(next?.packet?.sequenceNumber).isNotEqualTo(1)
        assertThat(next?.concealed).isTrue()
        assertThat(scheduler.snapshot().lateDropCount).isEqualTo(1)
    }

    @Test
    fun `nextDeadlineMs reports the earliest buffered deadline`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        assertThat(scheduler.nextDeadlineMs()).isNull()
        scheduler.submit(packet(sequence = 1, hostTimeMs = 40))
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20))
        assertThat(scheduler.nextDeadlineMs()).isEqualTo(20)
    }

    @Test
    fun `drain fades in when its first frame does not continue the live stream`() {
        val scheduler = ListenerPlaybackScheduler(
            mapper = HostTimeMapper(offsetMs = 0.0),
            thresholds = PlaybackThresholds(startupBufferMs = 20),
            expectedSessionId = SessionId("session"),
            expectedStreamId = StreamId("stream"),
            nowProvider = { 0L },
        )
        val amplitude: Short = 8_000
        scheduler.submit(packet(sequence = 0, hostTimeMs = 20, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 2, hostTimeMs = 60, payload = constantPayload(amplitude)))
        scheduler.submit(packet(sequence = 3, hostTimeMs = 80, payload = constantPayload(amplitude)))
        scheduler.poll(nowLocalTimeMs = 25)

        val drained = scheduler.drainRemaining()

        assertThat(drained.map { it.packet.sequenceNumber }).containsExactly(2L, 3L).inOrder()
        // Last live-delivered was 0; the drain starts at 2 -- a hole.
        assertThat(readSample(drained[0].packet.payload, frame = 0, channel = 0)).isEqualTo(0)
        assertThat(readSample(drained[0].packet.payload, frame = 300, channel = 0)).isEqualTo(amplitude.toInt())
    }

    private fun packet(sequence: Long, hostTimeMs: Long, payload: ByteArray = ByteArray(64)) = AudioPacket(
        version = 1,
        sessionId = SessionId("session"),
        streamId = StreamId("stream"),
        sequenceNumber = sequence,
        codec = "pcm16le",
        sampleRate = 48_000,
        channelCount = 2,
        samplesPerPacket = 960,
        firstSampleIndex = sequence * 960,
        hostPresentationTimeMs = hostTimeMs,
        payload = payload,
        checksum = null,
    )

    /** 960 stereo PCM16LE frames, every sample set to [value]. */
    private fun constantPayload(value: Short): ByteArray {
        val payload = ByteArray(960 * 2 * 2)
        for (index in 0 until payload.size step 2) {
            payload[index] = (value.toInt() and 0xFF).toByte()
            payload[index + 1] = ((value.toInt() shr 8) and 0xFF).toByte()
        }
        return payload
    }

    private fun readSample(payload: ByteArray, frame: Int, channel: Int): Int {
        val byteIndex = frame * 4 + channel * 2
        val low = payload[byteIndex].toInt() and 0xFF
        val high = payload[byteIndex + 1].toInt() and 0xFF
        return ((high shl 8) or low).toShort().toInt()
    }
}
