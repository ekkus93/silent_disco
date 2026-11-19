package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.mockito.Mockito.mock

/**
 * The manual listener opens its output when approved, on a silent standby
 * ring, so the first stream rebinds an open output instead of spending
 * ~350 ms opening one (startup-counter investigation, 2026-11-19). The
 * native open and rebind are Oboe's and run on the device.
 */
class ManualStandbyOutputTest {
    private var opened = 0
    private var released = 0
    private val controller = ManualListenerTransportController(
        closeNativeOutput = {},
        openStandbyOutput = {
            opened += 1
            AutoCloseable { released += 1 }
        },
    )

    @Test
    fun approvalOpensTheStandbyOnceAndNotWhenAStreamIsRunning() {
        controller.openStandbyOutputIfClosed()
        controller.openStandbyOutputIfClosed()
        assertThat(opened).isEqualTo(1)

        val running = ManualListenerTransportController(
            closeNativeOutput = {},
            openStandbyOutput = { error("must not open with a stream running") },
        )
        running.playbackRuntime = mock(FfiListenerPlaybackHandle::class.java)
        running.openStandbyOutputIfClosed()
        assertThat(running.standbyOutput).isNull()
    }

    @Test
    fun aStreamChangeKeepsTheStandbyAndATeardownReleasesIt() {
        controller.openStandbyOutputIfClosed()
        controller.endStream(keepOutputOpen = true)
        assertThat(released).isEqualTo(0)

        controller.stopPlayback()
        assertThat(released).isEqualTo(1)
        assertThat(controller.standbyOutput).isNull()
    }

    @Test
    fun releasingAfterTheFirstStreamRebindsHappensOnce() {
        controller.openStandbyOutputIfClosed()
        controller.releaseStandbyOutput()
        controller.releaseStandbyOutput()
        assertThat(released).isEqualTo(1)
    }

    @Test
    fun anOutputThatCannotOpenIsNotFatal() {
        val failing = ManualListenerTransportController(
            closeNativeOutput = {},
            openStandbyOutput = { throw IllegalStateException("no audio device") },
        )
        failing.openStandbyOutputIfClosed()
        assertThat(failing.standbyOutput).isNull()
        val refused = ManualListenerTransportController(closeNativeOutput = {}, openStandbyOutput = { null })
        refused.openStandbyOutputIfClosed()
        assertThat(refused.standbyOutput).isNull()
    }
}
