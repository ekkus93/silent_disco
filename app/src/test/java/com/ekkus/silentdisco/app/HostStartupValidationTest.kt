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

    private fun createValidator(): (AppUiState) -> String? {
        return { state ->
            val form = state.hostForm
            when {
                form.sessionName.isBlank() -> "Enter a session name before hosting."
                form.selectedAudio == null -> "Choose an audio file before hosting."
                form.approvalMode == ApprovalMode.INVITE_CODE && form.inviteCode.isBlank() ->
                    "Enter an invite code or choose a different approval mode."
                else -> null
            }
        }
    }

    @Test
    fun validation_passeWithValidForm() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.MANUAL,
        )
        val state = AppUiState(hostForm = form)
        assertNull(createValidator()(state))
    }

    @Test
    fun validation_failsWhenSessionNameBlank() {
        val form = HostFormState(
            sessionName = "",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.MANUAL,
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter a session name before hosting.", createValidator()(state))
    }

    @Test
    fun validation_failsWhenSessionNameWhitespace() {
        val form = HostFormState(
            sessionName = "   ",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.MANUAL,
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter a session name before hosting.", createValidator()(state))
    }

    @Test
    fun validation_failsWhenAudioNotSelected() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = null,
            approvalMode = ApprovalMode.MANUAL,
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Choose an audio file before hosting.", createValidator()(state))
    }

    @Test
    fun validation_failsWhenInviteCodeModeButCodeBlank() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "",
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter an invite code or choose a different approval mode.", createValidator()(state))
    }

    @Test
    fun validation_failsWhenInviteCodeModeButCodeWhitespace() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "   ",
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter an invite code or choose a different approval mode.", createValidator()(state))
    }

    @Test
    fun validation_passesWhenInviteCodeModeWithValidCode() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "1234",
        )
        val state = AppUiState(hostForm = form)
        assertNull(createValidator()(state))
    }

    @Test
    fun validation_passesWhenManualModeWithoutCode() {
        val form = HostFormState(
            sessionName = "Test Session",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.MANUAL,
            inviteCode = "",
        )
        val state = AppUiState(hostForm = form)
        assertNull(createValidator()(state))
    }

    @Test
    fun validation_checksNameFirst() {
        val form = HostFormState(
            sessionName = "",
            selectedAudio = null,
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "",
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter a session name before hosting.", createValidator()(state))
    }

    @Test
    fun validation_checksAudioSecond() {
        val form = HostFormState(
            sessionName = "Test",
            selectedAudio = null,
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "",
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Choose an audio file before hosting.", createValidator()(state))
    }

    @Test
    fun validation_checksInviteCodeThird() {
        val form = HostFormState(
            sessionName = "Test",
            selectedAudio = SelectedAudioFile(uri = mockUri, displayName = "test.mp3", mimeType = "audio/mpeg", sizeBytes = 1000L),
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = "",
        )
        val state = AppUiState(hostForm = form)
        assertEquals("Enter an invite code or choose a different approval mode.", createValidator()(state))
    }
}
