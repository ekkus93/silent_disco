package com.ekkus.silentdisco.core.audio

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class CallbackPcmWavWriterTest {
    private fun temporaryFile(): File {
        val directory = Files.createTempDirectory("callback-pcm-test").toFile()
        directory.deleteOnExit()
        return File(directory, "callback.wav").also { it.deleteOnExit() }
    }

    @Test
    fun callbackPcmIsWrittenAsLittleEndianStereoWavWithExactLength() {
        val file = temporaryFile()
        val writer = CallbackPcmWavWriter(file, sampleRate = 48_000)
        writer.append(listOf(0f, 1f, -1f, 0.5f))
        assertEquals(2L, writer.finish())
        assertEquals(2L, writer.finish())
        val bytes = file.readBytes()
        assertArrayEquals("RIFF".toByteArray(), bytes.copyOfRange(0, 4))
        assertArrayEquals("WAVE".toByteArray(), bytes.copyOfRange(8, 12))
        assertEquals(52, bytes.size)
        assertArrayEquals(
            byteArrayOf(0, 0, -1, 127, 0, -128, 0, 64),
            bytes.copyOfRange(44, 52),
        )
    }

    @Test
    fun partialFramesAndExcessiveCaptureFailExplicitly() {
        val file = temporaryFile()
        val writer = CallbackPcmWavWriter(file, maxFrames = 1)
        assertThrows(IllegalArgumentException::class.java) { writer.append(listOf(1f)) }
        writer.append(listOf(0.1f, 0.2f))
        assertThrows(IllegalStateException::class.java) { writer.append(listOf(0.3f, 0.4f)) }
        writer.finish()
        assertThrows(IllegalStateException::class.java) { writer.append(listOf(0.5f, 0.6f)) }
    }
}
