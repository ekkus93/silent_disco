package com.ekkus.silentdisco.core.identity

/**
 * Startup check for the Keystore keys that have no continuity check of their
 * own: the host TLS key (remembered listeners are scoped to it) and the
 * listener proof key (a listener's identity). Either one is created silently
 * when missing, so without this a Keystore reset would quietly drop a host's
 * remembered listeners or give a listener a new identity (remediation §9).
 *
 * The check never creates a key. [existingPublicKeyDer] reads the Keystore by
 * alias; JVM tests replace it.
 */
class KeyContinuityMonitor(
    private val records: Map<Kind, HostKeyRecord>,
    private val existingPublicKeyDer: (String) -> ByteArray? = { alias -> KeystoreEcKey(alias).existingPublicKeyDer() },
) {
    enum class Kind(val alias: String) {
        HOST_TLS(HostTlsIdentityManager.KEY_ALIAS),
        LISTENER_PROOF(ListenerIdentityManager.KEY_ALIAS),
    }

    /** The keys that changed or disappeared since the last check, each reported once. */
    fun check(): Map<Kind, RecordedKeyChange> =
        records.mapNotNull { (kind, record) ->
            checkRecordedKey(record, existingPublicKeyDer(kind.alias))?.let { kind to it }
        }.toMap()
}
