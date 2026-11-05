package com.ekkus.silentdisco.core.identity

import java.io.File
import java.security.MessageDigest

/** What a host signing key looks like compared with the last one this install recorded. */
sealed interface HostKeyContinuity {
    /** No earlier key was ever recorded; this is a genuine first use. */
    data object FirstUse : HostKeyContinuity

    /** The key matches the recorded one. */
    data object Unchanged : HostKeyContinuity

    /**
     * A key was recorded earlier but the Keystore now holds a different one or
     * none. Typical causes: restore to a new device, a Keystore reset, or an
     * app-data copy that carried the record but not the non-exportable key.
     * Nothing about the old identity may be treated as trusted.
     */
    data class Replaced(val previousFingerprint: String, val currentFingerprint: String) : HostKeyContinuity
}

/** Pure decision: compare the recorded fingerprint with the key now in use. */
fun assessHostKeyContinuity(recorded: String?, current: String): HostKeyContinuity = when {
    recorded == null -> HostKeyContinuity.FirstUse
    recorded == current -> HostKeyContinuity.Unchanged
    else -> HostKeyContinuity.Replaced(recorded, current)
}

fun hostKeyFingerprint(publicKeyDer: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(publicKeyDer).joinToString("") { "%02x".format(it) }

/** Durable record of the last host key fingerprint seen by this install. */
interface HostKeyRecord {
    fun read(): String?
    fun write(fingerprint: String)
}

/**
 * File-backed record. Keep it under `noBackupFilesDir`: backup and device
 * transfer are denied for the whole app, and a copied record without the
 * non-exportable key is exactly what this class exists to detect.
 */
class FileHostKeyRecord(private val file: File) : HostKeyRecord {
    override fun read(): String? =
        if (file.isFile) file.readText().trim().takeIf { it.isNotEmpty() } else null

    override fun write(fingerprint: String) {
        file.parentFile?.mkdirs()
        val temporary = File(file.parentFile, file.name + ".tmp")
        temporary.writeText(fingerprint)
        if (!temporary.renameTo(file)) {
            temporary.delete()
            error("Could not record the host key fingerprint at ${file.absolutePath}")
        }
    }
}
