package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.rust.FakeHostCoreController
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.rust.DURABLE_LISTENER_TRUST_UNAVAILABLE
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiCoreError
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiDeliveryReport
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiStorageEffect
import com.ekkus.silentdisco.core.uniffi.FfiTransportEffect
import com.ekkus.silentdisco.core.uniffi.FfiTuningSettings
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
import org.mockito.Mockito.mock

@OptIn(ExperimentalCoroutinesApi::class)
class HostSessionControllerTest {
    private val core = FakeHostCoreController()
    private val transport = HostTransportController()
    private val failures = mutableListOf<Pair<String, String?>>()
    private val coreErrors = mutableListOf<String>()
    private val recovered = mutableListOf<FfiCoreNotification>()
    private val saved = mutableListOf<RustStoredTuningSettings>()
    private var saveFailure: Exception? = null
    private val platformEffects = mutableListOf<FfiPlatformEffect>()
    private var platformFailure: Exception? = null

    private val callbacks = object : HostSessionCallbacks {
        override fun onSnapshot(snapshot: FfiCoreSnapshot) = Unit
        override suspend fun executePlatformEffect(core: HostCoreController, effect: FfiPlatformEffect) {
            platformEffects += effect
            platformFailure?.let { throw it }
        }
        override fun onCoreError(message: String) {
            coreErrors += message
        }
        override fun recoverNotificationFailure(
            core: HostCoreController,
            notification: FfiCoreNotification,
            error: Exception,
        ) {
            recovered += notification
        }
        override fun commandFailed(action: String, error: Throwable) {
            failures += action to error.message
        }
        override fun pendingJoinDisplayName(listenerId: String): String? = null
        override suspend fun saveTuning(settings: RustStoredTuningSettings) {
            saveFailure?.let { throw it }
            saved += settings
        }
    }

    private fun TestScope.controller(): Pair<HostSessionController, CoroutineScope> {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val parent = CoroutineScope(SupervisorJob() + dispatcher)
        val session = HostSessionController(
            parentScope = parent,
            transport = transport,
            coreFactory = { core },
            deviceId = { "host-1" },
            logger = AppLogger(),
            callbacks = callbacks,
            ioDispatcher = dispatcher,
        )
        return session to parent
    }

    private val tuning = FfiTuningSettings(
        syncSampleWindow = 12u,
        syncCadenceMs = 2_000uL,
        startupBufferMs = 400uL,
        latePacketThresholdMs = 40uL,
        hardResyncThresholdMs = 120uL,
        syncDriftThresholdMs = 18.0,
        scanWindowMs = 3_000uL,
    )

    private val failedDelivery = FfiDeliveryReport(intendedPeers = 1u, successfulPeers = 0u, failedPeers = 1u)

    @Test
    fun aJoinIsTrackedAndSubmittedAndADisconnectReleasesItsRoute() = runTest {
        val (session, parent) = controller()
        val handle = mock(FfiHostTransportHandle::class.java)

        session.handleTransportEvent(
            core,
            FfiHostTransportEvent.JoinRequestReceived(
                controlAddress = "127.0.0.1:10001",
                deviceId = "listener-1",
                displayName = "Alex",
                inviteCode = null,
                syncPort = 101u,
                audioPort = 102u,
            ),
            handle,
        )
        assertThat(core.submitJoinRequestCalls.single().deviceId).isEqualTo("listener-1")
        assertThat(session.joinRoutes.size).isEqualTo(1)

        session.handleTransportEvent(
            core,
            FfiHostTransportEvent.PeerDisconnected(listenerId = "listener-1", message = null),
            handle,
        )
        assertThat(session.joinRoutes.size).isEqualTo(0)
        assertThat(core.submitListenerDisconnectedCalls).containsExactly("listener-1")
        parent.cancel()
    }

    @Test
    fun anApprovalWithoutItsJoinRouteOrAskingForDurableTrustIsReportedAsUndelivered() = runTest {
        val (session, parent) = controller()
        session.ensureCore()
        listOf(false to "route missing", true to DURABLE_LISTENER_TRUST_UNAVAILABLE).forEachIndexed { index, (trust, reason) ->
            core.emit(
                FfiCoreNotification.TransportEffect(
                    FfiTransportEffect.DeliverJoinApproval(
                        operationId = "op-$index",
                        requestId = "req-$index",
                        sessionId = "session-1",
                        listenerId = "listener-1",
                        trustedForFuture = trust,
                    ),
                ),
            )
            advanceUntilIdle()
            assertThat(core.transportDeliveryCompletedCalls.last()).isEqualTo("op-$index" to failedDelivery)
            assertThat(failures.last().second).contains(reason)
        }
        assertThat(core.submitListenerConnectedCalls).isEmpty()
        parent.cancel()
    }

    @Test
    fun settingsAreSavedOrFailRetryablyAndDurableTrustIsRefused() = runTest {
        val (session, parent) = controller()
        session.ensureCore()

        core.emit(FfiCoreNotification.StorageEffect(FfiStorageEffect.PersistSettings("op-save", tuning)))
        advanceUntilIdle()
        assertThat(saved.single().syncSampleWindow).isEqualTo(12)
        assertThat(core.settingsSavedCalls).containsExactly("op-save")

        saveFailure = IllegalStateException("disk full")
        core.emit(FfiCoreNotification.StorageEffect(FfiStorageEffect.PersistSettings("op-fail", tuning)))
        core.emit(
            FfiCoreNotification.StorageEffect(
                FfiStorageEffect.PersistTrustedDevice(operationId = "op-trust", deviceId = "l", displayName = "L"),
            ),
        )
        advanceUntilIdle()
        assertThat(core.storageOperationFailedCalls.map { it.operationId to it.retryable })
            .containsExactly("op-fail" to true, "op-trust" to false).inOrder()
        parent.cancel()
    }

    @Test
    fun aThrowingEffectIsRecoveredAndTheLoopKeepsRunning() = runTest {
        val (session, parent) = controller()
        assertThat(session.ensureCore()).isSameInstanceAs(session.ensureCore())
        platformFailure = IllegalStateException("bluetooth stack exploded")
        val effect = FfiCoreNotification.PlatformEffect(
            FfiPlatformEffect.RequestCapabilities(operationId = "op-cap", capabilities = emptyList()),
        )

        core.emit(effect)
        core.emit(FfiCoreNotification.Error(FfiCoreError(code = "E", subsystem = "host", severity = "error", retryable = false, operationId = null, message = "actor said no")))
        advanceUntilIdle()

        assertThat(recovered).containsExactly(effect)
        assertThat(coreErrors).containsExactly("actor said no")
        parent.cancel()
    }

    @Test
    fun advertisingCompletesOnlyForAPendingOperationOnceTheGroupHasAnAddress() = runTest {
        val (session, parent) = controller()
        session.ensureCore()
        session.sessionId = SessionId("session-1")

        session.completeAdvertising(TransportSnapshot(state = TransportConnectionState.ADVERTISING, hostAddressHint = "1"))
        assertThat(core.platformOperationSucceededCalls).isEmpty()

        session.pendingStartAdvertisingOperationId = "op-adv"
        session.completeAdvertising(TransportSnapshot(state = TransportConnectionState.CONNECTING, hostAddressHint = "1"))
        session.completeAdvertising(TransportSnapshot(state = TransportConnectionState.ADVERTISING))
        assertThat(session.pendingStartAdvertisingOperationId).isEqualTo("op-adv")
        assertThat(core.platformOperationSucceededCalls).isEmpty()
        assertThat(core.platformOperationFailedCalls).isEmpty()
        parent.cancel()
    }
}
