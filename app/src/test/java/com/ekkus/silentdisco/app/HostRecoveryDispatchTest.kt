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
        val handle = Any()
        repeat(4) {
            assertThat(limiter.launchSync(scope, dispatcher, handle, "peer-a") { awaitCancellation() }).isTrue()
        }
        repeat(256) {
            assertThat(limiter.launchSync(scope, dispatcher, handle, "peer-a") { error("unreachable overflow") }).isFalse()
        }
        assertThat(limiter.activeSyncRequests).isEqualTo(4)
        assertThat(limiter.rejectedSyncRequests).isEqualTo(256)
        scope.cancel()
        advanceUntilIdle()
        assertThat(limiter.activeSyncRequests).isEqualTo(0)

        val replacement = CoroutineScope(SupervisorJob() + dispatcher)
        var delivered = 0
        repeat(4) {
            assertThat(limiter.launchSync(replacement, dispatcher, handle, "peer-a") { delivered++ }).isTrue()
        }
        advanceUntilIdle()
        assertThat(delivered).isEqualTo(4)
        assertThat(limiter.activeSyncRequests).isEqualTo(0)
        replacement.cancel()
    }

    @Test
    fun noisyPeerCannotStarveOtherListenersOrReplacementGeneration() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val limiter = HostRecoveryDispatch(maxSyncInFlight = 5, maxSyncPerListener = 2)
        val generation = Any()
        val replacement = Any()
        repeat(2) {
            assertThat(limiter.launchSync(scope, dispatcher, generation, "noisy") {
                awaitCancellation()
            }).isTrue()
        }
        repeat(200) {
            assertThat(limiter.launchSync(scope, dispatcher, generation, "noisy") {
                error("per-listener quota must reject")
            }).isFalse()
        }
        assertThat(limiter.rejectedSyncRequests).isEqualTo(200)
        // A distinct peer and the same peer on a replacement host each get
        // their own quota, even when the old native generation is still busy.
        assertThat(limiter.launchSync(scope, dispatcher, generation, "quiet") {
            awaitCancellation()
        }).isTrue()
        assertThat(limiter.launchSync(scope, dispatcher, replacement, "noisy") {
            awaitCancellation()
        }).isTrue()
        assertThat(limiter.activeSyncRequests).isEqualTo(4)
        assertThat(limiter.activeSyncListeners).isEqualTo(3)
        scope.cancel()
        advanceUntilIdle()
        assertThat(limiter.activeSyncRequests).isEqualTo(0)
        assertThat(limiter.activeSyncListeners).isEqualTo(0)
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
    fun repeatedNoticeDuringBlockedReplayCausesOneFollowupNotSilentLoss() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val limiter = HostRecoveryDispatch()
        val handle = Any()
        val unblock = kotlinx.coroutines.CompletableDeferred<Unit>()
        var replays = 0
        assertThat(limiter.launchResync(scope, dispatcher, handle, "listener") {
            replays++
            if (replays == 1) unblock.await()
        }).isTrue()
        advanceUntilIdle()
        assertThat(replays).isEqualTo(1)
        repeat(256) {
            assertThat(limiter.launchResync(scope, dispatcher, handle, "listener") {
                error("duplicate must coalesce into the original replay body")
            }).isFalse()
        }
        assertThat(limiter.activeResyncRequests).isEqualTo(1)
        unblock.complete(Unit)
        advanceUntilIdle()
        assertThat(replays).isEqualTo(2)
        assertThat(limiter.activeResyncRequests).isEqualTo(0)
        // A notice after the final replay must start a new generation-scoped
        // operation, not be lost in the completion-to-retirement race.
        assertThat(limiter.launchResync(scope, dispatcher, handle, "listener") {
            replays++
        }).isTrue()
        advanceUntilIdle()
        assertThat(replays).isEqualTo(3)
        scope.cancel()
    }

    @Test
    fun invalidSyncLimitCannotEnableAnUnboundedQueue() {
        val error = runCatching { HostRecoveryDispatch(maxSyncInFlight = 0) }.exceptionOrNull()
        assertThat(error).isInstanceOf(IllegalArgumentException::class.java)
    }
}
