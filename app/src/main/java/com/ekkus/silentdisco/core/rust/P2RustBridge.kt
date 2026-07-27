package com.ekkus.silentdisco.core.rust

private const val P2_SUCCESS = 0
private const val P2_NOT_FOUND = 1

class P2RustException(
    val statusCode: Int,
    operation: String,
) : IllegalStateException(
    "$operation failed in the Rust P2 core: ${p2StatusDescription(statusCode)} (status=$statusCode)",
)

class P2RustProtocolException(message: String) : IllegalStateException(message)
class P2DatabaseClosedException : IllegalStateException("Rust P2 database handle is already closed")

enum class P2SessionRole(val nativeCode: Int) {
    HOST(1),
    LISTENER(2),
}

enum class P2SessionOutcome(val nativeCode: Int) {
    COMPLETED(1),
    CANCELLED(2),
    FAILED(3),
    ;

    companion object {
        fun fromNative(code: Int): P2SessionOutcome = entries.firstOrNull { it.nativeCode == code }
            ?: throw P2RustProtocolException("Rust returned unknown recent-session outcome $code")
    }
}

data class P2RecentSession(
    val sessionId: String,
    val sessionName: String,
    val hostName: String,
    val hostFingerprint: String?,
    val startedAtMs: Long,
    val endedAtMs: Long,
    val outcome: P2SessionOutcome,
)

data class P2TrustedHost(
    val fingerprint: String,
    val displayName: String,
    val publicKeyDer: ByteArray,
    val lastVerifiedMs: Long,
)

data class P2ValidatedInvitation(
    val sessionId: String,
    val sessionName: String,
    val hostName: String,
    val hostFingerprint: String,
    val hostPublicKeyDer: ByteArray,
    val approvalMode: String,
    val inviteCode: String?,
    val issuedAtMs: Long,
    val expiresAtMs: Long,
)

private fun p2StatusDescription(status: Int): String = when (status) {
    -200 -> "invalid P2 argument"
    -201 -> "invalid or closed P2 database handle"
    -202 -> "P2 storage or integrity failure"
    -203 -> "invalid Silent Disco QR invitation"
    -204 -> "expired Silent Disco QR invitation"
    -205 -> "replayed Silent Disco QR invitation"
    -206 -> "invalid Silent Disco QR signature"
    -207 -> "native P2 result cache is unavailable"
    -208 -> "native P2 registry lock failed"
    -209 -> "JNI value conversion failed"
    else -> "unknown native P2 status"
}

private fun requireP2Success(status: Int, operation: String) {
    if (status != P2_SUCCESS) throw P2RustException(status, operation)
}

class P2Database internal constructor(nativeHandle: Long) : AutoCloseable {
    private var handle: Long = nativeHandle

    @Synchronized
    fun recordSession(
        sessionId: String,
        role: P2SessionRole,
        sessionName: String,
        hostName: String,
        hostFingerprint: String?,
        startedAtMs: Long,
        endedAtMs: Long,
        outcome: P2SessionOutcome,
    ) {
        requireP2Success(
            P2RustBridge.recordSession(
                handle = requireOpenHandle(),
                sessionId = sessionId,
                role = role.nativeCode,
                sessionName = sessionName,
                hostName = hostName,
                hostFingerprint = hostFingerprint,
                startedAtMs = startedAtMs,
                endedAtMs = endedAtMs,
                outcome = outcome.nativeCode,
            ),
            "record recent session",
        )
    }

    @Synchronized
    fun loadRecentSessions(
        nowMs: Long = System.currentTimeMillis(),
        maxAgeMs: Long = 30L * 24L * 60L * 60L * 1_000L,
        limit: Int = 10,
    ): List<P2RecentSession> {
        val current = requireOpenHandle()
        requireP2Success(
            P2RustBridge.loadRecentStatus(current, nowMs, maxAgeMs, limit),
            "load recent sessions",
        )
        val count = P2RustBridge.cachedRecentCount(current)
        if (count < 0) throw P2RustException(count, "read recent-session count")
        return List(count) { index ->
            val sessionId = P2RustBridge.cachedRecentSessionId(current, index)
                ?: throw P2RustProtocolException("Rust omitted recent-session ID at index $index")
            val sessionName = P2RustBridge.cachedRecentSessionName(current, index)
                ?: throw P2RustProtocolException("Rust omitted recent-session name at index $index")
            val hostName = P2RustBridge.cachedRecentHostName(current, index)
                ?: throw P2RustProtocolException("Rust omitted recent host name at index $index")
            val fingerprint = P2RustBridge.cachedRecentHostFingerprint(current, index)
                ?.takeIf(String::isNotBlank)
            val startedAtMs = requireNonNegativeP2(
                "recent-session start time",
                P2RustBridge.cachedRecentStartedAtMs(current, index),
            )
            val endedAtMs = requireNonNegativeP2(
                "recent-session end time",
                P2RustBridge.cachedRecentEndedAtMs(current, index),
            )
            P2RecentSession(
                sessionId = sessionId,
                sessionName = sessionName,
                hostName = hostName,
                hostFingerprint = fingerprint,
                startedAtMs = startedAtMs,
                endedAtMs = endedAtMs,
                outcome = P2SessionOutcome.fromNative(P2RustBridge.cachedRecentOutcome(current, index)),
            )
        }
    }

