package com.ekkus.silentdisco.core.transport

internal const val MAX_PENDING_MDNS_RESOLVES = 64
internal const val MAX_DISCOVERED_MDNS_SESSIONS = 64
internal const val MAX_REMEMBERED_LOST_MDNS_NAMES = 256

/** Result of asking the ledger to take on one more peer-controlled service. */
internal enum class LedgerAdmission {
    ACCEPTED,

    /** Full. First rejection since the ledger last had room: report it once. */
    REJECTED_FIRST,

    /** Full and already reported: count it, do not report it again. */
    REJECTED_REPEATED,
}

/**
 * Finite mDNS discovery bookkeeping. Every service name stored here is chosen
 * by a peer on the local network, while NSD resolves one service at a time, so
 * an announcement flood must not be able to grow memory or the visible list
 * without bound. At a limit the newcomer is refused, never an existing entry,
 * and the refusal is counted and reported to the caller instead of vanishing.
 */
internal class MdnsDiscoveryLedger<P : Any, S : Any>(
    private val maxPending: Int = MAX_PENDING_MDNS_RESOLVES,
    private val maxSessions: Int = MAX_DISCOVERED_MDNS_SESSIONS,
    private val maxLostNames: Int = MAX_REMEMBERED_LOST_MDNS_NAMES,
) {
    init {
        require(maxPending > 0 && maxSessions > 0 && maxLostNames > 0)
    }

    private val lock = Any()
    private val pending = LinkedHashMap<String, P>()
    private val sessions = LinkedHashMap<String, S>()
    private val lostNames = LinkedHashSet<String>()
    private var overflowReported = false
    private var rejectedPending = 0L
    private var rejectedSessions = 0L

    /** Services refused because the resolve queue was full. */
    val rejectedPendingCount: Long get() = synchronized(lock) { rejectedPending }

    /** Resolved services refused because the session list was full. */
    val rejectedSessionCount: Long get() = synchronized(lock) { rejectedSessions }

    /** Queue a service for resolution. A repeat announcement keeps its place. */
    fun offerPending(name: String, item: P): LedgerAdmission = synchronized(lock) {
        lostNames.remove(name)
        if (name !in pending && pending.size >= maxPending) {
            rejectedPending++
            return@synchronized rejection()
        }
        pending[name] = item
        overflowReported = false
        LedgerAdmission.ACCEPTED
    }

    fun pollPending(): P? = synchronized(lock) {
        val first = pending.entries.firstOrNull() ?: return@synchronized null
        pending.remove(first.key)
        first.value
    }

    fun isLost(name: String): Boolean = synchronized(lock) { name in lostNames }

    /** Returns true when a published session was removed. */
    fun markLost(name: String): Boolean = synchronized(lock) {
        lostNames.add(name)
        // Forget the oldest remembered losses first; they only guard races
        // between a late resolve and its own disappearance.
        while (lostNames.size > maxLostNames) {
            lostNames.remove(lostNames.first())
        }
        pending.remove(name)
        sessions.remove(name) != null
    }

    fun admitSession(name: String, session: S): LedgerAdmission = synchronized(lock) {
        if (name !in sessions && sessions.size >= maxSessions) {
            rejectedSessions++
            return@synchronized rejection()
        }
        sessions[name] = session
        overflowReported = false
        LedgerAdmission.ACCEPTED
    }

    fun sessionsSnapshot(): List<S> = synchronized(lock) { sessions.values.toList() }

    fun clear() = synchronized(lock) {
        pending.clear()
        sessions.clear()
        lostNames.clear()
        overflowReported = false
    }

    private fun rejection(): LedgerAdmission =
        if (overflowReported) {
            LedgerAdmission.REJECTED_REPEATED
        } else {
            overflowReported = true
            LedgerAdmission.REJECTED_FIRST
        }
}
