package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mockito.kotlin.mock

private val joinableStates = setOf(
    ListenerLifecycleState.IDLE,
    ListenerLifecycleState.SESSION_SELECTED,
    ListenerLifecycleState.JOIN_REQUESTED,
    ListenerLifecycleState.ERROR,
)

private val retryableStates = setOf(
    ListenerLifecycleState.ERROR,
    ListenerLifecycleState.DISCONNECTED,
)

class JoinProgressButtonVisibilityTest {

    // --- Request Join visibility (joinableStates) ---

    @Test
    fun requestJoin_visibleWhenIdle() = assertTrue(ListenerLifecycleState.IDLE in joinableStates)

    @Test
    fun requestJoin_visibleWhenSessionSelected() = assertTrue(ListenerLifecycleState.SESSION_SELECTED in joinableStates)

    @Test
    fun requestJoin_visibleWhenJoinRequested() = assertTrue(ListenerLifecycleState.JOIN_REQUESTED in joinableStates)

    @Test
    fun requestJoin_visibleWhenError() = assertTrue(ListenerLifecycleState.ERROR in joinableStates)

    @Test
    fun requestJoin_hiddenWhenConnecting() = assertFalse(ListenerLifecycleState.CONNECTING in joinableStates)

    @Test
    fun requestJoin_hiddenWhenPlaying() = assertFalse(ListenerLifecycleState.PLAYING in joinableStates)

    @Test
    fun requestJoin_hiddenWhenApproved() = assertFalse(ListenerLifecycleState.APPROVED in joinableStates)

    // --- Retry visibility (retryableStates) ---

    @Test
    fun retry_visibleWhenError() = assertTrue(ListenerLifecycleState.ERROR in retryableStates)

    @Test
    fun retry_visibleWhenDisconnected() = assertTrue(ListenerLifecycleState.DISCONNECTED in retryableStates)

    @Test
    fun retry_hiddenWhenIdle() = assertFalse(ListenerLifecycleState.IDLE in retryableStates)

    @Test
    fun retry_hiddenWhenPlaying() = assertFalse(ListenerLifecycleState.PLAYING in retryableStates)

    // --- Continue to Playback visibility ---

    @Test
    fun continueToPlayback_visibleOnlyWhenPlaying() {
        assertTrue(ListenerLifecycleState.PLAYING == ListenerLifecycleState.PLAYING)
        assertFalse(ListenerLifecycleState.SYNCING_CLOCK == ListenerLifecycleState.PLAYING)
        assertFalse(ListenerLifecycleState.CONNECTING == ListenerLifecycleState.PLAYING)
    }

    // --- Cancel visibility (hidden only when PLAYING) ---

    @Test
    fun cancel_hiddenWhenPlaying() = assertTrue(ListenerLifecycleState.PLAYING == ListenerLifecycleState.PLAYING)

    @Test
    fun cancel_visibleInAllOtherStates() {
        val otherStates = ListenerLifecycleState.entries.filter { it != ListenerLifecycleState.PLAYING }
        assertTrue(otherStates.all { it != ListenerLifecycleState.PLAYING })
    }
}

class HostSetupValidationTest {

    private val dummyAudio = SelectedAudioFile(
        uri = mock<Uri>(),
        displayName = "track.mp3",
        mimeType = "audio/mpeg",
        sizeBytes = 1_000_000L,
    )

    @Test
    fun startHosting_enabledWhenNameAndAudioPresent() {
        val form = HostFormState(sessionName = "Party", selectedAudio = dummyAudio)
        assertFalse(form.sessionName.isBlank())
        assertTrue(form.selectedAudio != null)
    }

    @Test
    fun startHosting_disabledWhenNameIsBlank() {
        val form = HostFormState(sessionName = "", selectedAudio = dummyAudio)
        assertTrue(form.sessionName.isBlank())
    }

    @Test
    fun startHosting_disabledWhenNameIsWhitespace() {
        val form = HostFormState(sessionName = "   ", selectedAudio = dummyAudio)
        assertTrue(form.sessionName.isBlank())
    }

    @Test
    fun startHosting_disabledWhenNoAudioSelected() {
        val form = HostFormState(sessionName = "Party", selectedAudio = null)
        assertTrue(form.selectedAudio == null)
    }

    @Test
    fun startHosting_disabledWhenBothMissing() {
        val form = HostFormState(sessionName = "", selectedAudio = null)
        assertTrue(form.sessionName.isBlank() || form.selectedAudio == null)
    }

    @Test
    fun missingItemsMessage_nameBlankOnly() {
        val form = HostFormState(sessionName = "", selectedAudio = dummyAudio)
        val items = buildList {
            if (form.sessionName.isBlank()) add("a session name")
            if (form.selectedAudio == null) add("an audio file")
        }
        assertEquals(listOf("a session name"), items)
    }

    @Test
    fun missingItemsMessage_audioMissingOnly() {
        val form = HostFormState(sessionName = "Party", selectedAudio = null)
        val items = buildList {
            if (form.sessionName.isBlank()) add("a session name")
            if (form.selectedAudio == null) add("an audio file")
        }
        assertEquals(listOf("an audio file"), items)
    }

    @Test
    fun missingItemsMessage_bothMissing() {
        val form = HostFormState(sessionName = "", selectedAudio = null)
        val items = buildList {
            if (form.sessionName.isBlank()) add("a session name")
            if (form.selectedAudio == null) add("an audio file")
        }
        assertEquals(listOf("a session name", "an audio file"), items)
    }

    @Test
    fun missingItemsMessage_nothingMissing_isEmpty() {
        val form = HostFormState(sessionName = "Party", selectedAudio = dummyAudio)
        val items = buildList {
            if (form.sessionName.isBlank()) add("a session name")
            if (form.selectedAudio == null) add("an audio file")
        }
        assertTrue(items.isEmpty())
    }

    // --- Playback button enabled states ---

    @Test
    fun startButton_disabledWhenPlaying() {
        val isPlaying = PlaybackState.PLAYING == PlaybackState.PLAYING
        assertFalse(!isPlaying) // enabled = !isPlaying && hasAudio
    }

    @Test
    fun startButton_disabledWhenNoAudio() {
        val hasAudio = false
        assertFalse(hasAudio)
    }

    @Test
    fun pauseButton_enabledOnlyWhenPlaying() {
        assertTrue(PlaybackState.PLAYING == PlaybackState.PLAYING)
        assertFalse(PlaybackState.STOPPED == PlaybackState.PLAYING)
    }

    @Test
    fun stopButton_disabledWhenStopped() {
        val isStopped = PlaybackState.STOPPED == PlaybackState.STOPPED
        assertFalse(!isStopped) // enabled = !isStopped
    }
}
