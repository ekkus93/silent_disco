package com.ekkus.silentdisco.core.transport

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import com.ekkus.silentdisco.core.model.ApprovalMode
import java.nio.charset.StandardCharsets
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

private const val SILENT_DISCO_SERVICE_TYPE = "_silentdisco._tcp."
/**
 * Endpoint versions listed in discovery. v4 hosts are joined through their
 * QR; v3 hosts are still listed so that joining one shows why it is refused
 * (phase 4) instead of the host silently missing from the list.
 */
private val LISTED_PROTOCOL_VERSIONS = setOf(3, 4)

/** One fully-resolved Silent Disco desktop mDNS advertisement. */
data class MdnsSessionAdvertisement(
    val sessionId: String,
    val hostDeviceId: String,
    val sessionName: String,
    val approvalMode: ApprovalMode,
    val protocolVersion: Int,
    val address: String,
    val controlPort: Int,
    val syncPort: Int,
    val audioPort: Int,
)

data class MdnsOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        fun started() = MdnsOperationResult(started = true)
        fun failed(message: String) = MdnsOperationResult(started = false, message = message)
    }
}

/** Narrow discovery port so listener effect-runner tests do not require Android NSD. */
interface MdnsTransport {
    val discoveredSessions: StateFlow<List<MdnsSessionAdvertisement>>
    val failures: SharedFlow<String>

    fun startDiscovery(): MdnsOperationResult
    fun stopDiscovery()
}

/** Pure resolved-service shape used to test TXT parsing without Android framework objects. */
internal data class MdnsResolvedRecord(
    val serviceName: String,
    val address: String,
    val servicePort: Int,
    val attributes: Map<String, ByteArray>,
)

internal fun parseMdnsAdvertisement(record: MdnsResolvedRecord): MdnsSessionAdvertisement? {
    fun attribute(name: String): String? = record.attributes[name]
        ?.toString(StandardCharsets.UTF_8)
        ?.trim()
        ?.takeIf(String::isNotEmpty)

    val sessionId = attribute("sessionId") ?: return null
    val hostDeviceId = attribute("hostDeviceId") ?: return null
    val sessionName = attribute("sessionName") ?: return null
    val protocolVersion = attribute("protocolVersion")?.toIntOrNull() ?: return null
    if (protocolVersion !in LISTED_PROTOCOL_VERSIONS) return null
    val syncPort = attribute("syncPort")?.toIntOrNull()?.takeIf(::validPort) ?: return null
    val audioPort = attribute("audioPort")?.toIntOrNull()?.takeIf(::validPort) ?: return null
    if (!validPort(record.servicePort)) return null

    // The TCP SRV port is authoritative. If a TXT controlPort is also present,
    // require it to agree rather than silently choosing between conflicting
    // advertisements.
    val txtControlPort = attribute("controlPort")?.toIntOrNull()
    if (txtControlPort != null && txtControlPort != record.servicePort) return null

    val approvalMode = when (attribute("approvalMode")) {
        "manual" -> ApprovalMode.MANUAL
        "trusted_devices" -> ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
        "invite_code" -> ApprovalMode.INVITE_CODE
        else -> return null
    }
    attribute("inviteCodeRequired")?.toBooleanStrictOrNull()?.let { advertisedRequired ->
        if (advertisedRequired != (approvalMode == ApprovalMode.INVITE_CODE)) return null
    }
    val address = record.address.substringBefore('%').trim()
    if (address.isEmpty()) return null

    return MdnsSessionAdvertisement(
        sessionId = sessionId,
        hostDeviceId = hostDeviceId,
        sessionName = sessionName,
        approvalMode = approvalMode,
        protocolVersion = protocolVersion,
        address = address,
        controlPort = record.servicePort,
        syncPort = syncPort,
        audioPort = audioPort,
    )
}

private fun validPort(port: Int): Boolean = port in 1..65_535

private const val DISCOVERY_FULL_MESSAGE =
    "Nearby session list is full: additional mDNS services are ignored until some disappear. " +
        "Use the manual endpoint or QR invitation to join a host that is not listed."

/** Android NSD implementation of the desktop `_silentdisco._tcp` convenience layer. */
class MdnsDiscoveryService(context: Context) : MdnsTransport {
    private val nsdManager = context.applicationContext.getSystemService(NsdManager::class.java)
    private val _discoveredSessions = MutableStateFlow<List<MdnsSessionAdvertisement>>(emptyList())
    override val discoveredSessions: StateFlow<List<MdnsSessionAdvertisement>> = _discoveredSessions.asStateFlow()
    private val _failures = MutableSharedFlow<String>(extraBufferCapacity = 8)
    override val failures: SharedFlow<String> = _failures.asSharedFlow()

    private data class PendingResolve(val serviceInfo: NsdServiceInfo, val generation: Long)