    @Synchronized
    fun validateInvitation(
        payload: String,
        nowMs: Long = System.currentTimeMillis(),
    ): P2ValidatedInvitation {
        val current = requireOpenHandle()
        requireP2Success(P2RustBridge.validateQr(current, payload, nowMs), "validate QR invitation")
        return P2ValidatedInvitation(
            sessionId = P2RustBridge.validatedSessionId(current)
                ?: throw P2RustProtocolException("Rust omitted validated session ID"),
            sessionName = P2RustBridge.validatedSessionName(current)
                ?: throw P2RustProtocolException("Rust omitted validated session name"),
            hostName = P2RustBridge.validatedHostName(current)
                ?: throw P2RustProtocolException("Rust omitted validated host name"),
            hostFingerprint = P2RustBridge.validatedFingerprint(current)
                ?: throw P2RustProtocolException("Rust omitted validated host fingerprint"),
            hostPublicKeyDer = P2RustBridge.validatedPublicKeyDer(current)
                ?: throw P2RustProtocolException("Rust omitted validated host public key"),
            approvalMode = P2RustBridge.validatedApprovalMode(current)
                ?: throw P2RustProtocolException("Rust omitted validated approval mode"),
            inviteCode = P2RustBridge.validatedInviteCode(current)?.takeIf(String::isNotBlank),
            issuedAtMs = requireNonNegativeP2(
                "validated invitation issue time",
                P2RustBridge.validatedIssuedAtMs(current),
            ),
            expiresAtMs = requireNonNegativeP2(
                "validated invitation expiry time",
                P2RustBridge.validatedExpiresAtMs(current),
            ),
        )
    }

    @Synchronized
    fun trustValidatedHost(verifiedAtMs: Long = System.currentTimeMillis()) {
        requireP2Success(
            P2RustBridge.trustValidatedHost(requireOpenHandle(), verifiedAtMs),
            "persist trusted host",
        )
    }

    @Synchronized
    fun loadTrustedHosts(): List<P2TrustedHost> {
        val current = requireOpenHandle()
        requireP2Success(P2RustBridge.loadTrustedStatus(current), "load trusted hosts")
        val count = P2RustBridge.cachedTrustedCount(current)
        if (count < 0) throw P2RustException(count, "read trusted-host count")
        return List(count) { index ->
            P2TrustedHost(
                fingerprint = P2RustBridge.cachedTrustedFingerprint(current, index)
                    ?: throw P2RustProtocolException("Rust omitted trusted-host fingerprint at index $index"),
                displayName = P2RustBridge.cachedTrustedDisplayName(current, index)
                    ?: throw P2RustProtocolException("Rust omitted trusted-host name at index $index"),
                publicKeyDer = P2RustBridge.cachedTrustedPublicKeyDer(current, index)
                    ?: throw P2RustProtocolException("Rust omitted trusted-host key at index $index"),
                lastVerifiedMs = requireNonNegativeP2(
                    "trusted-host verification time",
                    P2RustBridge.cachedTrustedLastVerifiedMs(current, index),
                ),
            )
        }
    }

    @Synchronized
    fun deleteTrustedHost(fingerprint: String): Boolean = when (
        val status = P2RustBridge.deleteTrusted(requireOpenHandle(), fingerprint)
    ) {
        P2_SUCCESS -> true
        P2_NOT_FOUND -> false
        else -> throw P2RustException(status, "delete trusted host")
    }

    @Synchronized
    override fun close() {
        val current = requireOpenHandle()
        requireP2Success(P2RustBridge.close(current), "close P2 database")
        handle = 0L
    }

    private fun requireOpenHandle(): Long {
        if (handle <= 0L) throw P2DatabaseClosedException()
        return handle
    }
}

