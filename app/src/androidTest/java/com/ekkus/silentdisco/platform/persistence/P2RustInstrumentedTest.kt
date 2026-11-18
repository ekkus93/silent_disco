package com.ekkus.silentdisco.platform.persistence

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ekkus.silentdisco.core.identity.HostIdentityManager
import com.ekkus.silentdisco.core.rust.P2RustBridge
import com.ekkus.silentdisco.core.rust.P2RustException
import com.ekkus.silentdisco.core.rust.P2SessionOutcome
import com.ekkus.silentdisco.core.rust.P2SessionRole
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.security.KeyStore
import java.util.UUID
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class P2RustInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test
    fun signedInvitationIsVerifiedTrustedAndReplayProtectedByRust() {
        val suffix = UUID.randomUUID().toString()
        val path = File(context.noBackupFilesDir, "p2-test-$suffix.sqlite3")
        val identity = HostIdentityManager("silent-disco-p2-test-$suffix")
        val database = P2RustBridge.open(path.absolutePath)
        try {
            val now = System.currentTimeMillis()
            val publicKey = identity.publicKeyDer()
            val unsigned = P2RustBridge.prepareUnsignedQr(
                sessionId = "550e8400-e29b-41d4-a716-446655440000",
                sessionName = "Instrumented Disco",
                hostName = "Instrumented Host",
                publicKeyDer = publicKey,
                approvalMode = "manual",
                inviteCode = null,
                issuedAtMs = now,
                expiresAtMs = now + 300_000L,
                nonce = suffix,
            )
            val signed = P2RustBridge.finalizeQr(
                unsignedJson = unsigned,
                signatureBase64url = identity.signBase64Url(unsigned.encodeToByteArray()),
            )

            val invitation = database.validateInvitation(signed, now + 1_000L)
            assertThat(invitation.sessionName).isEqualTo("Instrumented Disco")
            assertThat(invitation.hostPublicKeyDer).isEqualTo(publicKey)
            assertThat(invitation.hostFingerprint).isEqualTo(P2RustBridge.fingerprint(publicKey))

            database.trustValidatedHost(now + 1_000L)
            val trusted = database.loadTrustedHosts()
            assertThat(trusted).hasSize(1)
            assertThat(trusted.single().displayName).isEqualTo("Instrumented Host")
            assertThat(trusted.single().publicKeyDer).isEqualTo(publicKey)

            val replay = assertThrows(P2RustException::class.java) {
                database.validateInvitation(signed, now + 2_000L)
            }
            assertThat(replay.statusCode).isEqualTo(-205)
        } finally {
            database.close()
            deleteDatabase(path)
        }
    }

    /**
     * Remediation TODO §1 (signed invite): a Keystore-signed invitation that
     * was altered, signed by another key or expired is refused by Rust on the
     * device, and the genuine one still validates afterwards.
     */
    @Test
    fun tamperedWrongKeyAndExpiredInvitationsAreRefusedByRust() {
        val suffix = UUID.randomUUID().toString()
        val path = File(context.noBackupFilesDir, "p2-refusal-test-$suffix.sqlite3")
        val hostAlias = "silent-disco-p2-refusal-host-$suffix"
        val otherAlias = "silent-disco-p2-refusal-other-$suffix"
        val host = HostIdentityManager(hostAlias)
        val other = HostIdentityManager(otherAlias)
        val database = P2RustBridge.open(path.absolutePath)
        try {
            val now = System.currentTimeMillis()
            fun unsigned(sessionName: String) = P2RustBridge.prepareUnsignedQr(
                sessionId = "550e8400-e29b-41d4-a716-446655440002",
                sessionName = sessionName,
                hostName = "Refusal Host",
                publicKeyDer = host.publicKeyDer(),
                approvalMode = "manual",
                inviteCode = null,
                issuedAtMs = now,
                expiresAtMs = now + 300_000L,
                nonce = suffix,
            )
            val genuine = unsigned("Refusal Disco")
            val hostSignature = host.signBase64Url(genuine.encodeToByteArray())
            fun refusal(payload: String, atMs: Long) =
                assertThrows(P2RustException::class.java) { database.validateInvitation(payload, atMs) }.statusCode

            // Altered after signing: the host's signature no longer matches.
            val tampered = P2RustBridge.finalizeQr(unsigned("Someone Else's Disco"), hostSignature)
            assertThat(refusal(tampered, now + 1_000L)).isEqualTo(INVALID_SIGNATURE)

            // Signed by a key other than the one the invitation names.
            val wrongKey = P2RustBridge.finalizeQr(genuine, other.signBase64Url(genuine.encodeToByteArray()))
            assertThat(refusal(wrongKey, now + 1_000L)).isEqualTo(INVALID_SIGNATURE)

            // Past its expiry and the 60 s clock-skew grace after it.
            val signed = P2RustBridge.finalizeQr(genuine, hostSignature)
            assertThat(refusal(signed, now + 300_000L + 60_000L + 1L)).isEqualTo(EXPIRED_QR)

            // The refusals did not consume the genuine invitation.
            assertThat(database.validateInvitation(signed, now + 1_000L).sessionName).isEqualTo("Refusal Disco")
        } finally {
            database.close()
            deleteDatabase(path)
            KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.run {
                listOf(hostAlias, otherAlias).forEach { if (containsAlias(it)) deleteEntry(it) }
            }
        }
    }

    @Test
    fun terminalListenerSessionReturnsAsRecentHistory() {
        val path = File(context.noBackupFilesDir, "p2-recent-${UUID.randomUUID()}.sqlite3")
        val database = P2RustBridge.open(path.absolutePath)
        try {
            val now = System.currentTimeMillis()
            database.recordSession(
                sessionId = "550e8400-e29b-41d4-a716-446655440001",
                role = P2SessionRole.LISTENER,
                sessionName = "Recent Disco",
                hostName = "Recent Host",
                hostFingerprint = null,
                startedAtMs = now - 10_000L,
                endedAtMs = now,
                outcome = P2SessionOutcome.COMPLETED,
            )

            val recent = database.loadRecentSessions(nowMs = now + 1L)
            assertThat(recent).hasSize(1)
            assertThat(recent.single().sessionName).isEqualTo("Recent Disco")
            assertThat(recent.single().outcome).isEqualTo(P2SessionOutcome.COMPLETED)
        } finally {
            database.close()
            deleteDatabase(path)
        }
    }

    private companion object {
        // Status codes from the Rust P2 bridge (`android_p2_abi`).
        const val EXPIRED_QR = -204
        const val INVALID_SIGNATURE = -206
    }

    private fun deleteDatabase(path: File) {
        listOf(path, File("${path.absolutePath}-wal"), File("${path.absolutePath}-shm")).forEach(File::delete)
    }
}
