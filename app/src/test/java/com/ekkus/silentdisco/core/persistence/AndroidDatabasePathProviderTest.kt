package com.ekkus.silentdisco.core.persistence

import com.google.common.truth.Truth.assertThat
import java.nio.file.Files
import org.junit.Assert.assertThrows
import org.junit.Test

class AndroidDatabasePathProviderTest {
    @Test
    fun createsOnlyTheNoBackupParentAndReturnsTheCompleteDatabasePath() {
        val root = Files.createTempDirectory("silent-disco-no-backup")
        try {
            val database = AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())

            assertThat(database).isEqualTo(root.resolve("databases/silent-disco.sqlite3").toFile())
            assertThat(root.resolve("databases").toFile().isDirectory).isTrue()
            assertThat(database.exists()).isFalse()
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    @Test
    fun rejectsAParentPathOccupiedByARegularFile() {
        val root = Files.createTempDirectory("silent-disco-no-backup-file")
        try {
            Files.write(root.resolve("databases"), "not a directory".toByteArray())
            assertThrows(AndroidDatabasePathException::class.java) {
                AndroidDatabasePathProvider.resolveNoBackupPath(root.toFile())
            }
        } finally {
            root.toFile().deleteRecursively()
        }
    }
}