object P2RustBridge {
    private external fun nativeP2Open(path: String): Long
    private external fun nativeP2Close(handle: Long): Int
    private external fun nativeP2RecordSession(
        handle: Long,
        sessionId: String,
        role: Int,
        sessionName: String,
        hostName: String,
        hostFingerprint: String?,
        startedAtMs: Long,
        endedAtMs: Long,
        outcome: Int,
    ): Int
    private external fun nativeP2LoadRecentStatus(handle: Long, nowMs: Long, maxAgeMs: Long, limit: Int): Int
    private external fun nativeP2CachedRecentCount(handle: Long): Int
    private external fun nativeP2CachedRecentSessionId(handle: Long, index: Int): String?
    private external fun nativeP2CachedRecentSessionName(handle: Long, index: Int): String?
    private external fun nativeP2CachedRecentHostName(handle: Long, index: Int): String?
    private external fun nativeP2CachedRecentHostFingerprint(handle: Long, index: Int): String?
    private external fun nativeP2CachedRecentStartedAtMs(handle: Long, index: Int): Long
    private external fun nativeP2CachedRecentEndedAtMs(handle: Long, index: Int): Long
    private external fun nativeP2CachedRecentOutcome(handle: Long, index: Int): Int
    private external fun nativeP2PrepareUnsignedQr(
        sessionId: String,
        sessionName: String,
        hostName: String,
        publicKeyDer: ByteArray,
        approvalMode: String,
        inviteCode: String?,
        issuedAtMs: Long,
        expiresAtMs: Long,
        nonce: String,
    ): String?
    private external fun nativeP2FinalizeQr(unsignedJson: String, signatureBase64url: String): String?
    private external fun nativeP2Fingerprint(publicKeyDer: ByteArray): String?
    private external fun nativeP2ValidateQr(handle: Long, payload: String, nowMs: Long): Int
    private external fun nativeP2ValidatedSessionId(handle: Long): String?
    private external fun nativeP2ValidatedSessionName(handle: Long): String?
    private external fun nativeP2ValidatedHostName(handle: Long): String?
    private external fun nativeP2ValidatedFingerprint(handle: Long): String?
    private external fun nativeP2ValidatedPublicKeyDer(handle: Long): ByteArray?
    private external fun nativeP2ValidatedApprovalMode(handle: Long): String?
    private external fun nativeP2ValidatedInviteCode(handle: Long): String?
    private external fun nativeP2ValidatedIssuedAtMs(handle: Long): Long
    private external fun nativeP2ValidatedExpiresAtMs(handle: Long): Long
    private external fun nativeP2TrustValidatedHost(handle: Long, verifiedAtMs: Long): Int
    private external fun nativeP2LoadTrustedStatus(handle: Long): Int
    private external fun nativeP2CachedTrustedCount(handle: Long): Int
    private external fun nativeP2CachedTrustedFingerprint(handle: Long, index: Int): String?
    private external fun nativeP2CachedTrustedDisplayName(handle: Long, index: Int): String?
    private external fun nativeP2CachedTrustedPublicKeyDer(handle: Long, index: Int): ByteArray?
    private external fun nativeP2CachedTrustedLastVerifiedMs(handle: Long, index: Int): Long
    private external fun nativeP2DeleteTrusted(handle: Long, fingerprint: String): Int

    fun open(path: String): P2Database {
        RustCoreBridge.requireSupportedAbiVersion()
        val handle = invokeP2Native("open the P2 database") { nativeP2Open(path) }
        if (handle <= 0L) throw P2RustException(handle.toInt(), "open P2 database")
        return P2Database(handle)
    }

    fun prepareUnsignedQr(
        sessionId: String,
        sessionName: String,
        hostName: String,
        publicKeyDer: ByteArray,
        approvalMode: String,
        inviteCode: String?,
        issuedAtMs: Long,
        expiresAtMs: Long,
        nonce: String,
    ): String = invokeP2Native("prepare QR invitation") {
        nativeP2PrepareUnsignedQr(
            sessionId,
            sessionName,
            hostName,
            publicKeyDer,
            approvalMode,
            inviteCode,
            issuedAtMs,
            expiresAtMs,
            nonce,
        ) ?: throw P2RustProtocolException("Rust did not return an unsigned QR payload")
    }

    fun finalizeQr(unsignedJson: String, signatureBase64url: String): String =
        invokeP2Native("finalize QR invitation") {
            nativeP2FinalizeQr(unsignedJson, signatureBase64url)
                ?: throw P2RustProtocolException("Rust did not return a signed QR payload")
        }

    fun fingerprint(publicKeyDer: ByteArray): String = invokeP2Native("fingerprint host public key") {
        nativeP2Fingerprint(publicKeyDer)
            ?: throw P2RustProtocolException("Rust did not return a host fingerprint")
    }