    // Every name here is chosen by a LAN peer: bounded, with visible refusals.
    private val ledger = MdnsDiscoveryLedger<PendingResolve, MdnsSessionAdvertisement>()
    private val resolving = AtomicBoolean(false)
    private val discoveryGeneration = AtomicLong(0)
    @Volatile private var discoveryListener: NsdManager.DiscoveryListener? = null

    override fun startDiscovery(): MdnsOperationResult {
        if (discoveryListener != null) return MdnsOperationResult.started()
        val manager = nsdManager ?: return MdnsOperationResult.failed("Android NSD service is unavailable")
        discoveryGeneration.incrementAndGet()
        val listener = discoveryListener()
        discoveryListener = listener
        return try {
            manager.discoverServices(SILENT_DISCO_SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
            MdnsOperationResult.started()
        } catch (error: RuntimeException) {
            discoveryListener = null
            MdnsOperationResult.failed("mDNS discovery could not start: ${error.message ?: error::class.java.simpleName}")
        }
    }

    override fun stopDiscovery() {
        val listener = discoveryListener ?: run {
            clearSessions()
            return
        }
        discoveryListener = null
        discoveryGeneration.incrementAndGet()
        try {
            nsdManager?.stopServiceDiscovery(listener)
        } catch (error: RuntimeException) {
            _failures.tryEmit("mDNS discovery stop failed: ${error.message ?: error::class.java.simpleName}")
        } finally {
            clearSessions()
        }
    }

    private fun discoveryListener() = object : NsdManager.DiscoveryListener {
        override fun onDiscoveryStarted(serviceType: String) = Unit

        override fun onServiceFound(serviceInfo: NsdServiceInfo) {
            if (!serviceInfo.serviceType.startsWith("_silentdisco._tcp")) return
            val admission = ledger.offerPending(
                serviceInfo.serviceName,
                PendingResolve(serviceInfo, discoveryGeneration.get()),
            )
            if (admission == LedgerAdmission.REJECTED_FIRST) _failures.tryEmit(DISCOVERY_FULL_MESSAGE)
            resolveNext()
        }

        override fun onServiceLost(serviceInfo: NsdServiceInfo) {
            if (ledger.markLost(serviceInfo.serviceName)) publishSessions()
        }

        override fun onDiscoveryStopped(serviceType: String) = Unit

        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
            discoveryListener = null
            discoveryGeneration.incrementAndGet()
            _failures.tryEmit("mDNS discovery start failed with Android NSD error $errorCode")
            clearSessions()
        }

        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
            discoveryListener = null
            discoveryGeneration.incrementAndGet()
            _failures.tryEmit("mDNS discovery stop failed with Android NSD error $errorCode")
            clearSessions()
        }
    }

    private fun resolveNext() {
        if (!resolving.compareAndSet(false, true)) return
        val pending = ledger.pollPending()
        if (pending == null) {
            resolving.set(false)
            return
        }
        resolve(pending)
    }

    @Suppress("DEPRECATION")
    private fun resolve(pending: PendingResolve) {
        val serviceInfo = pending.serviceInfo
        val generation = pending.generation
        val manager = nsdManager
        if (manager == null) {
            finishResolve()
            return
        }
        try {
            manager.resolveService(
                serviceInfo,
                object : NsdManager.ResolveListener {
                    override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                        _failures.tryEmit(
                            "mDNS service ${serviceInfo.serviceName} could not be resolved (Android NSD error $errorCode)",
                        )
                        finishResolve()
                    }

                    override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                        try {
                            if (generation != discoveryGeneration.get() || discoveryListener == null) return
                            if (ledger.isLost(serviceInfo.serviceName)) return
                            val address = serviceInfo.host?.hostAddress ?: return
                            val parsed = parseMdnsAdvertisement(
                                MdnsResolvedRecord(
                                    serviceName = serviceInfo.serviceName,
                                    address = address,
                                    servicePort = serviceInfo.port,
                                    attributes = serviceInfo.attributes,
                                ),
                            ) ?: return
                            when (ledger.admitSession(serviceInfo.serviceName, parsed)) {
                                LedgerAdmission.ACCEPTED -> publishSessions()
                                LedgerAdmission.REJECTED_FIRST -> _failures.tryEmit(DISCOVERY_FULL_MESSAGE)
                                LedgerAdmission.REJECTED_REPEATED -> Unit
                            }
                        } finally {
                            finishResolve()
                        }
                    }
                },
            )
        } catch (error: RuntimeException) {
            _failures.tryEmit(
                "mDNS service ${serviceInfo.serviceName} could not be resolved: ${error.message ?: error::class.java.simpleName}",
            )
            finishResolve()
        }
    }

    private fun finishResolve() {
        resolving.set(false)
        resolveNext()
    }

    private fun clearSessions() {
        ledger.clear()
        _discoveredSessions.value = emptyList()
    }

    private fun publishSessions() {
        _discoveredSessions.value = ledger.sessionsSnapshot()
            .distinctBy { it.sessionId }
            .sortedBy { it.sessionName }
    }
}
