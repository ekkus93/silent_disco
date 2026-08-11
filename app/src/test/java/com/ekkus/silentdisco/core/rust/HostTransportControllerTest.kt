package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.onCompletion
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class HostTransportControllerTest {
    @Test
    fun closeReleasesSessionWithoutClosingReusableEventStream() = runTest {
        val controller = HostTransportController()
        var eventStreamCompleted = false
        val collector = backgroundScope.launch {
            controller.events
                .onCompletion { eventStreamCompleted = true }
                .collect()
        }
        runCurrent()

        controller.close()
        runCurrent()

        assertThat(eventStreamCompleted).isFalse()
        collector.cancel()
    }
}
