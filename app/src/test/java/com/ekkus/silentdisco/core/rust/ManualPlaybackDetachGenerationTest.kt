package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.mockito.Mockito.mock
import org.mockito.Mockito.never
import org.mockito.Mockito.times
import org.mockito.Mockito.verify

/**
 * Ending a manual stream detaches playback from the connection that owns the
 * stream, never from whichever connection is current (remediation §9).
 */
class ManualPlaybackDetachGenerationTest {
    private val controller = ManualListenerTransportController(closeNativeOutput = {})

    private fun bind(handle: FfiListenerTransportHandle) =
        controller.handleGate.replace(retire = {}, open = { handle }, start = {})

    @Test
    fun endingAnOldSessionsStreamNeverDetachesTheNewSessionsPlayback() {
        val old = mock(FfiListenerTransportHandle::class.java)
        val replacement = mock(FfiListenerTransportHandle::class.java)
        bind(old)
        controller.playbackRuntime = mock(FfiListenerPlaybackHandle::class.java)

        // A rebind replaces the connection before the old stream's teardown runs.
        bind(replacement)
        controller.stopPlayback()

        verify(replacement, never()).detachPlayback()
    }

    @Test
    fun anOwnerRetiredByARebindIsNotDetachedAndTheReplacementIsUntouched() {
        val old = mock(FfiListenerTransportHandle::class.java)
        val replacement = mock(FfiListenerTransportHandle::class.java)
        bind(old)
        controller.playbackRuntime = mock(FfiListenerPlaybackHandle::class.java)
        controller.playbackOwnerRef.set(old)

        bind(replacement)
        controller.stopPlayback()

        verify(old, never()).detachPlayback()
        verify(replacement, never()).detachPlayback()
        assertThat(controller.playbackOwnerRef.get()).isNull()
    }

    @Test
    fun endingTheCurrentSessionsStreamDetachesItsOwnConnectionOnce() {
        val current = mock(FfiListenerTransportHandle::class.java)
        bind(current)
        controller.playbackRuntime = mock(FfiListenerPlaybackHandle::class.java)
        controller.playbackOwnerRef.set(current)

        controller.stopPlayback()
        controller.stopPlayback()

        verify(current, times(1)).detachPlayback()
        assertThat(controller.playbackOwnerRef.get()).isNull()
    }
}
