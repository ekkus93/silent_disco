package com.ekkus.silentdisco.app

/**
 * The actor's request ID, not a peer-claimed device ID, owns a pending
 * datagram route. The same legacy ID may be reused by a reconnecting TCP
 * generation or a hostile peer. A delayed approval must never borrow ports
 * or a native handle from a different request.
 *
 * This remains a fail-closed legacy-v3 adapter, not cryptographic identity:
 * native transport still needs authenticated generation-bound enrollment.
 */
internal class PendingHostJoinRoutes<H : Any>(private val capacity: Int = 128) {
    init {
        require(capacity > 0)
    }

    internal data class Route<H : Any>(
        val deviceId: String,
        val syncPort: UShort,
        val audioPort: UShort,
        val sourceHandle: H,
    )

    private val lock = Any()
    private val pending = linkedMapOf<String, Route<H>>()

    fun record(requestId: String, deviceId: String, syncPort: UShort, audioPort: UShort, handle: H): Boolean =
        synchronized(lock) {
            if (pending.containsKey(requestId) || pending.size >= capacity) {
                false
            } else {
                pending[requestId] = Route(deviceId, syncPort, audioPort, handle)
                true
            }
        }

    /** Consume once, including on an ID mismatch: never reoffer a stale route. */
    fun take(requestId: String, deviceId: String): Route<H>? = synchronized(lock) {
        pending.remove(requestId)?.takeIf { it.deviceId == deviceId }
    }

    fun discard(requestId: String, deviceId: String) {
        take(requestId, deviceId)
    }

    /** A terminal socket event invalidates every outstanding claim of that ID. */
    fun removeDevice(deviceId: String) = synchronized(lock) {
        pending.entries.removeAll { it.value.deviceId == deviceId }
    }

    fun clear() = synchronized(lock) {
        pending.clear()
    }

    internal val size: Int get() = synchronized(lock) { pending.size }
}
