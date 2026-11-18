package com.ekkus.silentdisco.app.controller

import android.net.Uri
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.app.toFfiAudioSource
import com.ekkus.silentdisco.app.toFfiHostDraft
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.UniFfiHostCoreController
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiHostLifecycle
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiQueueEntry
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.io.FileInputStream
import java.util.Collections
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Queue edits racing the producer's successor reservation on the phone, on
 * the path the Android host uses in production (gapless TODO §5): the real
 * Rust core through [UniFfiHostCoreController], the authoritative producer it
 * opens ([HostCoreController.openHostPlaylist]), songs staged by
 * [HostSourceStaging], and the edit applied to the producer by
 * [HostPlaybackController.retarget] from the actor's snapshot, as
 * `MainViewModel` does. Before this, the only Android retarget test used the
 * non-authoritative producer.
 *
 * With 5 ms packets the 40-packet current track backpressures the producer's
 * 32-frame queue, so an edit before the consumer reads lands before the
 * reservation deterministically.
 */
@RunWith(AndroidJUnit4::class)
class HostAuthoritativeQueueEditDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val folder = File(context.cacheDir, "authoritative-edit-${UUID.randomUUID()}")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val errors = Collections.synchronizedList(mutableListOf<String>())
    private lateinit var core: HostCoreController
    private lateinit var staging: HostSourceStaging
    private lateinit var playback: HostPlaybackController

    @Before
    fun setUp() {
        folder.mkdirs()
        core = UniFfiHostCoreController("authoritative-edit-host")
        staging = HostSourceStaging(
            cacheDir = { folder },
            openInput = { uri -> FileInputStream(File(requireNotNull(uri.path))) },
        )
        playback = HostPlaybackController(scope, staging)
        scope.launch {
            core.notifications.collect { notification ->
                when (notification) {
                    is FfiCoreNotification.PlatformEffect -> {
                        val effect = notification.effect
                        if (effect is FfiPlatformEffect.StartAdvertising) {
                            core.platformOperationSucceeded(
                                effect.operationId,
                                FfiPlatformCompletion.AdvertisingStarted,
                            )
                        }
                    }
                    is FfiCoreNotification.Error -> errors += notification.error.message
                    else -> Unit
                }
            }
        }
    }

    @After
    fun tearDown() {
        runCatching { playback.close() }
        runCatching { core.close() }
        scope.cancel()
        folder.deleteRecursively()
    }

    @Test
    fun anEditBeforeTheReservationRetargetsAndOnlyTheReplacementIsReservedAndHeard() = runBlocking {
        startPlaying(listOf("obsolete" to 6_000, "replacement" to 12_000))
        val stream = prepare()

        core.removeHostQueueEntry("obsolete")
        playback.retarget(awaitSnapshot("the removal") { it.hostQueue.size == 2 })

        val drained = drain(stream)
        assertThat(drained.boundaries).containsExactly("replacement" to CURRENT_FRAMES)
        drained.assertPcm(mapOf(0L to 3_000, CURRENT_FRAMES to 12_000), total = CURRENT_FRAMES + SUCCESSOR_FRAMES)
        assertThat(snapshot().hostQueueNextBoundary?.entryId).isEqualTo("replacement")
        assertThat(errors).isEmpty()
    }

    @Test
    fun anEditDuringAPauseBeforeTheReservationRetargetsAndPlaysAfterResume() = runBlocking {
        startPlaying(listOf("obsolete" to 6_000, "replacement" to 12_000))
        val stream = prepare()

        core.transitionPlaybackState(FfiPlaybackState.PAUSED)
        core.removeHostQueueEntry("obsolete")
        playback.retarget(awaitSnapshot("the paused removal") { it.hostQueue.size == 2 })
        core.transitionPlaybackState(FfiPlaybackState.PLAYING)

        val drained = drain(stream)
        assertThat(drained.boundaries).containsExactly("replacement" to CURRENT_FRAMES)
        drained.assertPcm(mapOf(0L to 3_000, CURRENT_FRAMES to 12_000), total = CURRENT_FRAMES + SUCCESSOR_FRAMES)
        assertThat(snapshot().hostQueueNextBoundary?.entryId).isEqualTo("replacement")
        assertThat(errors).isEmpty()
    }

    @Test
    fun removingTheSuccessorTheProducerAlreadyReservedIsRefusedAndItStillPlays() = runBlocking {
        startPlaying(listOf("reserved" to 12_000))
        val stream = prepare()

        // Read until the producer reserves, pausing for the actor's snapshot
        // so the boundary itself stays unread.
        val early = mutableListOf<FfiPlaylistFrame>()
        while (snapshotWithin(100) { it.hostQueueNextBoundary != null } == null) {
            assertThat(early.size).isLessThan(40)
            stream.nextFrame(200uL)?.let { frame ->
                assertThat(frame).isNotInstanceOf(FfiPlaylistFrame.TrackBoundary::class.java)
                early += frame
            }
        }
        val revision = snapshot().hostQueueRevision
        val refused = try {
            core.removeHostQueueEntry("reserved")
            false
        } catch (_: TimeoutCancellationException) {
            true // the actor refused it, so no newer snapshot arrived
        }
        assertThat(refused).isTrue()
        assertThat(errors.single()).contains("already scheduled next track cannot be removed")
        assertThat(snapshot().hostQueueRevision).isEqualTo(revision)
        assertThat(snapshot().hostQueue.map { it.entryId }).contains("reserved")

        val drained = drain(stream, alreadyRead = early)
        assertThat(drained.boundaries).containsExactly("reserved" to CURRENT_FRAMES)
        drained.assertPcm(mapOf(0L to 3_000, CURRENT_FRAMES to 12_000), total = CURRENT_FRAMES + SUCCESSOR_FRAMES)
    }

    private fun snapshot(): FfiCoreSnapshot = requireNotNull(core.snapshots.value)

    private fun snapshotWithin(millis: Long, predicate: (FfiCoreSnapshot) -> Boolean): FfiCoreSnapshot? {
        val deadline = System.nanoTime() + millis * 1_000_000
        while (System.nanoTime() < deadline) {
            core.snapshots.value?.takeIf(predicate)?.let { return it }
            Thread.sleep(5)
        }
        return null
    }

    private fun awaitSnapshot(what: String, predicate: (FfiCoreSnapshot) -> Boolean): FfiCoreSnapshot =
        snapshotWithin(5_000, predicate) ?: throw AssertionError("timed out waiting for $what")

    private fun wavFile(name: String, frames: Long, sample: Int): SelectedAudioFile {
        val file = File(folder, "$name.wav").apply { writeBytes(wavPcm16(frames.toInt(), sample.toShort())) }
        return SelectedAudioFile(Uri.fromFile(file), "$name.wav", "audio/wav", file.length())
    }

    /** A host session playing `current`, with [upcoming] queued, every song staged. */
    private suspend fun startPlaying(upcoming: List<Pair<String, Int>>) {
        // Staged, identified and queued exactly as MainViewModel does.
        val current = wavFile("current", CURRENT_FRAMES, 3_000)
        staging.stage(current)
        val entries = upcoming.map { (name, sample) ->
            val selected = wavFile(name, SUCCESSOR_FRAMES, sample)
            staging.stage(selected)
            FfiQueueEntry(name, selected.toFfiAudioSource())
        }
        core.configureAndCreate(
            HostFormState(sessionName = "Authoritative edit", selectedAudio = current)
                .toFfiHostDraft(TuningSettings()),
            entries,
        )
        awaitSnapshot("the queue") { it.hostQueue.size == 1 + upcoming.size }
        awaitSnapshot("advertising to complete (errors: $errors)") {
            it.hostLifecycle == FfiHostLifecycle.READY || it.hostLifecycle == FfiHostLifecycle.WAITING_FOR_LISTENERS
        }
        // The production play path: each step waits for the actor to accept it.
        core.transitionPlaybackState(FfiPlaybackState.BUFFERING)
        core.transitionPlaybackState(FfiPlaybackState.PLAYING)
        core.playbackPositionAdvanced(STREAM_ID, 0L)
    }

    /** Opens the stream the way the host does; it reads the first packet. */
    private suspend fun prepare(): com.ekkus.silentdisco.core.rust.HostPlaylistStreamController {
        val sources = snapshot().hostQueue.map { entry ->
            com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource(
                entry.entryId,
                entry.source.displayName,
                requireNotNull(staging.stagedFile(entry.source.sourceId)).absolutePath,
            )
        }
        val prepared = playback.prepareStream(SessionId("authoritative-session"), StreamId(STREAM_ID)) {
            core.openHostPlaylist("authoritative-session", STREAM_ID, 5_000uL, sources)
        }
        assertThat(snapshot().hostQueueNextBoundary).isNull()
        assertThat(prepared.firstPacket.firstSampleIndex).isEqualTo(0L)
        return prepared.streamer
    }

    private class Drained(
        val values: List<Pair<Long, Int>>,
        val samples: Long,
        val boundaries: List<Pair<String, Long>>,
    ) {
        /** Each packet's first sample equals the value of the latest segment at or before it. */
        fun assertPcm(segments: Map<Long, Int>, total: Long) {
            assertThat(samples).isEqualTo(total)
            values.forEach { (at, value) ->
                val expected = segments.filterKeys { it <= at }.maxBy { it.key }.value
                assertThat(value).isEqualTo(expected)
            }
        }
    }

    /** Reads to End, checking one contiguous sample timeline. The prepared first packet counts. */
    private fun drain(
        stream: com.ekkus.silentdisco.core.rust.HostPlaylistStreamController,
        alreadyRead: List<FfiPlaylistFrame> = emptyList(),
    ): Drained {
        val values = mutableListOf<Pair<Long, Int>>()
        val boundaries = mutableListOf<Pair<String, Long>>()
        var samples = SAMPLES_PER_PACKET // the first packet prepareStream read
        values += 0L to 3_000
        fun take(frame: FfiPlaylistFrame): Boolean = when (frame) {
            FfiPlaylistFrame.End -> true
            is FfiPlaylistFrame.TrackBoundary -> {
                assertThat(frame.firstSampleIndex.toLong()).isEqualTo(samples)
                boundaries += frame.entryId to frame.firstSampleIndex.toLong()
                false
            }
            is FfiPlaylistFrame.Audio -> {
                assertThat(frame.firstSampleIndex.toLong()).isEqualTo(samples)
                val sample = (frame.payload[0].toInt() and 0xff) or (frame.payload[1].toInt() shl 8)
                values += samples to sample
                samples += frame.samplesPerPacket.toLong()
                false
            }
        }
        alreadyRead.forEach { take(it) }
        val deadline = System.nanoTime() + 10_000_000_000L
        while (System.nanoTime() < deadline) {
            val frame = stream.nextFrame(500uL) ?: continue
            if (take(frame)) return Drained(values, samples, boundaries)
        }
        throw AssertionError("the producer never ended")
    }

    private fun wavPcm16(frames: Int, sample: Short): ByteArray {
        val dataBytes = frames * 4
        val output = ByteArray(44 + dataBytes)
        fun ascii(offset: Int, value: String) = value.encodeToByteArray().copyInto(output, offset)
        fun u16(offset: Int, value: Int) {
            output[offset] = value.toByte()
            output[offset + 1] = (value shr 8).toByte()
        }
        fun u32(offset: Int, value: Int) = repeat(4) { output[offset + it] = (value shr (it * 8)).toByte() }
        ascii(0, "RIFF")
        u32(4, 36 + dataBytes)
        ascii(8, "WAVEfmt ")
        u32(16, 16)
        u16(20, 1)
        u16(22, 2)
        u32(24, 48_000)
        u32(28, 192_000)
        u16(32, 4)
        u16(34, 16)
        ascii(36, "data")
        u32(40, dataBytes)
        var offset = 44
        repeat(frames * 2) {
            output[offset++] = sample.toByte()
            output[offset++] = (sample.toInt() shr 8).toByte()
        }
        return output
    }

    private companion object {
        const val STREAM_ID = "authoritative-stream"
        const val CURRENT_FRAMES = 9_600L
        const val SUCCESSOR_FRAMES = 480L
        const val SAMPLES_PER_PACKET = 240L
    }
}
