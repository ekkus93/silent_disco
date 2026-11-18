package com.ekkus.silentdisco.app.controller

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.core.identity.HostTlsIdentityManager
import com.ekkus.silentdisco.core.identity.KeystoreHostTlsSigner
import com.ekkus.silentdisco.core.identity.KeystoreListenerProofSigner
import com.ekkus.silentdisco.core.identity.ListenerIdentityManager
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.rust.HostTransportController
import com.ekkus.silentdisco.core.rust.RustDatabase
import com.ekkus.silentdisco.core.rust.RustDatabaseBridge
import com.ekkus.silentdisco.core.rust.RustStoredTuningSettings
import com.ekkus.silentdisco.core.rust.UniFfiHostCoreController
import com.ekkus.silentdisco.core.transport.TransportSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import com.ekkus.silentdisco.core.uniffi.FfiAudioSource
import com.ekkus.silentdisco.core.uniffi.FfiCoreNotification
import com.ekkus.silentdisco.core.uniffi.FfiCoreSnapshot
import com.ekkus.silentdisco.core.uniffi.FfiHostDraft
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiListenerTransportHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlatformEffect
import com.ekkus.silentdisco.core.uniffi.FfiTrustState
import com.ekkus.silentdisco.core.uniffi.listVerifiedListenersInDatabase
import com.ekkus.silentdisco.core.uniffi.listenerDeviceIdForKey
import com.ekkus.silentdisco.core.uniffi.revokeVerifiedListenerInDatabase
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.security.KeyStore
import java.util.Collections
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Remediation F1 on the phone, through the real Rust core: the same
 * [HostSessionController] the app uses, the real v4 QUIC host transport on
 * loopback, real v4 listeners proving Keystore keys, and a real Rust
 * database. Everything is test-only: its own database file and its own
 * Keystore aliases, never the app's stores.
 */
