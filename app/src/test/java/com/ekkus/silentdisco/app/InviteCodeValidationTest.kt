package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.mockito.kotlin.mock
import org.mockito.kotlin.whenever

class InviteCodeValidationTest {
    @Test
    fun inviteCodeAndOpaqueAudioIdentityArePassedToRustWithoutKotlinValidation() {
        val uri: Uri = mock()
        whenever(uri.toString()).thenReturn("content://private/audio/42")
        val draft = HostFormState(
            sessionName = "Invite session",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = " 2468 ",
            selectedAudio = SelectedAudioFile(
                uri = uri,
                displayName = "set.wav",
                mimeType = "audio/wav",
                sizeBytes = 512,
            ),
        ).toFfiHostDraft(TuningSettings())

        assertEquals(FfiApprovalMode.INVITE_CODE, draft.approvalMode)
        assertEquals(" 2468 ", draft.inviteCode)
        assertFalse(draft.audioSource!!.sourceId.contains("content://"))
    }
}
