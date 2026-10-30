package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportDelivery
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.mockito.Mockito

/** Real Android native-event projection, not a direct controller-only test. */
@OptIn(ExperimentalCoroutinesApi::class)
class HostResyncProjectionTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    @Test
    fun listenerResyncNoticeReplaysAuthoritativeControlsToOriginalNativeSession() = runTest(dispatcher) {
        val native = Mockito.mock(FfiHostTransportHandle::class.java)
        Mockito.doAnswer { Thread.sleep(10); null }.`when`(native).pollEvent(500uL)
        // Stub before native polling begins: concurrent Mockito stubbing and
        // polling on one mock is not supported.
        Mockito.doReturn(FfiHostTransportDelivery(1u, 1u, 0u, 120uL))
            .`when`(native).replayCachedPlaybackToListener("listener")
        val transport = HostTransportController(
            callDispatcher = Dispatchers.IO,
            bindHandle = { _, _, _, _, _ -> native },
        )
        val harness = newTestMainViewModelHarness(hostTransportController = transport)
        try {
            transport.bind(backgroundScope, "127.0.0.1", 1, 2, 3, "session-1")
            harness.viewModel.handleHostTransportEvent(
                harness.hostCoreController,
                FfiHostTransportEvent.ResyncNotice(
                    listenerId = "listener",
                    reason = "lost current track boundary",
                ),
                native,
            )
            Mockito.verify(native, Mockito.timeout(2_000))
                .replayCachedPlaybackToListener("listener")
        } finally {
            transport.close()
        }
    }
}