@RunWith(AndroidJUnit4::class)
class HostVerifiedTrustDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val databaseFile = File(context.cacheDir, "host-verified-trust-test.sqlite3")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val failures = Collections.synchronizedList(mutableListOf<String>())
    private val hostSigner = KeystoreHostTlsSigner(HostTlsIdentityManager(HOST_ALIAS))
    private lateinit var database: RustDatabase
    private lateinit var transport: HostTransportController
    private lateinit var core: HostCoreController
    private lateinit var session: HostSessionController
    private val listeners = mutableListOf<FfiListenerTransportHandle>()

    @Before
    fun setUp() {
        deleteDatabaseFiles()
        database = RustDatabaseBridge.open(databaseFile.absolutePath)
        transport = HostTransportController(hostTlsSigner = hostSigner)
        core = UniFfiHostCoreController(HOST_DEVICE_ID)
        session = HostSessionController(
            parentScope = scope,
            transport = transport,
            coreFactory = { core },
            deviceId = { HOST_DEVICE_ID },
            logger = AppLogger(),
            callbacks = callbacks,
        )
    }

    @After
    fun tearDown() {
        listeners.forEach { runCatching { it.shutdown() } }
        runCatching { transport.close() }
        runCatching { (core as? AutoCloseable)?.close() }
        scope.cancel()
        runCatching { database.close() }
        deleteDatabaseFiles()
        KeyStore.getInstance("AndroidKeyStore").apply {
            load(null)
            listOf(HOST_ALIAS, LISTENER_A_ALIAS, LISTENER_B_ALIAS).forEach { alias ->
                if (containsAlias(alias)) deleteEntry(alias)
            }
        }
    }

    private val callbacks = object : HostSessionCallbacks {
        override fun onSnapshot(snapshot: FfiCoreSnapshot) = Unit

        override suspend fun executePlatformEffect(core: HostCoreController, effect: FfiPlatformEffect) {
            if (effect is FfiPlatformEffect.StartAdvertising) {
                // What MainViewModel does once its Wi-Fi Direct group forms;
                // here the group is loopback.
                session.sessionId = SessionId(effect.sessionId)
                session.pendingStartAdvertisingOperationId = effect.operationId
                session.completeAdvertising(
                    TransportSnapshot(
                        state = TransportConnectionState.ADVERTISING,
                        hostAddressHint = LOOPBACK,
                    ),
                )
            }
        }

        override fun onCoreError(message: String) {
            failures += "core: $message"
        }

        override fun recoverNotificationFailure(
            core: HostCoreController,
            notification: FfiCoreNotification,
            error: Exception,
        ) {
            failures += "notification: ${error.message}"
        }

        override fun commandFailed(action: String, error: Throwable) {
            failures += "$action: ${error.message}"
        }

        override fun pendingJoinDisplayName(listenerId: String): String? =
            core.snapshots.value?.pendingJoinRequests?.firstOrNull { it.deviceId == listenerId }?.displayName

        override suspend fun saveTuning(settings: RustStoredTuningSettings) = Unit

        override suspend fun rememberVerifiedListener(
            core: HostCoreController,
            deviceId: String,
            displayName: String,
        ) {
            database.withNativeHandle { handle ->
                core.enrollVerifiedListener(
                    handle,
                    hostSigner.spkiDer(),
                    deviceId,
                    displayName,
                    System.currentTimeMillis(),
                )
            }
        }
    }

    @Test
    fun approvedDevicesAdmitsOnlyARememberedUnrevokedKey() = runBlocking {
        val remembered = KeystoreListenerProofSigner(ListenerIdentityManager(LISTENER_A_ALIAS))
        val stranger = KeystoreListenerProofSigner(ListenerIdentityManager(LISTENER_B_ALIAS))
        val rememberedId = listenerDeviceIdForKey(remembered.spkiDer())
        session.ensureCore()
        database.withNativeHandle { handle ->
            core.enrollVerifiedListener(handle, hostSigner.spkiDer(), rememberedId, "Kitchen", 1L)
        }
        startHosting(FfiApprovalMode.TRUSTED_DEVICES)

        val first = connect(remembered)
        assertThat(awaitDecision(first)).isEqualTo(Decision.Remembered)
        awaitListenerTrust(rememberedId, FfiTrustState.TRUSTED)

        assertThat(awaitDecision(connect(stranger))).isEqualTo(Decision.Rejected)

        database.withNativeHandle { handle ->
            revokeVerifiedListenerInDatabase(
                handle,
                hostSigner.spkiDer(),
                rememberedId,
                System.currentTimeMillis().toULong(),
            )
        }
        // Owner decision 2026-11-18: the running host disconnects a revoked
        // listener at once, not only at its next join.
        assertThat(awaitHostDisconnect(first)).isEqualTo("host_revoked_listener")
        awaitNoConnectedListener(rememberedId)
        assertThat(awaitDecision(connect(remembered))).isEqualTo(Decision.Rejected)
        assertThat(failures).isEmpty()
    }

    @Test
    fun alwaysAllowStoresTheRecordAndConnectsTheListenerAsTrusted() = runBlocking {
        val phone = KeystoreListenerProofSigner(ListenerIdentityManager(LISTENER_A_ALIAS))
        val phoneId = listenerDeviceIdForKey(phone.spkiDer())
        session.ensureCore()
        startHosting(FfiApprovalMode.MANUAL)

        val listener = connect(phone)
        val request = awaitUntil("a pending join from the listener") {
            core.snapshots.value?.pendingJoinRequests?.firstOrNull { it.deviceId == phoneId }
        }
        core.approveJoin(request.requestId, rememberForFuture = true)

        assertThat(awaitDecision(listener)).isEqualTo(Decision.Remembered)
        awaitListenerTrust(phoneId, FfiTrustState.TRUSTED)
        val stored = database.withNativeHandle { handle ->
            listVerifiedListenersInDatabase(handle, hostSigner.spkiDer())
        }
        assertThat(stored.map { it.deviceId }).containsExactly(phoneId)
        assertThat(failures).isEmpty()
    }

    private suspend fun startHosting(mode: FfiApprovalMode) {
        database.withNativeHandle { handle -> core.loadHostTrust(handle, hostSigner.spkiDer()) }
        val tuning = awaitUntil("the initial snapshot") { core.snapshots.value }.hostDraft.tuning
        core.configureAndCreate(
            FfiHostDraft(
                sessionName = "Trust test",
                approvalMode = mode,
                inviteCode = null,
                audioSource = FfiAudioSource(
                    sourceId = "source-1",
                    displayName = "fixture.wav",
                    sizeBytes = 4_096uL,
                    durationMs = 2_000uL,
                ),
                rememberApprovedDevices = false,
                tuning = tuning,
            ),
        )
        awaitUntil("the host to bind and advertise") {
            session.sessionId?.takeIf { session.pendingStartAdvertisingOperationId == null }
        }
    }

    private fun connect(signer: KeystoreListenerProofSigner): FfiListenerTransportHandle {
        val payload = """{"hostAddress":"$LOOPBACK","controlPort":$CONTROL_PORT,""" +
            """"syncPort":$CONTROL_PORT,"audioPort":$CONTROL_PORT,""" +
            """"sessionId":"${session.sessionId!!.value}","protocolVersion":4,""" +
            """"inviteCodeRequired":false,"expiresAtMs":null}"""
        val handle = FfiListenerTransportHandle.connectQuic(
            payload,
            System.currentTimeMillis().toULong(),
            listenerDeviceIdForKey(signer.spkiDer()),
            LOOPBACK,
            hostSigner.spkiDer(),
            signer,
        )
        listeners += handle
        handle.sendJoinRequest("Test phone", null)
        return handle
    }

    private enum class Decision { Remembered, ApprovedOnce, Rejected }

    private suspend fun awaitDecision(listener: FfiListenerTransportHandle): Decision =
        awaitUntil("the host's decision") {
            when (val event = listener.pollEvent(100uL)) {
                is FfiListenerTransportEvent.JoinApproved ->
                    if (event.trustedForFuture) Decision.Remembered else Decision.ApprovedOnce
                is FfiListenerTransportEvent.JoinRejected -> Decision.Rejected
                else -> null
            }
        }

    /** The reason the host gave when it disconnected [listener]. */
    private suspend fun awaitHostDisconnect(listener: FfiListenerTransportHandle): String =
        awaitUntil("the host to disconnect the listener") {
            (listener.pollEvent(100uL) as? FfiListenerTransportEvent.HostDisconnected)?.reason
        }

    private suspend fun awaitListenerTrust(deviceId: String, trust: FfiTrustState) {
        awaitUntil("$deviceId connected as $trust") {
            core.snapshots.value?.listeners?.firstOrNull {
                it.deviceId == deviceId && it.trustState == trust
            }
        }
    }

    private suspend fun awaitNoConnectedListener(deviceId: String) {
        awaitUntil("$deviceId to leave") {
            val listeners = core.snapshots.value?.listeners ?: return@awaitUntil null
            Unit.takeIf {
                listeners.none {
                    it.deviceId == deviceId &&
                        it.transportState == com.ekkus.silentdisco.core.uniffi.FfiTransportState.CONNECTED
                }
            }
        }
    }

    private suspend fun <T : Any> awaitUntil(what: String, probe: () -> T?): T {
        val deadline = System.currentTimeMillis() + TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
            probe()?.let { return it }
            kotlinx.coroutines.delay(50)
        }
        throw AssertionError("timed out waiting for $what; failures: $failures")
    }

    private fun deleteDatabaseFiles() {
        listOf("", "-wal", "-shm", "-journal").forEach {
            File(databaseFile.absolutePath + it).delete()
        }
    }

    private companion object {
        const val HOST_DEVICE_ID = "host-verified-trust-test"
        const val HOST_ALIAS = "silent-disco-test-host-tls"
        const val LISTENER_A_ALIAS = "silent-disco-test-listener-a"
        const val LISTENER_B_ALIAS = "silent-disco-test-listener-b"
        const val LOOPBACK = "127.0.0.1"
        const val CONTROL_PORT = 41_000
        const val TIMEOUT_MS = 10_000L
    }
}
