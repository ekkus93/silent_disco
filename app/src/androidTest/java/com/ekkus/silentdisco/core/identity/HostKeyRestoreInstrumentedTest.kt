package com.ekkus.silentdisco.core.identity

import android.app.Application
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.app.HOST_KEY_REPLACED_MESSAGE
import com.ekkus.silentdisco.app.P2ViewModel
import com.ekkus.silentdisco.app.P2UiState
import com.ekkus.silentdisco.core.rust.P2RustBridge
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.security.KeyStore
import java.util.UUID
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Restore and upgrade simulation on the real Android Keystore and the real P2
 * database. A simulation is not cloud or physical restore proof.
 */
class HostKeyRestoreInstrumentedTest {
    private val application = ApplicationProvider.getApplicationContext<Application>()
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val suffix = UUID.randomUUID().toString()
    private val alias = "silent-disco-restore-test-$suffix"
    private val recordFile = File(application.noBackupFilesDir, "restore-$suffix.txt")

    @Before
    fun clean() {
        recordFile.delete()
    }

    @After
    fun cleanUp() {
        runCatching { keyStore.deleteEntry(alias) }
        recordFile.delete()
    }

    private fun manager() = HostIdentityManager(alias, FileHostKeyRecord(recordFile))

    private fun startViewModel(identity: HostIdentityManager): P2ViewModel {
        lateinit var viewModel: P2ViewModel
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            viewModel = P2ViewModel(application, hostIdentity = identity)
        }
        return viewModel
    }

    private fun awaitState(viewModel: P2ViewModel, condition: (P2UiState) -> Boolean): P2UiState {
        val deadline = System.currentTimeMillis() + 15_000L
        while (System.currentTimeMillis() < deadline) {
            val state = viewModel.uiState.value
            if (condition(state)) return state
            Thread.sleep(50)
        }
        return viewModel.uiState.value
    }

    @Test
    fun startupWarnsOnceWhenTheRecordedKeyIsGone() {
        manager().publicKeyDer()
        keyStore.deleteEntry(alias)

        val viewModel = startViewModel(manager())
        val state = awaitState(viewModel) { it.lastMessage == HOST_KEY_REPLACED_MESSAGE }
        assertThat(state.lastMessage).isEqualTo(HOST_KEY_REPLACED_MESSAGE)
        // A second start sees the settled new identity and stays quiet.
        val again = startViewModel(manager())
        Thread.sleep(1_500)
        assertThat(again.uiState.value.lastMessage).isNotEqualTo(HOST_KEY_REPLACED_MESSAGE)
    }

    @Test
    fun startupIsQuietWhenTheKeyIsIntact() {
        manager().publicKeyDer()
        val viewModel = startViewModel(manager())
        Thread.sleep(1_500)
        assertThat(viewModel.uiState.value.lastMessage).isNotEqualTo(HOST_KEY_REPLACED_MESSAGE)
    }

    @Test
    fun upgradeFromAnInstallWithoutARecordNeitherWarnsNorCreatesAKey() {
        // Older installs have no fingerprint record. The check must not invent
        // an identity for a listener-only user, and must not call it a restore.
        val identity = manager()
        assertThat(identity.checkContinuity()).isNull()
        assertThat(keyStore.containsAlias(alias)).isFalse()
        assertThat(recordFile.exists()).isFalse()

        // First real use records the key quietly.
        val key = identity.publicKeyDer()
        assertThat(identity.consumeKeyReplacement()).isNull()
        assertThat(FileHostKeyRecord(recordFile).read()).isEqualTo(hostKeyFingerprint(key))

        // An existing key with no record (upgrade of an already-keyed install) is adopted, not flagged.
        recordFile.delete()
        val adopted = manager()
        assertThat(adopted.publicKeyDer()).isEqualTo(key)
        assertThat(adopted.consumeKeyReplacement()).isNull()
        assertThat(FileHostKeyRecord(recordFile).read()).isEqualTo(hostKeyFingerprint(key))
    }

    @Test
    fun revokedHostIsNoLongerTrustedAndAReplacedKeyIsNotTheTrustedOne() {
        val path = File(application.noBackupFilesDir, "restore-p2-$suffix.sqlite3")
        val database = P2RustBridge.open(path.absolutePath)
        try {
            val now = System.currentTimeMillis()
            val sessionId = "550e8400-e29b-41d4-a716-446655440010"
            fun signedInvitation(identity: HostIdentityManager, nonce: String): String {
                val unsigned = P2RustBridge.prepareUnsignedQr(
                    sessionId = sessionId,
                    sessionName = "Restore Disco",
                    hostName = "Restore Host",
                    publicKeyDer = identity.publicKeyDer(),
                    approvalMode = "manual",
                    inviteCode = null,
                    issuedAtMs = now,
                    expiresAtMs = now + 300_000L,
                    nonce = nonce,
                )
                return P2RustBridge.finalizeQr(unsigned, identity.signBase64Url(unsigned.encodeToByteArray()))
            }

            val original = manager()
            val originalInvitation = database.validateInvitation(signedInvitation(original, "n1-$suffix"), now + 1_000L)
            database.trustValidatedHost(now + 1_000L)
            val trusted = database.loadTrustedHosts()
            assertThat(trusted).hasSize(1)

            // The host loses its key (restore to a new device) and signs again.
            keyStore.deleteEntry(alias)
            val reborn = manager()
            val newInvitation = database.validateInvitation(signedInvitation(reborn, "n2-$suffix"), now + 2_000L)
            assertThat(newInvitation.hostFingerprint).isNotEqualTo(originalInvitation.hostFingerprint)
            val state = P2UiState(trustedHosts = trusted, validatedInvitation = newInvitation)
            assertThat(state.validatedHostIsTrusted()).isFalse()

            // Revocation removes the old trust and reports the second delete as absent.
            assertThat(database.deleteTrustedHost(originalInvitation.hostFingerprint)).isTrue()
            assertThat(database.loadTrustedHosts()).isEmpty()
            assertThat(database.deleteTrustedHost(originalInvitation.hostFingerprint)).isFalse()
        } finally {
            database.close()
            listOf(path, File("${path.absolutePath}-wal"), File("${path.absolutePath}-shm")).forEach(File::delete)
        }
    }
}
