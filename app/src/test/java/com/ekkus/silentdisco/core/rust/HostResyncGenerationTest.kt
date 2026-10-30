package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.Mockito

/** A late ResyncNotice cannot replay an old stream into a replacement host. */
class HostResyncGenerationTest {
    @Test
    fun recoveryReplaysOnlyToOriginalAuthorizedHostGeneration() = runTest {
        val old = Mockito.mock(FfiHostTransportHandle::class.java)
        val fresh = Mockito.mock(FfiHostTransportHandle::class.java)
        for (handle in listOf(old, fresh)) {
            Mockito.doAnswer { Thread.sleep(10); null }.`when`(handle).pollEvent(500uL)
        }
        val delivery = FfiHostTransportDelivery(1u, 1u, 0u, 120uL)
        // Configure before launching the native polling coroutine.
        Mockito.doReturn(delivery).`when`(fresh).replayCachedPlaybackToListener("listener")
        val handles = ArrayDeque(listOf(old, fresh))
        val controller = HostTransportController(
            callDispatcher = Dispatchers.Unconfined,
            bindHandle = { _, _, _, _, _ -> handles.removeFirst() },
        )
        try {
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "old-session")
            controller.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "new-session")
            assertThat(controller.replayCachedPlaybackForGeneration(old, "listener")).isNull()
            Mockito.verify(old, Mockito.never()).replayCachedPlaybackToListener("listener")
            Mockito.verify(fresh, Mockito.never()).replayCachedPlaybackToListener("listener")
            assertThat(controller.replayCachedPlaybackForGeneration(fresh, "listener")).isEqualTo(delivery)
            Mockito.verify(fresh).replayCachedPlaybackToListener("listener")
        } finally {
            controller.close()
        }
    }
}
