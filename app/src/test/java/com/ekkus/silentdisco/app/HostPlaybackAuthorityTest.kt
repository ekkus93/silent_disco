package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

class HostPlaybackAuthorityTest {
    @Test
    fun acceptedTransitionRunsSideEffectAfterAuthorityConfirmation() = runTest {
        val events = mutableListOf<String>()

        val result = runAfterAuthoritativeHostPlaybackTransition(
            target = FfiPlaybackState.PAUSED,
            transition = { state -> events += "transition:$state" },
            afterAccepted = {
                events += "side-effect"
                "complete"
            },
        )

        assertThat(result).isEqualTo("complete")
        assertThat(events).containsExactly(
            "transition:PAUSED",
            "side-effect",
        ).inOrder()
    }

    @Test
    fun rejectedTransitionPreventsPlatformSideEffects() = runTest {
        var sideEffectRan = false

        val result = runCatching {
            runAfterAuthoritativeHostPlaybackTransition(
                target = FfiPlaybackState.STOPPED,
                transition = { throw IllegalStateException("transition rejected") },
                afterAccepted = {
                    sideEffectRan = true
                },
            )
        }

        assertThat(result.exceptionOrNull()).isInstanceOf(IllegalStateException::class.java)
        assertThat(sideEffectRan).isFalse()
    }
}
