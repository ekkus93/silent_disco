package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mockito.kotlin.mock

/**
 * Covers [canStartOrPauseHostPlayback] and [canStopHostPlayback], the
 * canonical presentation-layer legality helpers the host dashboard renders
 * (Block 21.2) instead of reconstructing the same conditions inline.
 */
class HostPlaybackControlStateTest {

    private val audio = SelectedAudioFile(
        uri = mock<Uri>(),
        displayName = "track.mp3",
        mimeType = "audio/mpeg",
        sizeBytes = 1_000_000L,
    )

    @Test
    fun canStartOrPauseHostPlayback_requiresSelectedAudio() {
        val withoutAudio = AppUiState(
            hostForm = HostFormState(selectedAudio = null),
            hostPlaybackState = PlaybackState.STOPPED,
        )
        assertFalse(withoutAudio.canStartOrPauseHostPlayback())

        val withAudio = withoutAudio.copy(hostForm = HostFormState(selectedAudio = audio))
        assertTrue(withAudio.canStartOrPauseHostPlayback())
    }

    @Test
    fun canStartOrPauseHostPlayback_disabledWhileBuffering() {
        val buffering = AppUiState(
            hostForm = HostFormState(selectedAudio = audio),
            hostPlaybackState = PlaybackState.BUFFERING,
        )
        assertFalse(buffering.canStartOrPauseHostPlayback())
    }

    @Test
    fun canStartOrPauseHostPlayback_enabledForPlayingAndPaused() {
        listOf(PlaybackState.PLAYING, PlaybackState.PAUSED, PlaybackState.STOPPED, PlaybackState.READY)
            .forEach { state ->
                val uiState = AppUiState(
                    hostForm = HostFormState(selectedAudio = audio),
                    hostPlaybackState = state,
                )
                assertTrue("state=$state should allow play/pause", uiState.canStartOrPauseHostPlayback())
            }
    }

    @Test
    fun canStopHostPlayback_rejectsStoppedAndError() {
        listOf(PlaybackState.STOPPED, PlaybackState.ERROR).forEach { state ->
            val uiState = AppUiState(hostPlaybackState = state)
            assertFalse("state=$state should reject stop", uiState.canStopHostPlayback())
        }
    }

    @Test
    fun canStopHostPlayback_allowsActiveStates() {
        listOf(
            PlaybackState.BUFFERING,
            PlaybackState.READY,
            PlaybackState.PLAYING,
            PlaybackState.PAUSED,
            PlaybackState.UNDERRUN,
        ).forEach { state ->
            val uiState = AppUiState(hostPlaybackState = state)
            assertTrue("state=$state should allow stop", uiState.canStopHostPlayback())
        }
    }
}
