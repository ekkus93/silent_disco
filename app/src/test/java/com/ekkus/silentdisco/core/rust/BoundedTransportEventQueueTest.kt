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
}
