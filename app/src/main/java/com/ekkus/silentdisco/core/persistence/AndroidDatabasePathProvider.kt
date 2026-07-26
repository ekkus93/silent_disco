package com.ekkus.silentdisco.core.persistence

import android.content.Context
import java.io.File

private const val DATABASE_DIRECTORY_NAME = "databases"
private const val DATABASE_FILE_NAME = "silent-disco.sqlite3"

/**
 * Selects the app-private Rust database path under Android's no-backup root.
 *
 * Kotlin creates and validates only the parent directory. It never creates,
 * opens, inspects, migrates, or repairs the database file; only Rust may do so.
 * Keeping the database under [Context.getNoBackupFilesDir] intentionally
 * excludes authoritative session, settings, and trust data from Android backup.
 */
object AndroidDatabasePathProvider {
    fun resolve(context: Context): File = resolveNoBackupPath(context.noBackupFilesDir)

    internal fun resolveNoBackupPath(noBackupRoot: File): File {
        val root = noBackupRoot.absoluteFile
        if (!root.exists() || !root.isDirectory) {
            throw AndroidDatabasePathException(
                "Android no-backup root is unavailable or is not a directory: ${root.path}",
            )
        }
        val parent = File(root, DATABASE_DIRECTORY_NAME)
        if (parent.exists()) {
            if (!parent.isDirectory) {
                throw AndroidDatabasePathException(
                    "Rust database parent exists but is not a directory: ${parent.path}",
                )
            }
        } else if (!parent.mkdirs()) {
            throw AndroidDatabasePathException(
                "Unable to create Rust database parent directory: ${parent.path}",
            )
        }
        val canonicalRoot = root.canonicalFile
        val canonicalParent = parent.canonicalFile
        if (canonicalParent.parentFile != canonicalRoot) {
            throw AndroidDatabasePathException(
                "Rust database parent escaped the Android no-backup root",
            )
        }
        return File(canonicalParent, DATABASE_FILE_NAME).absoluteFile
    }
}

class AndroidDatabasePathException(
    message: String,
    cause: Throwable? = null,
) : IllegalStateException(message, cause)
