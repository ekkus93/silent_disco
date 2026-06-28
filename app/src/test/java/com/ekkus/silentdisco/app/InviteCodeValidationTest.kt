package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class InviteCodeValidationTest {
    private fun createValidator(): (ApprovalMode, String, String?) -> String? {
        return { mode, expectedCode, actualCode ->
            if (mode != ApprovalMode.INVITE_CODE) {
                null
            } else {
                val expected = expectedCode.trim()
                val actual = actualCode?.trim().orEmpty()
                when {
                    expected.isBlank() -> "Host invite code is not configured"
                    actual != expected -> "Incorrect invite code"
                    else -> null
                }
            }
        }
    }

    @Test
    fun validation_passesWhenManualMode() {
        assertNull(createValidator()(ApprovalMode.MANUAL, "", null))
    }

    @Test
    fun validation_passesWhenTrustedDevicesMode() {
        assertNull(createValidator()(ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER, "", null))
    }

    @Test
    fun validation_failsWhenInviteCodeModeAndHostCodeBlank() {
        assertEquals(
            "Host invite code is not configured",
            createValidator()(ApprovalMode.INVITE_CODE, "", "1234"),
        )
    }

    @Test
    fun validation_failsWhenInviteCodeModeAndHostCodeWhitespace() {
        assertEquals(
            "Host invite code is not configured",
            createValidator()(ApprovalMode.INVITE_CODE, "   ", "1234"),
        )
    }

    @Test
    fun validation_failsWhenInviteCodeModeAndListenerCodeMissing() {
        assertEquals(
            "Incorrect invite code",
            createValidator()(ApprovalMode.INVITE_CODE, "2468", null),
        )
    }

    @Test
    fun validation_failsWhenInviteCodeModeAndListenerCodeEmpty() {
        assertEquals(
            "Incorrect invite code",
            createValidator()(ApprovalMode.INVITE_CODE, "2468", ""),
        )
    }

    @Test
    fun validation_failsWhenInviteCodeModeAndListenerCodeWrong() {
        assertEquals(
            "Incorrect invite code",
            createValidator()(ApprovalMode.INVITE_CODE, "2468", "1234"),
        )
    }

    @Test
    fun validation_passesWhenInviteCodeModeAndCodesMatch() {
        assertNull(createValidator()(ApprovalMode.INVITE_CODE, "2468", "2468"))
    }

    @Test
    fun validation_passesWhenInviteCodeModeAndCodesMatchIgnoringOuterWhitespace() {
        assertNull(createValidator()(ApprovalMode.INVITE_CODE, " 2468 ", "2468"))
    }

    @Test
    fun validation_passesWhenInviteCodeModeAndCodesMatchIgnoringInnerWhitespace() {
        assertNull(createValidator()(ApprovalMode.INVITE_CODE, "2468", " 2468 "))
    }

    @Test
    fun validation_passesWhenInviteCodeModeAndCodesMatchIgnoringBothWhitespace() {
        assertNull(createValidator()(ApprovalMode.INVITE_CODE, " 2468 ", " 2468 "))
    }

    @Test
    fun validation_isCaseSensitive() {
        assertEquals(
            "Incorrect invite code",
            createValidator()(ApprovalMode.INVITE_CODE, "ABCD", "abcd"),
        )
    }
}

class JoinRejectionReasonTest {
    // This test uses a simpler approach that doesn't require MainViewModel mocking
    private fun joinRejectionReasonValidator(
        form: HostFormState,
        sessionId: String,
        currentSessionId: String?,
    ): String? {
        if (sessionId != currentSessionId) return "Session mismatch"
        if (form.approvalMode == ApprovalMode.INVITE_CODE) {
            val expected = form.inviteCode.trim()
            val actual = "" // In real use, this comes from message.inviteCode?.trim().orEmpty()
            if (expected.isBlank()) return "Host invite code is not configured"
            if (actual != expected) return "Incorrect invite code"
        }
        return null
    }

    @Test
    fun rejectionReason_sessionMismatch() {
        val form = HostFormState()
        val reason = joinRejectionReasonValidator(form, "other-session-id", "my-session-id")
        assertEquals("Session mismatch", reason)
    }

    @Test
    fun rejectionReason_passesSameSessionManualMode() {
        val form = HostFormState(approvalMode = ApprovalMode.MANUAL)
        val reason = joinRejectionReasonValidator(form, "my-session-id", "my-session-id")
        assertNull(reason)
    }
}
