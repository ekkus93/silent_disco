package com.ekkus.silentdisco.core.identity

import com.google.common.truth.Truth.assertThat
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * Restore simulation for the non-exportable host key. The record lives in
 * no-backup storage; a restore to a new device, a Keystore reset or a copied
 * record without its key must never be mistaken for the same identity.
 */
class HostKeyContinuityTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val oldKey = byteArrayOf(1, 2, 3, 4)
    private val newKey = byteArrayOf(9, 8, 7, 6)

    @Test
    fun firstUseIsNotReportedAsAReplacement() {
        assertThat(assessHostKeyContinuity(null, hostKeyFingerprint(oldKey)))
            .isEqualTo(HostKeyContinuity.FirstUse)
    }

    @Test
    fun theSameKeyIsUnchanged() {
        val fingerprint = hostKeyFingerprint(oldKey)
        assertThat(assessHostKeyContinuity(fingerprint, fingerprint)).isEqualTo(HostKeyContinuity.Unchanged)
    }

    @Test
    fun aDifferentKeyIsReportedAsReplacedWithBothFingerprints() {
        val outcome = assessHostKeyContinuity(hostKeyFingerprint(oldKey), hostKeyFingerprint(newKey))
        assertThat(outcome).isEqualTo(
            HostKeyContinuity.Replaced(hostKeyFingerprint(oldKey), hostKeyFingerprint(newKey)),
        )
    }

    @Test
    fun aCorruptRecordFailsSafeAsReplacedRatherThanTrustingIt() {
        val record = FileHostKeyRecord(File(folder.root, "fp.txt"))
        record.write("not-a-fingerprint")
        val outcome = assessHostKeyContinuity(record.read(), hostKeyFingerprint(oldKey))
        assertThat(outcome).isInstanceOf(HostKeyContinuity.Replaced::class.java)
    }

    @Test
    fun recordRoundTripsAndAnEmptyFileCountsAsNoRecord() {
        val file = File(folder.root, "nested/fp.txt")
        val record = FileHostKeyRecord(file)
        assertThat(record.read()).isNull()
        record.write(hostKeyFingerprint(oldKey))
        assertThat(record.read()).isEqualTo(hostKeyFingerprint(oldKey))
        record.write(hostKeyFingerprint(newKey))
        assertThat(record.read()).isEqualTo(hostKeyFingerprint(newKey))
        file.writeText("  \n")
        assertThat(record.read()).isNull()
    }

    @Test
    fun restoreToANewDeviceIdentityIsDetectedOnceAndThenSettles() {
        val record = FileHostKeyRecord(File(folder.root, "fp.txt"))
        // Original device records its key.
        assertThat(assessHostKeyContinuity(record.read(), hostKeyFingerprint(oldKey)))
            .isEqualTo(HostKeyContinuity.FirstUse)
        record.write(hostKeyFingerprint(oldKey))
        // Record carried to a new device whose Keystore holds a different key.
        val onNewDevice = assessHostKeyContinuity(record.read(), hostKeyFingerprint(newKey))
        assertThat(onNewDevice).isInstanceOf(HostKeyContinuity.Replaced::class.java)
        record.write(hostKeyFingerprint(newKey))
        // The new identity is stable from then on.
        assertThat(assessHostKeyContinuity(record.read(), hostKeyFingerprint(newKey)))
            .isEqualTo(HostKeyContinuity.Unchanged)
    }
}
