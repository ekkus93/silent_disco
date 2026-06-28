package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.mockito.kotlin.mock

class HostStartupValidationTest {
    private val mockUri: Uri = mock()
    private val audio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L)

    @Test
    fun validate_passesWithValidManualForm() {
        val form = HostFormState(sessionName = "Test Session", selectedAudio = audio, approvalMode = ApprovalMode.MANUAL)
        assertNull(HostSessionValidator.validate(form))
    }

    @Test
    fun validate_failsWhenSessionNameBlank() {
        val form = HostFormState(sessionName = "", selectedAudio = audio, approvalMode = ApprovalMode.MANUAL)
        assertEquals("Enter a session name before hosting.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_failsWhenSessionNameWhitespace() {
        val form = HostFormState(sessionName = "   ", selectedAudio = audio, approvalMode = ApprovalMode.MANUAL)
        assertEquals("Enter a session name before hosting.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_failsWhenAudioNotSelected() {
        val form = HostFormState(sessionName = "Test Session", selectedAudio = null, approvalMode = ApprovalMode.MANUAL)
        assertEquals("Choose an audio file before hosting.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_failsWhenInviteCodeModeButCodeBlank() {
        val form = HostFormState(sessionName = "Test", selectedAudio = audio, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "")
        assertEquals("Enter an invite code or choose a different approval mode.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_failsWhenInviteCodeModeButCodeWhitespace() {
        val form = HostFormState(sessionName = "Test", selectedAudio = audio, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "   ")
        assertEquals("Enter an invite code or choose a different approval mode.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_passesWhenInviteCodeModeWithValidCode() {
        val form = HostFormState(sessionName = "Test", selectedAudio = audio, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "1234")
        assertNull(HostSessionValidator.validate(form))
    }

    @Test
    fun validate_passesWhenManualModeWithoutCode() {
        val form = HostFormState(sessionName = "Test", selectedAudio = audio, approvalMode = ApprovalMode.MANUAL, inviteCode = "")
        assertNull(HostSessionValidator.validate(form))
    }

    @Test
    fun validate_checksNameFirst() {
        val form = HostFormState(sessionName = "", selectedAudio = null, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "")
        assertEquals("Enter a session name before hosting.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_checksAudioSecond() {
        val form = HostFormState(sessionName = "Test", selectedAudio = null, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "")
        assertEquals("Choose an audio file before hosting.", HostSessionValidator.validate(form))
    }

    @Test
    fun validate_checksInviteCodeThird() {
        val form = HostFormState(sessionName = "Test", selectedAudio = audio, approvalMode = ApprovalMode.INVITE_CODE, inviteCode = "")
        assertEquals("Enter an invite code or choose a different approval mode.", HostSessionValidator.validate(form))
    }
}
