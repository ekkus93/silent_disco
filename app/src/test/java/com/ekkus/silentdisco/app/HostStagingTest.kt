package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.audio.AudioFileAccessException
import java.io.ByteArrayInputStream
import java.nio.file.Files
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class HostStagingTest {
    @Test
    fun insufficientKnownCapacityFailsBeforeWritingAnySourceBytes() = runTest {
        val target = Files.createTempFile("silent-disco-stage", ".wav").toFile()
        try {
            val failure = runCatching {
                copyAudioStreamToBoundedFile(
                    input = ByteArrayInputStream(byteArrayOf(1, 2, 3, 4)),
                    destination = target,
                    displayName = "known-size.wav",
                    expectedSize = 4,
                    availableBytes = { 3 },
                )
            }.exceptionOrNull()

            assertTrue(failure is AudioFileAccessException)
            assertTrue(failure?.message.orEmpty().contains("not enough device storage"))
            assertEquals(0, target.length())
        } finally {
            target.delete()
        }
    }

    @Test
    fun storageExhaustionDuringCopyFailsAndDoesNotReportSuccess() = runTest {
        val target = Files.createTempFile("silent-disco-stage", ".wav").toFile()
        var capacityChecks = 0
        try {
            val failure = runCatching {
                copyAudioStreamToBoundedFile(
                    input = ByteArrayInputStream(ByteArray(8) { it.toByte() }),
                    destination = target,
                    displayName = "fills-storage.wav",
                    expectedSize = null,
                    availableBytes = {
                        capacityChecks += 1
                        if (capacityChecks == 1) 8 else 4
                    },
                )
            }.exceptionOrNull()

            assertTrue(failure is AudioFileAccessException)
            assertTrue(failure?.message.orEmpty().contains("storage filled while staging"))
            assertEquals(0, target.length())
        } finally {
            target.delete()
        }
    }

    @Test
    fun revokedProviderAccessIsReportedWithTheSongName() = runTest {
        val target = Files.createTempFile("silent-disco-stage", ".wav").toFile()
        try {
            val failure = runCatching {
                copyProviderAudioToBoundedFile(
                    openInputStream = { throw SecurityException("provider grant revoked") },
                    destination = target,
                    displayName = "revoked-song.wav",
                    expectedSize = null,
                    availableBytes = { 1_024 },
                )
            }.exceptionOrNull()

            assertTrue(failure is AudioFileAccessException)
            assertTrue(failure?.message.orEmpty().contains("revoked-song.wav"))
            assertTrue(failure?.message.orEmpty().contains("access to this song was revoked"))
            assertEquals(0, target.length())
        } finally {
            target.delete()
        }
    }

    @Test
    fun providerRevocationDuringReadIsReportedInsteadOfAcceptedAsCleanEof() = runTest {
        val target = Files.createTempFile("silent-disco-stage", ".wav").toFile()
        try {
            val failure = runCatching {
                copyProviderAudioToBoundedFile(
                    openInputStream = {
                        object : ByteArrayInputStream(byteArrayOf(1, 2, 3)) {
                            override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                                throw SecurityException("provider grant revoked while reading")
                            }
                        }
                    },
                    destination = target,
                    displayName = "revoked-during-copy.wav",
                    expectedSize = null,
                    availableBytes = { 1_024 },
                )
            }.exceptionOrNull()

            assertTrue(failure is AudioFileAccessException)
            assertTrue(failure?.message.orEmpty().contains("revoked-during-copy.wav"))
            assertTrue(failure?.message.orEmpty().contains("access to this song was revoked"))
        } finally {
            target.delete()
        }
    }
}
