package com.ekkus.silentdisco.core.identity

import com.google.common.truth.Truth.assertThat
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * The host TLS and listener proof keys are created silently when missing, so
 * a Keystore reset must be noticed by comparing with a recorded fingerprint
 * (remediation §9). The check itself never creates a key.
 */
class KeyContinuityMonitorTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val oldKey = byteArrayOf(1, 2, 3, 4)
    private val newKey = byteArrayOf(9, 8, 7, 6)
    private val keystore = mutableMapOf<String, ByteArray>()
    private val tls = KeyContinuityMonitor.Kind.HOST_TLS
    private val listener = KeyContinuityMonitor.Kind.LISTENER_PROOF

    private fun record(name: String) = FileHostKeyRecord(File(folder.root, name))
    private fun monitor() = KeyContinuityMonitor(
        mapOf(tls to record("tls.txt"), listener to record("listener.txt")),
    ) { alias -> keystore[alias] }

    @Test
    fun aKeyWithNoRecordIsRecordedSilently() {
        keystore[tls.alias] = oldKey
        assertThat(monitor().check()).isEmpty()
        assertThat(record("tls.txt").read()).isEqualTo(hostKeyFingerprint(oldKey))
    }

    @Test
    fun aFirstInstallWithNoKeyAndNoRecordRecordsNothing() {
        assertThat(monitor().check()).isEmpty()
        assertThat(record("tls.txt").read()).isNull()
        assertThat(record("listener.txt").read()).isNull()
    }

    @Test
    fun theSameKeyIsNotReported() {
        keystore[tls.alias] = oldKey
        monitor().check()
        assertThat(monitor().check()).isEmpty()
    }

    @Test
    fun aDifferentKeyIsReportedOnceAndBecomesTheRecord() {
        keystore[tls.alias] = oldKey
        monitor().check()
        keystore[tls.alias] = newKey

        assertThat(monitor().check()).containsExactly(
            tls,
            RecordedKeyChange(hostKeyFingerprint(oldKey), hostKeyFingerprint(newKey)),
        )
        assertThat(monitor().check()).isEmpty()
        assertThat(record("tls.txt").read()).isEqualTo(hostKeyFingerprint(newKey))
    }

    @Test
    fun aMissingKeyIsReportedOnceAndTheNextKeyIsAFirstUse() {
        keystore[listener.alias] = oldKey
        monitor().check()
        keystore.remove(listener.alias)

        assertThat(monitor().check()).containsExactly(listener, RecordedKeyChange(hostKeyFingerprint(oldKey), null))
        assertThat(record("listener.txt").read()).isNull()
        assertThat(monitor().check()).isEmpty()

        // The app creates a new key on next use; that one is recorded silently.
        keystore[listener.alias] = newKey
        assertThat(monitor().check()).isEmpty()
        assertThat(record("listener.txt").read()).isEqualTo(hostKeyFingerprint(newKey))
    }

    @Test
    fun eachKeyIsTrackedIndependently() {
        keystore[tls.alias] = oldKey
        keystore[listener.alias] = oldKey
        monitor().check()
        keystore[listener.alias] = newKey

        assertThat(monitor().check().keys).containsExactly(listener)
    }
}
