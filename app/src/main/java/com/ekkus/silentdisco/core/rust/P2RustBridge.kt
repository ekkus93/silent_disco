package com.ekkus.silentdisco.core.rust

import java.util.Base64
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long

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

enum class P2SessionOutcome(val nativeCode: Int, val wireName: String) {
    COMPLETED(1, "completed"),
    CANCELLED(2, "cancelled"),
    FAILED(3, "failed"),
    ;

    companion object {
        fun fromWire(value: String): P2SessionOutcome = entries.firstOrNull { it.wireName == value }
            ?: throw P2RustProtocolException("Rust returned unknown recent-session outcome $value")
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
    /**
     * A host's manual-endpoint connection payload (host address/ports/
     * session id/protocol version), embedded verbatim -- present only for
     * hosts with a real network endpoint to hand a listener (a desktop
     * host today; Android's own peer-to-peer QR flow leaves this null and
     * keeps relying on BLE/Wi-Fi Direct discovery after verification).
     * Already structurally validated Rust-side (`ManualHostEndpoint::parse`)
     * before this ever reaches Kotlin -- safe to feed straight into
     * [ManualListenerTransportController.connect] as `rawInput` without
     * re-validating it here.
     */
    val connectionPayloadJson: String?,
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
        val value = P2RustBridge.envelopeValue(
            P2RustBridge.listRecentJson(requireOpenHandle(), nowMs, maxAgeMs, limit),
            "load recent sessions",
        ).jsonArray
        return value.mapIndexed { index, element -> parseRecent(element.jsonObject, index) }
    }

    @Synchronized
    fun validateInvitation(
        payload: String,
        nowMs: Long = System.currentTimeMillis(),
    ): P2ValidatedInvitation {
        val value = P2RustBridge.envelopeValue(
            P2RustBridge.validateQrJson(requireOpenHandle(), payload, nowMs),
            "validate QR invitation",
        ).jsonObject
        return parseInvitation(value)
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
        val value = P2RustBridge.envelopeValue(
            P2RustBridge.listTrustedJson(requireOpenHandle()),
            "load trusted hosts",
        ).jsonArray
        return value.mapIndexed { index, element -> parseTrusted(element.jsonObject, index) }
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

    private fun parseRecent(value: JsonObject, index: Int): P2RecentSession = P2RecentSession(
        sessionId = value.requiredString("sessionId", "recent session $index"),
        sessionName = value.requiredString("sessionName", "recent session $index"),
        hostName = value.requiredString("hostName", "recent session $index"),
        hostFingerprint = value.optionalString("hostFingerprint"),
        startedAtMs = value.requiredLong("startedAtMs", "recent session $index"),
        endedAtMs = value.requiredLong("endedAtMs", "recent session $index"),
        outcome = P2SessionOutcome.fromWire(
            value.requiredString("outcome", "recent session $index"),
        ),
    )

    private fun parseTrusted(value: JsonObject, index: Int): P2TrustedHost = P2TrustedHost(
        fingerprint = value.requiredString("fingerprint", "trusted host $index"),
        displayName = value.requiredString("displayName", "trusted host $index"),
        publicKeyDer = decodeStandardBase64(
            value.requiredString("publicKeyDer", "trusted host $index"),
            "trusted host public key $index",
        ),
        lastVerifiedMs = value.requiredLong("lastVerifiedMs", "trusted host $index"),
    )

    private fun parseInvitation(value: JsonObject): P2ValidatedInvitation = P2ValidatedInvitation(
        sessionId = value.requiredString("sessionId", "validated invitation"),
        sessionName = value.requiredString("sessionName", "validated invitation"),
        hostName = value.requiredString("hostName", "validated invitation"),
        hostFingerprint = value.requiredString("hostFingerprint", "validated invitation"),
        hostPublicKeyDer = decodeStandardBase64(
            value.requiredString("hostPublicKeyDer", "validated invitation"),
            "validated host public key",
        ),
        approvalMode = value.requiredString("approvalMode", "validated invitation"),
        inviteCode = value.optionalString("inviteCode"),
        issuedAtMs = value.requiredLong("issuedAtMs", "validated invitation"),
        expiresAtMs = value.requiredLong("expiresAtMs", "validated invitation"),
        connectionPayloadJson = value.optionalString("connectionPayloadJson"),
    )
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
    private external fun nativeP2ListRecentJson(
        handle: Long,
        nowMs: Long,
        maxAgeMs: Long,
        limit: Int,
    ): String
    private external fun nativeP2PrepareUnsignedQrJson(
        sessionId: String,
        sessionName: String,
        hostName: String,
        publicKeyDer: ByteArray,
        approvalMode: String,
        inviteCode: String?,
        issuedAtMs: Long,
        expiresAtMs: Long,
        nonce: String,
    ): String
    private external fun nativeP2FinalizeQrJson(unsignedJson: String, signatureBase64url: String): String
    private external fun nativeP2FingerprintJson(publicKeyDer: ByteArray): String
    private external fun nativeP2ValidateQrJson(handle: Long, payload: String, nowMs: Long): String
    private external fun nativeP2TrustValidatedHost(handle: Long, verifiedAtMs: Long): Int
    private external fun nativeP2ListTrustedJson(handle: Long): String
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
    ): String = envelopeValue(
        invokeP2Native("prepare QR invitation") {
            nativeP2PrepareUnsignedQrJson(
                sessionId,
                sessionName,
                hostName,
                publicKeyDer,
                approvalMode,
                inviteCode,
                issuedAtMs,
                expiresAtMs,
                nonce,
            )
        },
        "prepare QR invitation",
    ).jsonPrimitive.content

