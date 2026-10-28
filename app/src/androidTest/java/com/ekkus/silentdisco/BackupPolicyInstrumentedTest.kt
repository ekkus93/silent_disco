package com.ekkus.silentdisco

import android.content.pm.ApplicationInfo
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.xmlpull.v1.XmlPullParser

/** Backup policy must never migrate device-bound trust without its Keystore key. */
class BackupPolicyInstrumentedTest {
    @Test
    fun installedManifestDisablesBackupAndTransferRulesExcludePrivateData() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val info = context.packageManager.getApplicationInfo(context.packageName, 0)
        assertThat(info.flags and ApplicationInfo.FLAG_ALLOW_BACKUP).isEqualTo(0)

        // Android 12+ has a separate device-transfer policy which may not
        // honor allowBackup=false on every vendor implementation.
        val extraction = context.resources.getXml(R.xml.data_extraction_exclusions)
        val excludedSections = mutableSetOf<String>()
        var section = ""
        while (extraction.eventType != XmlPullParser.END_DOCUMENT) {
            if (extraction.eventType == XmlPullParser.START_TAG) {
                when (extraction.name) {
                    "cloud-backup", "device-transfer" -> section = extraction.name
                    "exclude" -> if (
                        extraction.getAttributeValue(null, "domain") == "root" &&
                        extraction.getAttributeValue(null, "path") == "."
                    ) {
                        excludedSections += section
                    }
                }
            }
            extraction.next()
        }
        extraction.close()
        assertThat(excludedSections).containsExactly("cloud-backup", "device-transfer")

        val legacy = context.resources.getXml(R.xml.full_backup_exclusions)
        var legacyRootExcluded = false
        while (legacy.eventType != XmlPullParser.END_DOCUMENT) {
            if (
                legacy.eventType == XmlPullParser.START_TAG &&
                legacy.name == "exclude" &&
                legacy.getAttributeValue(null, "domain") == "root" &&
                legacy.getAttributeValue(null, "path") == "."
            ) {
                legacyRootExcluded = true
            }
            legacy.next()
        }
        legacy.close()
        assertThat(legacyRootExcluded).isTrue()
    }
}
