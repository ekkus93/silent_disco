package com.ekkus.silentdisco.feature.listener

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class NearbySessionPresentationTest {
    @Test
    fun everyApprovalModeHasPlainLanguageLabel() {
        val labels = ApprovalMode.entries.associateWith { mode ->
            sessionAccessLabel(
                SessionInfo(
                    id = mode.name,
                    name = "Session",
                    hostDeviceName = "Host",
                    approvalMode = mode,
                    inviteCodeRequired = mode == ApprovalMode.INVITE_CODE,
                ),
            )
        }

        assertThat(labels[ApprovalMode.MANUAL]).isEqualTo("Approval required")
        assertThat(labels[ApprovalMode.INVITE_CODE]).isEqualTo("Invite code required")
        assertThat(labels[ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER]).isEqualTo("Approved devices only")
        labels.values.forEach { assertThat(it).doesNotContain("PLACEHOLDER") }
    }
}
