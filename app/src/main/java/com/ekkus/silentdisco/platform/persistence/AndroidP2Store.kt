package com.ekkus.silentdisco.platform.persistence

import android.content.Context
import com.ekkus.silentdisco.core.rust.P2Database
import com.ekkus.silentdisco.core.rust.P2RecentSession
import com.ekkus.silentdisco.core.rust.P2RustBridge
import com.ekkus.silentdisco.core.rust.P2SessionOutcome
import com.ekkus.silentdisco.core.rust.P2SessionRole
import com.ekkus.silentdisco.core.rust.P2TrustedHost
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import java.io.File
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

class AndroidP2Store internal constructor(
    context: Context,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    private val databasePath = File(
        File(context.noBackupFilesDir, "domain"),
        "silent-disco-p2.sqlite3",
    ).absolutePath
    private val mutex = Mutex()
    private var database: P2Database? = null

    suspend fun initialize(): Pair<List<P2RecentSession>, List<P2TrustedHost>> = withContext(ioDispatcher) {
        mutex.withLock {
            val current = database ?: P2RustBridge.open(databasePath).also { database = it }
            current.loadRecentSessions() to current.loadTrustedHosts()
        }
    }

    suspend fun recordSession(
        sessionId: String,
        role: P2SessionRole,
        sessionName: String,
        hostName: String,
        hostFingerprint: String?,
        startedAtMs: Long,
        endedAtMs: Long,
        outcome: P2SessionOutcome,
    ): List<P2RecentSession> = withContext(ioDispatcher) {
        mutex.withLock {
            val current = requireDatabase()
            current.recordSession(
                sessionId = sessionId,
                role = role,
                sessionName = sessionName,
                hostName = hostName,
                hostFingerprint = hostFingerprint,
                startedAtMs = startedAtMs,
                endedAtMs = endedAtMs,
                outcome = outcome,
            )
            current.loadRecentSessions()
        }
    }

    suspend fun validateInvitation(payload: String): P2ValidatedInvitation = withContext(ioDispatcher) {
        mutex.withLock { requireDatabase().validateInvitation(payload) }
    }

    suspend fun trustValidatedHost(): List<P2TrustedHost> = withContext(ioDispatcher) {
        mutex.withLock {
            val current = requireDatabase()
            current.trustValidatedHost()
            current.loadTrustedHosts()
        }
    }

    suspend fun loadTrustedHosts(): List<P2TrustedHost> = withContext(ioDispatcher) {
        mutex.withLock { requireDatabase().loadTrustedHosts() }
    }

    suspend fun deleteTrustedHost(fingerprint: String): Pair<Boolean, List<P2TrustedHost>> =
        withContext(ioDispatcher) {
            mutex.withLock {
                val current = requireDatabase()
                current.deleteTrustedHost(fingerprint) to current.loadTrustedHosts()
            }
        }

    suspend fun close() = withContext(ioDispatcher) {
        mutex.withLock {
            database?.close()
            database = null
        }
    }

    private fun requireDatabase(): P2Database = database
        ?: throw IllegalStateException("Rust P2 store has not initialized successfully")
}
