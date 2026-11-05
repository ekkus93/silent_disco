package com.ekkus.silentdisco.core.identity

import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import java.io.File
import java.security.KeyStore
import java.util.UUID
import org.junit.Test

/** Real Android Keystore: deleting the key must surface as a replacement, not silent reuse. */
class HostIdentityContinuityInstrumentedTest {
    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()

    @Test
    fun deletedKeystoreKeyIsReportedOnceAndNewKeyBecomesTheRecordedIdentity() {
        val alias = "silent-disco-continuity-test-${UUID.randomUUID()}"
        val recordFile = File(context.noBackupFilesDir, "continuity-$alias.txt")
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        try {
            val first = HostIdentityManager(alias, FileHostKeyRecord(recordFile))
            val originalKey = first.publicKeyDer()
            assertThat(first.consumeKeyReplacement()).isNull()

            // Same alias, same key, new manager: no false alarm.
            val sameKey = HostIdentityManager(alias, FileHostKeyRecord(recordFile))
            assertThat(sameKey.publicKeyDer()).isEqualTo(originalKey)
            assertThat(sameKey.consumeKeyReplacement()).isNull()

            // Simulate a restore or reset: the record survives but the key does not.
            keyStore.deleteEntry(alias)
            val afterLoss = HostIdentityManager(alias, FileHostKeyRecord(recordFile))
            val replacementKey = afterLoss.publicKeyDer()
            assertThat(replacementKey).isNotEqualTo(originalKey)
            val change = afterLoss.consumeKeyReplacement()
            assertThat(change).isNotNull()
            assertThat(change!!.previousFingerprint).isEqualTo(hostKeyFingerprint(originalKey))
            assertThat(change.currentFingerprint).isEqualTo(hostKeyFingerprint(replacementKey))
            // Reported once, then settled.
            assertThat(afterLoss.consumeKeyReplacement()).isNull()
            assertThat(HostIdentityManager(alias, FileHostKeyRecord(recordFile)).also { it.publicKeyDer() }
                .consumeKeyReplacement()).isNull()
        } finally {
            runCatching { keyStore.deleteEntry(alias) }
            recordFile.delete()
        }
    }
}
