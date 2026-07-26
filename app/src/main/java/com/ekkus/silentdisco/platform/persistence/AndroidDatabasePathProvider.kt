package com.ekkus.silentdisco.platform.persistence

import android.content.Context
import java.io.File

class AndroidDatabasePathException(
    message: String,
) : IllegalStateException(message)

/**
 * Selects an app-private, intentionally non-backed-up database path.
 *
 * Kotlin creates only the parent directory and never opens SQLite. Rust receives
 * the complete file path and remains the sole database owner.
 */
class AndroidDatabasePathProvider internal constructor(
    context: Context,
    private val databaseFileName: String = DEFAULT_DATABASE_FILE_NAME,
) {
    private val noBackupRoot = context.noBackupFilesDir

    fun databasePath(): String {
        require(databaseFileName.isNotBlank()) {
            "database file name must not be blank"
        }
        require(!databaseFileName.contains(File.separatorChar)) {
            "database file name must not contain path separators"
        }
        val parent = File(noBackupRoot, DOMAIN_DIRECTORY_NAME)
        if (parent.exists() && !parent.isDirectory) {
            throw AndroidDatabasePathException(
                "Rust database parent exists but is not a directory: ${parent.absolutePath}",
            )
        }
        if (!parent.exists() && !parent.mkdirs()) {
            throw AndroidDatabasePathException(
                "Unable to create Rust database parent directory: ${parent.absolutePath}",
            )
        }
        return File(parent, databaseFileName).absolutePath
    }

    companion object {
        const val DEFAULT_DATABASE_FILE_NAME = "silent-disco.sqlite3"
        private const val DOMAIN_DIRECTORY_NAME = "domain"
    }
}
