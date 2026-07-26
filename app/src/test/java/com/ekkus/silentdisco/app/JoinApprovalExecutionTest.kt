package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.TrustState
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

class JoinApprovalExecutionTest {
    @Test
    fun durableTrustCommitsBeforeApprovalAdvertisesIt() = runTest {
        val calls = mutableListOf<String>()

        val result = executeJoinApproval(
            rememberForFuture = true,
            persistTrust = {
                calls += "persist"
                Result.success(Unit)
            },
            sendApproval = { trustedForFuture ->
                calls += "send:$trustedForFuture"
                true
            },
        )

        assertThat(calls).containsExactly("persist", "send:true").inOrder()
        assertThat(result.delivered).isTrue()
        assertThat(result.trustedForFuture).isTrue()
        assertThat(result.trustState).isEqualTo(TrustState.TRUSTED_PLACEHOLDER)
        assertThat(result.persistenceError).isNull()
    }

    @Test
    fun failedTrustWriteFallsBackVisiblyToSessionOnlyApproval() = runTest {
        val calls = mutableListOf<String>()
        val failure = IllegalStateException("database busy")

        val result = executeJoinApproval(
            rememberForFuture = true,
            persistTrust = {
                calls += "persist"
                Result.failure(failure)
            },
            sendApproval = { trustedForFuture ->
                calls += "send:$trustedForFuture"
                true
            },
        )

        assertThat(calls).containsExactly("persist", "send:false").inOrder()
        assertThat(result.delivered).isTrue()
        assertThat(result.trustedForFuture).isFalse()
        assertThat(result.trustState).isEqualTo(TrustState.SESSION_ONLY)
        assertThat(result.persistenceError).isSameInstanceAs(failure)
    }

    @Test
    fun sessionOnlyApprovalDoesNotAttemptTrustPersistence() = runTest {
        val calls = mutableListOf<String>()

        val result = executeJoinApproval(
            rememberForFuture = false,
            persistTrust = {
                throw AssertionError("session-only approval must not write trust")
            },
            sendApproval = { trustedForFuture ->
                calls += "send:$trustedForFuture"
                true
            },
        )

        assertThat(calls).containsExactly("send:false")
        assertThat(result.trustState).isEqualTo(TrustState.SESSION_ONLY)
    }

    @Test
    fun approvedListenerModelUsesCommittedTrustState() {
        val request = JoinRequest(
            requestId = "request-1",
            sessionId = "session-1",
            listenerId = "listener-1",
            listenerName = "Listener One",
            inviteCode = null,
            requestedAtMs = 10,
        )

        assertThat(request.toListenerInfo(TrustState.TRUSTED_PLACEHOLDER).trustState)
            .isEqualTo(TrustState.TRUSTED_PLACEHOLDER)
    }
}