    internal fun recordSession(
        handle: Long,
        sessionId: String,
        role: Int,
        sessionName: String,
        hostName: String,
        hostFingerprint: String?,
        startedAtMs: Long,
        endedAtMs: Long,
        outcome: Int,
    ): Int = invokeP2Native("record recent session") {
        nativeP2RecordSession(
            handle,
            sessionId,
            role,
            sessionName,
            hostName,
            hostFingerprint,
            startedAtMs,
            endedAtMs,
            outcome,
        )
    }

    internal fun loadRecentStatus(handle: Long, nowMs: Long, maxAgeMs: Long, limit: Int): Int =
        invokeP2Native("load recent sessions") { nativeP2LoadRecentStatus(handle, nowMs, maxAgeMs, limit) }
    internal fun cachedRecentCount(handle: Long): Int = nativeP2CachedRecentCount(handle)
    internal fun cachedRecentSessionId(handle: Long, index: Int): String? = nativeP2CachedRecentSessionId(handle, index)
    internal fun cachedRecentSessionName(handle: Long, index: Int): String? = nativeP2CachedRecentSessionName(handle, index)
    internal fun cachedRecentHostName(handle: Long, index: Int): String? = nativeP2CachedRecentHostName(handle, index)
    internal fun cachedRecentHostFingerprint(handle: Long, index: Int): String? = nativeP2CachedRecentHostFingerprint(handle, index)
    internal fun cachedRecentStartedAtMs(handle: Long, index: Int): Long = nativeP2CachedRecentStartedAtMs(handle, index)
    internal fun cachedRecentEndedAtMs(handle: Long, index: Int): Long = nativeP2CachedRecentEndedAtMs(handle, index)
    internal fun cachedRecentOutcome(handle: Long, index: Int): Int = nativeP2CachedRecentOutcome(handle, index)
    internal fun validateQr(handle: Long, payload: String, nowMs: Long): Int =
        invokeP2Native("validate QR invitation") { nativeP2ValidateQr(handle, payload, nowMs) }
    internal fun validatedSessionId(handle: Long): String? = nativeP2ValidatedSessionId(handle)
    internal fun validatedSessionName(handle: Long): String? = nativeP2ValidatedSessionName(handle)
    internal fun validatedHostName(handle: Long): String? = nativeP2ValidatedHostName(handle)
    internal fun validatedFingerprint(handle: Long): String? = nativeP2ValidatedFingerprint(handle)
    internal fun validatedPublicKeyDer(handle: Long): ByteArray? = nativeP2ValidatedPublicKeyDer(handle)
    internal fun validatedApprovalMode(handle: Long): String? = nativeP2ValidatedApprovalMode(handle)
    internal fun validatedInviteCode(handle: Long): String? = nativeP2ValidatedInviteCode(handle)
    internal fun validatedIssuedAtMs(handle: Long): Long = nativeP2ValidatedIssuedAtMs(handle)
    internal fun validatedExpiresAtMs(handle: Long): Long = nativeP2ValidatedExpiresAtMs(handle)
    internal fun trustValidatedHost(handle: Long, verifiedAtMs: Long): Int =
        invokeP2Native("persist trusted host") { nativeP2TrustValidatedHost(handle, verifiedAtMs) }
    internal fun loadTrustedStatus(handle: Long): Int =
        invokeP2Native("load trusted hosts") { nativeP2LoadTrustedStatus(handle) }
    internal fun cachedTrustedCount(handle: Long): Int = nativeP2CachedTrustedCount(handle)
    internal fun cachedTrustedFingerprint(handle: Long, index: Int): String? = nativeP2CachedTrustedFingerprint(handle, index)
    internal fun cachedTrustedDisplayName(handle: Long, index: Int): String? = nativeP2CachedTrustedDisplayName(handle, index)
    internal fun cachedTrustedPublicKeyDer(handle: Long, index: Int): ByteArray? = nativeP2CachedTrustedPublicKeyDer(handle, index)
    internal fun cachedTrustedLastVerifiedMs(handle: Long, index: Int): Long = nativeP2CachedTrustedLastVerifiedMs(handle, index)
    internal fun deleteTrusted(handle: Long, fingerprint: String): Int =
        invokeP2Native("delete trusted host") { nativeP2DeleteTrusted(handle, fingerprint) }
    internal fun close(handle: Long): Int = invokeP2Native("close P2 database") { nativeP2Close(handle) }

    private inline fun <T> invokeP2Native(operation: String, call: () -> T): T = try {
        call()
    } catch (error: UnsatisfiedLinkError) {
        throw RustCoreUnavailableException(
            message = "Native Rust core library loaded but cannot $operation",
            cause = error,
        )
    }
}

private fun requireNonNegativeP2(field: String, value: Long): Long {
    if (value < 0L) throw P2RustProtocolException("Rust returned negative $field")
    return value
}