    fun finalizeQr(unsignedJson: String, signatureBase64url: String): String = envelopeValue(
        invokeP2Native("finalize QR invitation") {
            nativeP2FinalizeQrJson(unsignedJson, signatureBase64url)
        },
        "finalize QR invitation",
    ).jsonPrimitive.content

    fun fingerprint(publicKeyDer: ByteArray): String = envelopeValue(
        invokeP2Native("fingerprint host public key") { nativeP2FingerprintJson(publicKeyDer) },
        "fingerprint host public key",
    ).jsonPrimitive.content

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

    internal fun listRecentJson(handle: Long, nowMs: Long, maxAgeMs: Long, limit: Int): String =
        invokeP2Native("load recent sessions") {
            nativeP2ListRecentJson(handle, nowMs, maxAgeMs, limit)
        }

    internal fun validateQrJson(handle: Long, payload: String, nowMs: Long): String =
        invokeP2Native("validate QR invitation") { nativeP2ValidateQrJson(handle, payload, nowMs) }

    internal fun trustValidatedHost(handle: Long, verifiedAtMs: Long): Int =
        invokeP2Native("persist trusted host") { nativeP2TrustValidatedHost(handle, verifiedAtMs) }

    internal fun listTrustedJson(handle: Long): String = invokeP2Native("load trusted hosts") {
        nativeP2ListTrustedJson(handle)
    }

    internal fun deleteTrusted(handle: Long, fingerprint: String): Int =
        invokeP2Native("delete trusted host") { nativeP2DeleteTrusted(handle, fingerprint) }

    internal fun close(handle: Long): Int = invokeP2Native("close P2 database") {
        nativeP2Close(handle)
    }

    internal fun envelopeValue(raw: String, operation: String): JsonElement {
        val envelope = runCatching { Json.parseToJsonElement(raw).jsonObject }
            .getOrElse { throw P2RustProtocolException("Rust returned malformed JSON for $operation") }
        val status = envelope["status"]?.jsonPrimitive?.content?.toIntOrNull()
            ?: throw P2RustProtocolException("Rust omitted status for $operation")
        if (status != P2_SUCCESS) throw P2RustException(status, operation)
        return envelope["value"]
            ?: throw P2RustProtocolException("Rust omitted result value for $operation")
    }

    private inline fun <T> invokeP2Native(operation: String, call: () -> T): T = try {
        call()
    } catch (error: UnsatisfiedLinkError) {
        throw RustCoreUnavailableException(
            message = "Native Rust core library loaded but cannot $operation",
            cause = error,
        )
    }
}

private fun JsonObject.requiredString(field: String, context: String): String =
    this[field]?.jsonPrimitive?.contentOrNull?.takeIf(String::isNotBlank)
        ?: throw P2RustProtocolException("Rust omitted $field for $context")

private fun JsonObject.optionalString(field: String): String? {
    val value = this[field] ?: return null
    if (value is JsonNull) return null
    return value.jsonPrimitive.contentOrNull?.takeIf(String::isNotBlank)
}

private fun JsonObject.requiredLong(field: String, context: String): Long {
    val value = this[field]?.jsonPrimitive?.long
        ?: throw P2RustProtocolException("Rust omitted $field for $context")
    if (value < 0L) throw P2RustProtocolException("Rust returned negative $field for $context")
    return value
}

private fun decodeStandardBase64(value: String, context: String): ByteArray = runCatching {
    Base64.getDecoder().decode(value)
}.getOrElse {
    throw P2RustProtocolException("Rust returned invalid base64 for $context")
}
