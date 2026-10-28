package com.ekkus.silentdisco.app

import android.content.pm.ApplicationInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.xmlpull.v1.XmlPullParser

/**
 * Tests the installed app's merged backup flag and both XML backup policies.
 * API 29 can parse the API 31+ extraction resource even though its platform
 * backup agent uses the legacy fullBackupContent rules instead.
 */
@RunWith(AndroidJUnit4::class)
class BackupExclusionInstrumentedTest {
    @Test
    fun installedManifestDisablesAutomaticBackup() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val installed = context.packageManager.getApplicationInfo(context.packageName, 0)

        assertEquals(
            "The installed/merged manifest must never enable automatic backup",
            0,
            installed.flags and ApplicationInfo.FLAG_ALLOW_BACKUP,
        )
    }

    @Test
    fun legacyAndModernRulesExcludeAllPrivateAppFiles() {
        val legacy = inspectXml(R.xml.full_backup_exclusions)
        assertEquals(setOf("full-backup-content"), legacy.rootExclusions)
        assertEquals(0, legacy.includes)

        val modern = inspectXml(R.xml.data_extraction_exclusions)
        assertEquals(
            "Cloud restore and device-to-device transfer must both exclude the app root",
            setOf("cloud-backup", "device-transfer"),
            modern.rootExclusions,
        )
        assertEquals(0, modern.includes)
    }

    private fun inspectXml(resourceId: Int): Policy {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val excluded = mutableSetOf<String>()
        var includes = 0
        var section = ""
        context.resources.getXml(resourceId).use { parser ->
            while (parser.eventType != XmlPullParser.END_DOCUMENT) {
                when (parser.eventType) {
                    XmlPullParser.START_TAG -> {
                        when (parser.name) {
                            "full-backup-content", "cloud-backup", "device-transfer" -> {
                                section = parser.name
                            }
                            "include" -> includes++
                            "exclude" -> {
                                if (
                                    parser.getAttributeValue(null, "domain") == "root" &&
                                    parser.getAttributeValue(null, "path") == "."
                                ) {
                                    assertTrue("Exclusion must be scoped to a backup section", section.isNotEmpty())
                                    excluded.add(section)
                                }
                            }
                        }
                    }
                    XmlPullParser.END_TAG -> if (parser.name == section) section = ""
                }
                parser.next()
            }
        }
        return Policy(excluded, includes)
    }

    private data class Policy(val rootExclusions: Set<String>, val includes: Int)
}
