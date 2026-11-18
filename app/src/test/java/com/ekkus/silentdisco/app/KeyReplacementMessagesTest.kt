package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.identity.KeyContinuityMonitor
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class KeyReplacementMessagesTest {
    @Test
    fun noReplacedKeyGivesNoNotice() {
        assertThat(keyReplacementMessages(hostIdentityReplaced = false, otherKeys = emptySet())).isEmpty()
    }

    @Test
    fun eachReplacedKeyGetsItsOwnNoticeInAFixedOrder() {
        assertThat(
            keyReplacementMessages(
                hostIdentityReplaced = true,
                otherKeys = setOf(KeyContinuityMonitor.Kind.LISTENER_PROOF, KeyContinuityMonitor.Kind.HOST_TLS),
            ),
        ).containsExactly(
            HOST_KEY_REPLACED_MESSAGE,
            HOST_TLS_KEY_REPLACED_MESSAGE,
            LISTENER_KEY_REPLACED_MESSAGE,
        ).inOrder()
    }

    @Test
    fun aReplacedHostTlsKeyTellsTheHostItsRememberedListenersAreGone() {
        val notice = keyReplacementMessages(false, setOf(KeyContinuityMonitor.Kind.HOST_TLS)).single()
        assertThat(notice).contains("Always allow")
        assertThat(notice).contains("approved again")
    }
}
