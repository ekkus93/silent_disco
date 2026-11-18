package com.ekkus.silentdisco.core.identity

import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.security.KeyStore
import java.util.UUID
import org.junit.After
import org.junit.Test

/**
 * Real Android Keystore, test-only aliases: the continuity check reads keys
 * without creating them, and a deleted or replaced key is reported once
 * (remediation §9).
 */
class KeyContinuityMonitorInstrumentedTest {
    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val suffix = UUID.randomUUID().toString()
    private val testAlias = KeyContinuityMonitor.Kind.entries.associateWith { "silent-disco-continuity-test-${it.name}-$suffix" }
    private val recordFiles = KeyContinuityMonitor.Kind.entries.associateWith {
        File(context.noBackupFilesDir, "continuity-test-${it.name}-$suffix.txt")
    }

    private fun monitor() = KeyContinuityMonitor(recordFiles.mapValues { FileHostKeyRecord(it.value) }) { alias ->
        val kind = KeyContinuityMonitor.Kind.entries.single { it.alias == alias }
        KeystoreEcKey(testAlias.getValue(kind)).existingPublicKeyDer()
    }

    @After
    fun cleanUp() {
        testAlias.values.forEach { if (keyStore.containsAlias(it)) keyStore.deleteEntry(it) }
        recordFiles.values.forEach { it.delete() }
    }

    @Test
    fun theCheckNeverCreatesAKey() {
        assertThat(monitor().check()).isEmpty()
        testAlias.values.forEach { assertThat(keyStore.containsAlias(it)).isFalse() }
        recordFiles.values.forEach { assertThat(it.exists()).isFalse() }
    }

    @Test
    fun aDeletedKeyIsReportedOnceThenItsReplacementIsRecorded() {
        val tls = KeyContinuityMonitor.Kind.HOST_TLS
        val original = KeystoreEcKey(testAlias.getValue(tls)).publicKeyDer()
        assertThat(monitor().check()).isEmpty()

        // A Keystore reset: the record survives, the key does not.
        keyStore.deleteEntry(testAlias.getValue(tls))
        val change = monitor().check()
        assertThat(change).containsExactly(tls, RecordedKeyChange(hostKeyFingerprint(original), null))
        assertThat(keyStore.containsAlias(testAlias.getValue(tls))).isFalse()
        assertThat(monitor().check()).isEmpty()

        val replacement = KeystoreEcKey(testAlias.getValue(tls)).publicKeyDer()
        assertThat(replacement).isNotEqualTo(original)
        assertThat(monitor().check()).isEmpty()
        assertThat(FileHostKeyRecord(recordFiles.getValue(tls)).read()).isEqualTo(hostKeyFingerprint(replacement))
    }

    @Test
    fun aKeyReplacedBetweenChecksIsReportedWithBothFingerprints() {
        val listener = KeyContinuityMonitor.Kind.LISTENER_PROOF
        val original = KeystoreEcKey(testAlias.getValue(listener)).publicKeyDer()
        monitor().check()
        keyStore.deleteEntry(testAlias.getValue(listener))
        val replacement = KeystoreEcKey(testAlias.getValue(listener)).publicKeyDer()

        assertThat(monitor().check()).containsExactly(
            listener,
            RecordedKeyChange(hostKeyFingerprint(original), hostKeyFingerprint(replacement)),
        )
    }
}
