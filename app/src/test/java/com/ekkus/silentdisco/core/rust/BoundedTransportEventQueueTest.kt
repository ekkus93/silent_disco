package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class BoundedTransportEventQueueTest {
    @Test
    fun slowCollectorBackpressuresCriticalEventsWithoutDroppingOrReordering() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ handle }, capacity = 2)
        queue.emit(handle, "join-one")
        queue.emit(handle, "join-two")
        val pendingStop = backgroundScope.async { queue.emit(handle, "stop") }
        runCurrent()
        assertThat(pendingStop.isCompleted).isFalse()

        val received = backgroundScope.async { queue.events.take(3).toList() }
        runCurrent()
        assertThat(received.await()).containsExactly("join-one", "join-two", "stop").inOrder()
        assertThat(pendingStop.isCompleted).isTrue()
        assertThat(queue.droppedBestEffort).isEqualTo(0)
        queue.close()
    }

    @Test
    fun bestEffortAudioIsBoundedAndDoesNotEvictExistingControl() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ handle }, capacity = 2)
        queue.emit(handle, "approval")
        queue.tryEmit(handle, "audio-one")
        repeat(1000) { queue.tryEmit(handle, "audio-overflow-$it") }
        assertThat(queue.droppedBestEffort).isEqualTo(1000)

        val received = backgroundScope.async { queue.events.take(2).toList() }
        runCurrent()
        assertThat(received.await()).containsExactly("approval", "audio-one").inOrder()
        queue.close()
    }

    @Test
    fun highRateAudioReservesSixOfEightSlotsForAuthoritativeControls() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ handle }, capacity = 8)
        repeat(1000) { queue.tryEmit(handle, "audio-$it") }
        assertThat(queue.droppedBestEffort).isEqualTo(998)
        val controls = listOf("join-a", "join-b", "approval", "pause", "stop", "disconnect")
        for (control in controls) queue.emit(handle, control)

        // Only after all six reserved slots are occupied may a further
        // authoritative event wait. A stopped collector never lets a burst
        // of lossy PCM monopolize the whole native-to-UI bridge.
        val waiting = backgroundScope.async { queue.emit(handle, "terminal-error") }
        runCurrent()
        assertThat(waiting.isCompleted).isFalse()
        val received = backgroundScope.async { queue.events.take(9).toList() }
        runCurrent()
        assertThat(received.await())
            .containsExactly("audio-0", "audio-1", *controls.toTypedArray(), "terminal-error")
            .inOrder()
        assertThat(waiting.isCompleted).isTrue()
        queue.close()
    }

    @Test
    fun retiredSessionEventsNeverAppearAfterRebind() = runTest {
        val old = Any()
        val replacement = Any()
        var current = old
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 2)
        queue.emit(old, "old-approval")
        current = replacement
        queue.emit(replacement, "new-stop")

        val received = backgroundScope.async { queue.events.first() }
        runCurrent()
        assertThat(received.await()).isEqualTo("new-stop")
        queue.close()
    }

    @Test
    fun retiredBestEffortEventsReleaseQuotaBeforeNewSessionAudio() = runTest {
        val old = Any()
        val replacement = Any()
        var current = old
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 4)
        queue.tryEmit(old, "old-audio")
        current = replacement
        val draining = backgroundScope.async { queue.events.first() }
        runCurrent()
        queue.tryEmit(replacement, "new-audio")
        runCurrent()
        assertThat(draining.await()).isEqualTo("new-audio")
        assertThat(queue.droppedBestEffort).isEqualTo(0)
        queue.close()
    }

    @Test
    fun rebindFreesOldCriticalAndAudioCapacityBeforeNewSessionEvents() = runTest {
        val retired = Any()
        val replacement = Any()
        var current = retired
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 3)
        queue.emit(retired, "old-join")
        queue.tryEmit(retired, "old-audio")
        queue.emit(retired, "old-disconnect")

        // No collector is running. Without retirement, the replacement
        // session would stall even though none of these events is usable.
        current = replacement
        queue.discardRetiredEvents()
        queue.emit(replacement, "new-join")
        queue.tryEmit(replacement, "new-audio")
        queue.emit(replacement, "new-stop")
        val received = backgroundScope.async { queue.events.take(3).toList() }
        runCurrent()
        assertThat(received.await())
            .containsExactly("new-join", "new-audio", "new-stop").inOrder()
        assertThat(queue.droppedBestEffort).isEqualTo(0)
        queue.close()
    }

    @Test
    fun cancellingRetiredPollerBeforeRebindDoesNotLeakItsPendingCriticalSend() = runTest {
        val retired = Any()
        val replacement = Any()
        var current = retired
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 1)
        queue.emit(retired, "old-join")
        val oldPoll = backgroundScope.async { queue.emit(retired, "old-stop") }
        runCurrent()
        assertThat(oldPoll.isCompleted).isFalse()
        oldPoll.cancel()
        runCurrent()

        current = replacement
        queue.discardRetiredEvents()
        queue.emit(replacement, "new-approval")
        val received = backgroundScope.async { queue.events.first() }
        runCurrent()
        assertThat(received.await()).isEqualTo("new-approval")
        queue.close()
    }

    @Test
    fun lateRetiredPcmCannotConsumeFreshSessionCriticalCapacityWithoutCollector() = runTest {
        val retired = Any()
        val replacement = Any()
        var current = retired
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 4)
        queue.tryEmit(retired, "old-audio-before-rebind")
        current = replacement
        queue.discardRetiredEvents()

        // Simulate the old poller resuming after the generation check it made
        // before rebind. A thousand stale events must not reserve any slot or
        // consume the new session's best-effort quota while the UI is stopped.
        repeat(1_000) { queue.tryEmit(retired, "late-old-audio-$it") }
        assertThat(queue.droppedBestEffort).isEqualTo(0)
        val pending = backgroundScope.async {
            repeat(4) { queue.emit(replacement, "critical-$it") }
        }
        runCurrent()
        assertThat(pending.isCompleted).isTrue()
        assertThat(queue.droppedBestEffort).isEqualTo(0)

        val received = backgroundScope.async { queue.events.take(4).toList() }
        runCurrent()
        assertThat(received.await())
            .containsExactly("critical-0", "critical-1", "critical-2", "critical-3")
            .inOrder()
        queue.tryEmit(replacement, "fresh-audio")
        val fresh = backgroundScope.async { queue.events.first() }
        runCurrent()
        assertThat(fresh.await()).isEqualTo("fresh-audio")
        queue.close()
    }

    @Test
    fun nonCancelledBlockedCriticalSendRetiresWithoutOccupyingNewSession() = runTest {
        val old = Any()
        val replacement = Any()
        var current = old
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 1)
        queue.emit(old, "old-join")
        // Deliberately leave this producer alive across rebind: cancellation
        // cannot be the only guard against old-generation queue pollution.
        val stale = backgroundScope.async { queue.emit(old, "old-stop") }
        runCurrent()
        assertThat(stale.isCompleted).isFalse()

        current = replacement
        queue.discardRetiredEvents()
        runCurrent()
        assertThat(stale.isCompleted).isTrue()
        queue.emit(replacement, "fresh-join")
        val received = backgroundScope.async { queue.events.first() }
        runCurrent()
        assertThat(received.await()).isEqualTo("fresh-join")
        queue.close()
    }

    @Test
    fun repeatedRebindRetiresWaitingCriticalProducersWithoutCollector() = runTest {
        var current = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ current }, capacity = 1)
        repeat(128) { generation ->
            queue.emit(current, "queued-$generation")
            val retired = current
            val waiting = backgroundScope.async { queue.emit(retired, "stale-stop-$generation") }
            runCurrent()
            assertThat(waiting.isCompleted).isFalse()
            current = Any()
            queue.discardRetiredEvents()
            runCurrent()
            assertThat(waiting.isCompleted).isTrue()
        }
        queue.emit(current, "new-stop")
        val received = backgroundScope.async { queue.events.first() }
        runCurrent()
        assertThat(received.await()).isEqualTo("new-stop")
        queue.close()
    }

    @Test
    fun closingQueueUnblocksPendingAuthoritativeEventAndPreservesAlreadyQueuedJoin() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ handle }, capacity = 1)
        queue.emit(handle, "join")
        val waiting = backgroundScope.async {
            runCatching { queue.emit(handle, "stop") }.exceptionOrNull()
        }
        runCurrent()
        assertThat(waiting.isCompleted).isFalse()
        queue.close()
        runCurrent()
        assertThat(waiting.await()).isNotNull()
        val received = backgroundScope.async { queue.events.toList() }
        runCurrent()
        assertThat(received.await()).containsExactly("join")
        assertThat(queue.droppedBestEffort).isEqualTo(0)
    }
    @Test
    fun authoritativeResyncAndStopSurvivePureAudioFloodWithoutCollector() = runTest {
        val handle = Any()
        val queue = BoundedTransportEventQueue<Any, String>({ handle }, capacity = 4)
        repeat(1_000) { queue.tryEmit(handle, "audio-$it") }
        assertThat(queue.droppedBestEffort).isEqualTo(999)
        // Resync is a required host control event, not a lossy heartbeat.
        queue.emit(handle, "resync-notice")
        queue.emit(handle, "stop")
        queue.emit(handle, "terminal-disconnect")
        val received = backgroundScope.async { queue.events.take(4).toList() }
        runCurrent()
        assertThat(received.await())
            .containsExactly("audio-0", "resync-notice", "stop", "terminal-disconnect")
            .inOrder()
        queue.close()
    }

    @Test
    fun rapidRebindRejectsRetiredCriticalFloodWithoutStarvingFreshJoinStop() = runTest {
        val gate = HostNativeHandleGate<Any>()
        val queue = BoundedTransportEventQueue<Any, String>(gate::currentHandle, capacity = 4)
        var retired: Any? = null
        repeat(128) { generation ->
            val fresh = Any()
            gate.replace(
                retire = {},
                open = { fresh },
                start = { queue.discardRetiredEvents() },
            )
            // A poller may have passed its outer generation check before
            // close/rebind. Its late authoritative events must not occupy the
            // replacement's bounded bridge while the UI collector is stopped.
            retired?.let { stale ->
                repeat(100) { queue.emit(stale, "retired-$generation-$it") }
            }
            queue.emit(fresh, "join-$generation")
            queue.tryEmit(fresh, "audio-$generation")
            queue.emit(fresh, "stop-$generation")
            retired = fresh
        }
        val received = backgroundScope.async { queue.events.take(3).toList() }
        runCurrent()
        assertThat(received.await())
            .containsExactly("join-127", "audio-127", "stop-127").inOrder()
        assertThat(queue.droppedBestEffort).isEqualTo(0)
        gate.close { queue.discardRetiredEvents() }
        queue.close()
    }

}
