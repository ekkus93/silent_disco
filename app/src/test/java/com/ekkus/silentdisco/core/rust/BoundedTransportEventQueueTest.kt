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
}
