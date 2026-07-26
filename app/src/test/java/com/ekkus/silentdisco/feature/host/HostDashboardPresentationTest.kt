package com.ekkus.silentdisco.feature.host

import com.ekkus.silentdisco.app.JoinApprovalAction
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class HostDashboardPresentationTest {
    @Test
    fun approvalProgressExplainsPersistenceOrdering() {
        assertThat(approvalProgressLabel(JoinApprovalAction.APPROVE_ONCE))
            .isEqualTo("Sending session approval…")
        assertThat(approvalProgressLabel(JoinApprovalAction.ALWAYS_ALLOW))
            .contains("Remembering this phone")
        assertThat(approvalProgressLabel(JoinApprovalAction.REJECT))
            .isEqualTo("Sending rejection…")
    }

    @Test
    fun waitingDurationUsesPlainLanguage() {
        assertThat(waitingDurationLabel(0L)).isEqualTo("Just requested")
        assertThat(waitingDurationLabel(12L)).isEqualTo("Waiting 12s")
        assertThat(waitingDurationLabel(125L)).isEqualTo("Waiting 2m")
    }
}
