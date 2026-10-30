package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class HostRecoveryDispatchTest {
    @Test
    fun syncFloodHasFiniteInFlightJobsAndReleasesCancelledSlots() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val limiter = HostRecoveryDispatch(maxSyncInFlight = 4)
        repeat(4) {
            assertThat(limiter.launchSync(scope, dispatcher) { awaitCancellation() }).isTrue()
        }
        repeat(256) {
            assertThat(limiter.launchSync(scope, dispatcher) { error("unreachable overflow") }).isFalse()
        }
        assertThat(limiter.activeSyncRequests).isEqualTo(4)
        assertThat(limiter.rejectedSyncRequests).isEqualTo(256)
        scope.cancel()
        advanceUntilIdle()
        assertThat(limiter.activeSyncRequests).isEqualTo(0)

        val replacement = CoroutineScope(SupervisorJob() + dispatcher)
        var delivered = 0
        repeat(4) {
            assertThat(limiter.launchSync(replacement, dispatcher) { delivered++ }).isTrue()
        }
        advanceUntilIdle()
        assertThat(delivered).isEqualTo(4)
        assertThat(limiter.activeSyncRequests).isEqualTo(0)
        replacement.cancel()
    }

    @Test
    fun resyncCoalescesSamePeerGenerationButNeverOtherPeersOrRebinds() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val limiter = HostRecoveryDispatch()
        val old = Any()
        val replacement = Any()
        var delivered = 0
        assertThat(limiter.launchResync(scope, dispatcher, old, "peer-a") {
            delivered++
            awaitCancellation()
        }).isTrue()
        repeat(128) {
            assertThat(limiter.launchResync(scope, dispatcher, old, "peer-a") {
                error("duplicate replay")
            }).isFalse()
        }
        assertThat(limiter.launchResync(scope, dispatcher, old, "peer-b") {
            delivered++
            awaitCancellation()
        }).isTrue()
        assertThat(limiter.launchResync(scope, dispatcher, replacement, "peer-a") {
            delivered++
            awaitCancellation()
        }).isTrue()
        advanceUntilIdle()
        assertThat(delivered).isEqualTo(3)
        assertThat(limiter.activeResyncRequests).isEqualTo(3)

        scope.cancel()
        advanceUntilIdle()
        assertThat(limiter.activeResyncRequests).isEqualTo(0)
        val fresh = CoroutineScope(SupervisorJob() + dispatcher)
        assertThat(limiter.launchResync(fresh, dispatcher, replacement, "peer-a") { delivered++ }).isTrue()
        advanceUntilIdle()
        assertThat(delivered).isEqualTo(4)
        assertThat(limiter.activeResyncRequests).isEqualTo(0)
        fresh.cancel()
    }

    @Test
    fun invalidSyncLimitCannotEnableAnUnboundedQueue() {
        val error = runCatching { HostRecoveryDispatch(maxSyncInFlight = 0) }.exceptionOrNull()
        assertThat(error).isInstanceOf(IllegalArgumentException::class.java)
    }
}
