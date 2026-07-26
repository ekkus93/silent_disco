package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class JoinApprovalCommandTest {
    @Test
    fun requestedLifetimeIsAppliedOnlyForTheApprovalDispatch() {
        var lifetime = false
        val events = mutableListOf<String>()

        withScopedApprovalLifetime(
            requestedLifetime = true,
            currentLifetime = { lifetime },
            updateLifetime = {
                lifetime = it
                events += "lifetime=$it"
            },
            dispatch = {
                events += "dispatch=$lifetime"
            },
        )

        assertThat(events).containsExactly(
            "lifetime=true",
            "dispatch=true",
            "lifetime=false",
        ).inOrder()
        assertThat(lifetime).isFalse()
    }

    @Test
    fun previousLifetimeIsRestoredWhenDispatchThrows() {
        var lifetime = true

        val failure = runCatching {
            withScopedApprovalLifetime(
                requestedLifetime = false,
                currentLifetime = { lifetime },
                updateLifetime = { lifetime = it },
                dispatch = { error("approval dispatch failed") },
            )
        }.exceptionOrNull()

        assertThat(failure).isInstanceOf(IllegalStateException::class.java)
        assertThat(lifetime).isTrue()
    }
}
