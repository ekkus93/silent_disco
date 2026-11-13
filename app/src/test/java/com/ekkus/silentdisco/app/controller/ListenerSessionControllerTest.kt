package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.rust.FakeListenerCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerTransport
import com.ekkus.silentdisco.core.rust.ListenerCoreController
import com.ekkus.silentdisco.core.transport.FakeBleTransport
import com.ekkus.silentdisco.core.transport.FakeMdnsTransport
import com.ekkus.silentdisco.core.transport.FakeSessionTransport
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiPlatformCompletion
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ListenerSessionControllerTest {
    private val transport = FakeListenerTransport()
    private val wifiDirect = FakeSessionTransport()
    private val core = FakeListenerCoreController()
    private val callLog = mutableListOf<String>()
    private var invite: String? = "1234"
    private val session = SessionInfo(
        id = "session-1",
        name = "Patio",
        hostDeviceName = "host",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = true,
    )

    private val callbacks = object : ListenerSessionCallbacks {
        override fun discoveredSession(sessionId: String) = session.takeIf { it.id == sessionId }
        override val selectedSession: SessionInfo get() = session
        override val inviteCode: String? get() = invite
        override fun hasNearbyPermissions() = false
        override fun onSnapshot(snapshot: FfiCoreSnapshot) = Unit
        override fun onTransportEvent(core: ListenerCoreController, event: FfiListenerTransportEvent) = Unit
    }

    private fun TestScope.controller(): Pair<ListenerSessionController, CoroutineScope> {
        val parent = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))
        transport.callLog = callLog
        core.callLog = callLog
        val discovery = DiscoveryController(parent, FakeBleTransport(), FakeMdnsTransport(), wifiDirect) { null }
        val listener = ListenerSessionController(
            parentScope = parent,
            transport = transport,
            wifiDirect = wifiDirect,
            discovery = discovery,
            coreFactory = { core },
            deviceId = { "device-1" },
            logger = AppLogger(),
            callbacks = callbacks,
        )
        listener.ensureCore()
        return listener to parent
    }

    private fun establish(operationId: String, address: String? = "192.168.1.50") =
        FfiCoreNotification.PlatformEffect(
            FfiPlatformEffect.EstablishNetwork(
                operationId = operationId,
                sessionId = "session-1",
                address = address,
                controlPort = address?.let { 41_000u },
                syncPort = address?.let { 41_001u },
                audioPort = address?.let { 41_002u },
            ),
        )

    @Test
    fun anEndpointIsReportedReadyBeforeTheJoinIsSentAndPinsTheInvitedKey() = runTest {
        val (listener, parent) = controller()
        val key = byteArrayOf(1, 2, 3)
        listener.rememberInvitedHostTlsKey("session-1", key)

        core.emit(establish("op-1"))
        advanceUntilIdle()

        assertThat(transport.connectCalls.single().localDeviceId).isEqualTo("device-1")
        assertThat(transport.connectCalls.single().hostTlsPublicKeyDer).isSameInstanceAs(key)
        assertThat(callLog).containsExactly(
            "core.platformOperationSucceeded:NetworkEndpointReady",
            "transport.sendJoinRequest",
        ).inOrder()
        assertThat(transport.joinRequestCalls.single().second).isEqualTo("1234")
        assertThat(listener.transportConnected).isTrue()
        assertThat(listener.ridesWifiDirect).isFalse()
        parent.cancel()
    }

    @Test
    fun aFailedJoinSendDisconnectsAndReportsARetryableFailure() = runTest {
        val (listener, parent) = controller()
        transport.joinRequestFailure = IllegalStateException("stale socket")

        core.emit(establish("op-1"))
        advanceUntilIdle()

        assertThat(listener.transportConnected).isFalse()
        assertThat(transport.disconnectCalls).containsExactly("Join request failed")
        assertThat(core.transportFailedCalls).containsExactly("stale socket" to true)
        parent.cancel()
    }

    @Test
    fun aWifiDirectJoinWaitsForTheGroupAndFailsWithoutAHostAddress() = runTest {
        val (listener, parent) = controller()

        core.emit(establish("op-wd", address = null))
        advanceUntilIdle()
        assertThat(wifiDirect.connectToSessionCalls).containsExactly(session)
        assertThat(listener.hasPendingWifiDirectJoin).isTrue()
        assertThat(transport.connectCalls).isEmpty()

        listener.completeWifiDirectEstablishment(TransportSnapshot(state = TransportConnectionState.CONNECTING))
        assertThat(listener.pendingEstablishOperationId).isEqualTo("op-wd")

        listener.completeWifiDirectEstablishment(TransportSnapshot(state = TransportConnectionState.CONNECTED))
        advanceUntilIdle()
        assertThat(listener.pendingEstablishOperationId).isNull()
        assertThat(core.platformOperationFailedCalls.single().operationId).isEqualTo("op-wd")
        assertThat(transport.connectCalls).isEmpty()
        parent.cancel()
    }

    @Test
    fun aConnectedWifiDirectGroupOpensTheEndpointAtItsHostAddress() = runTest {
        val (listener, parent) = controller()
        core.emit(establish("op-wd", address = null))
        advanceUntilIdle()

        listener.completeWifiDirectEstablishment(
            TransportSnapshot(state = TransportConnectionState.CONNECTED, hostAddressHint = "192.168.49.1"),
        )
        advanceUntilIdle()

        assertThat(transport.connectCalls.single().rawEndpoint).contains("192.168.49.1")
        assertThat(core.platformOperationSucceededCalls.single().operationId).isEqualTo("op-wd")
        assertThat(listener.ridesWifiDirect).isTrue()
        parent.cancel()
    }

    @Test
    fun releasingTheNetworkClearsEveryFlagAndStopsBothTransports() = runTest {
        val (listener, parent) = controller()
        core.emit(establish("op-1"))
        advanceUntilIdle()

        core.emit(FfiCoreNotification.PlatformEffect(FfiPlatformEffect.ReleaseNetwork(operationId = "op-release")))
        advanceUntilIdle()

        assertThat(listener.transportConnected).isFalse()
        assertThat(listener.hasPendingWifiDirectJoin).isFalse()
        assertThat(transport.disconnectCalls).containsExactly("Listener released network")
        assertThat(wifiDirect.stopCallCount).isEqualTo(1)
        assertThat(core.platformOperationSucceededCalls.last().completion)
            .isEqualTo(FfiPlatformCompletion.NetworkReleased)
        parent.cancel()
    }
}
