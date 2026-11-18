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

    /** Forgets the recorded fingerprint, so the next key is a first use. */
    fun clear()
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

    override fun clear() {
        if (file.exists() && !file.delete()) {
            error("Could not clear the host key fingerprint at ${file.absolutePath}")
        }
    }
}

/**
 * A recorded Keystore key that is now different or gone. [currentFingerprint]
 * is null when the Keystore holds no key under that alias any more.
 */
data class RecordedKeyChange(val previousFingerprint: String, val currentFingerprint: String?)

/**
 * Compares the key now in the Keystore ([currentPublicKeyDer], null when
 * absent) with [record], without ever creating a key, and keeps the record in
 * step. Returns a change once: after it the record holds the new fingerprint,
 * or is cleared when the key is gone, so the next key is a first use.
 * A key with no record yet is recorded silently; no record and no key is a
 * genuine first install and records nothing.
 */
fun checkRecordedKey(record: HostKeyRecord, currentPublicKeyDer: ByteArray?): RecordedKeyChange? {
    val recorded = record.read()
    val current = currentPublicKeyDer?.let(::hostKeyFingerprint)
    return when {
        recorded == null -> {
            if (current != null) record.write(current)
            null
        }
        recorded == current -> null
        current == null -> {
            record.clear()
            RecordedKeyChange(recorded, null)
        }
        else -> {
            record.write(current)
            RecordedKeyChange(recorded, current)
        }
    }
}
