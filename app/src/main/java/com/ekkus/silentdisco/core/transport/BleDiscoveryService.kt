package com.ekkus.silentdisco.core.transport

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Build
import android.os.ParcelUuid
import androidx.core.content.ContextCompat
import com.ekkus.silentdisco.core.logging.AppLogger
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SessionInfo
import java.lang.ref.WeakReference
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.util.UUID
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

data class BleOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        val Started = BleOperationResult(started = true)
        fun failed(message: String) = BleOperationResult(started = false, message = message)
    }
}

enum class BleOperation {
    ADVERTISE,
    SCAN,
}

data class BleOperationFailure(
    val operation: BleOperation,
    val message: String,
)

/**
 * Android-facing BLE discovery/advertising port used by `MainViewModel`'s
 * platform-effect runner (`startAdvertisingForRust`/`startRustListenerDiscovery`
 * in `MainViewModelRustHost.kt`/`MainViewModelRustListener.kt`).
 *
 * Exists so those effect-runner paths can be exercised in JVM unit tests
 * against a recording fake, without constructing a real [BleDiscoveryService]
 * -- which requires a real Android `Context`/`BluetoothManager` and cannot be
 * constructed in a plain JVM test (see `BleDiscoveryServiceTest.kt`).
 */
interface BleTransport {
    val discoveredSessions: StateFlow<List<SessionInfo>>
    val failures: SharedFlow<BleOperationFailure>

    fun startAdvertising(advertisement: BleAdvertisement): BleOperationResult
    fun startScanning(): BleOperationResult
    fun stop()
    fun stopAdvertising()
    fun stopScanning()
}

class BleDiscoveryService(
    context: Context,
    private val logger: AppLogger = AppLogger(),
) : BleTransport {
    private val appContext = context.applicationContext
    private val bluetoothManager = appContext.getSystemService(BluetoothManager::class.java)
    private val bluetoothAdapter: BluetoothAdapter? = bluetoothManager?.adapter
    private val advertiser: BluetoothLeAdvertiser?
        get() = bluetoothAdapter?.bluetoothLeAdvertiser
    private val scanner: BluetoothLeScanner?
        get() = bluetoothAdapter?.bluetoothLeScanner

    private val _discoveredSessions = MutableStateFlow<List<SessionInfo>>(emptyList())
    override val discoveredSessions: StateFlow<List<SessionInfo>> = _discoveredSessions.asStateFlow()
    private val _advertisement = MutableStateFlow<BleAdvertisement?>(null)
    val advertisement: StateFlow<BleAdvertisement?> = _advertisement.asStateFlow()
    private val _failures = MutableSharedFlow<BleOperationFailure>(extraBufferCapacity = 8)
    override val failures: SharedFlow<BleOperationFailure> = _failures.asSharedFlow()

    private val seenSessions = linkedMapOf<String, SessionInfo>()
    private var advertiseCallback: AdvertiseCallback? = null
    private var scanCallback: ScanCallback? = null

    init {
        activeInstance = WeakReference(this)
    }

    @SuppressLint("MissingPermission")
    override fun startAdvertising(advertisement: BleAdvertisement): BleOperationResult {
        _advertisement.value = advertisement
        stopAdvertising()
        if (!hasAdvertisePermission()) {
            val message = "Missing Bluetooth advertise permission"
            logger.w("ble.advertise", message)
            return BleOperationResult.failed(message)
        }
        val advertiser = advertiser ?: run {
            val message = "BLE advertiser unavailable on this device"
            logger.w("ble.advertise", message)
            return BleOperationResult.failed(message)
        }
        val deviceName = if (hasConnectPermission()) bluetoothAdapter?.name.orEmpty() else ""
        val serviceData = BleAdvertisementCodec.encode(advertisement, deviceName)
        val callback = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                logger.i("ble.advertise", "Advertising session ${advertisement.sessionId.take(8)}")
            }

            override fun onStartFailure(errorCode: Int) {
                emitAdvertiseFailure(errorCode)
            }
        }
        advertiseCallback = callback
        return runCatching {
            advertiser.startAdvertising(
                AdvertiseSettings.Builder()
                    .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
                    .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
                    .setConnectable(false)
                    .build(),
                AdvertiseData.Builder()
                    .addServiceUuid(BleAdvertisementCodec.serviceUuid)
                    .build(),
                AdvertiseData.Builder()
                    .addServiceData(BleAdvertisementCodec.serviceDataUuid, serviceData)
                    .build(),
                callback,
            )
        }.fold(
            onSuccess = { BleOperationResult.Started },
            onFailure = { error ->
                val message = error.message ?: "BLE advertise start failed"
                logger.w("ble.advertise", message)
                BleOperationResult.failed(message)
            },
        )
    }

    @SuppressLint("MissingPermission")
    override fun startScanning(): BleOperationResult {
        if (!hasScanPermission()) {
            val message = "Missing Bluetooth scan permission"
            logger.w("ble.scan", message)
            _discoveredSessions.value = emptyList()
            return BleOperationResult.failed(message)
        }
        val scanner = scanner ?: run {
            val message = "BLE scanner unavailable on this device"
            logger.w("ble.scan", message)
            _discoveredSessions.value = emptyList()
            return BleOperationResult.failed(message)
        }
        stopScanning()
        seenSessions.clear()
        _discoveredSessions.value = emptyList()
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val serviceData = result.scanRecord?.getServiceData(BleAdvertisementCodec.serviceDataUuid) ?: return
                val parsed = BleAdvertisementCodec.decode(payload = serviceData) ?: return
                val session = SessionInfo(
                    id = parsed.sessionId,
                    name = parsed.sessionName.ifBlank { "Silent Disco ${parsed.sessionId.take(4)}" },
                    hostDeviceName = parsed.hostName.ifBlank {
                        result.scanRecord?.deviceName ?: safeBluetoothName(result) ?: "Nearby host"
                    },
                    approvalMode = when {
                        parsed.inviteCodeRequired -> ApprovalMode.INVITE_CODE
                        parsed.approvalRequired -> ApprovalMode.MANUAL
                        else -> ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER
                    },
                    inviteCodeRequired = parsed.inviteCodeRequired,
                )
                seenSessions[session.id] = session
                _discoveredSessions.value = seenSessions.values.sortedBy { it.name }
            }

            override fun onScanFailed(errorCode: Int) {
                emitScanFailure(errorCode)
            }
        }
        scanCallback = callback
        return runCatching {
            scanner.startScan(
                listOf(
                    ScanFilter.Builder()
                        .setServiceUuid(BleAdvertisementCodec.serviceUuid)
                        .build(),
                ),
                ScanSettings.Builder()
                    .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                    .build(),
                callback,
            )
        }.fold(
            onSuccess = {
                logger.i("ble.scan", "Started BLE scanning")
                BleOperationResult.Started
            },
            onFailure = { error ->
                val message = error.message ?: "BLE scan start failed"
                logger.w("ble.scan", message)
                BleOperationResult.failed(message)
            },
        )
    }

    private fun emitAdvertiseFailure(errorCode: Int) {
        val message = "BLE advertise failed with code=$errorCode"
        logger.w("ble.advertise", message)
        _failures.tryEmit(BleOperationFailure(BleOperation.ADVERTISE, message))
    }

    private fun emitScanFailure(errorCode: Int) {
        val message = "BLE scan failed with code=$errorCode"
        logger.w("ble.scan", message)
        _discoveredSessions.value = emptyList()
        _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
    }

    internal fun emitAdvertiseFailureForTest(errorCode: Int) = emitAdvertiseFailure(errorCode)
    internal fun emitScanFailureForTest(errorCode: Int) = emitScanFailure(errorCode)

    override fun stop() {
        stopAdvertising()
        stopScanning()
        _advertisement.value = null
        seenSessions.clear()
        _discoveredSessions.value = emptyList()
    }

    @SuppressLint("MissingPermission")
    override fun stopAdvertising() {
        val callback = advertiseCallback ?: return
        if (hasAdvertisePermission()) {
            runCatching { advertiser?.stopAdvertising(callback) }
        }
        advertiseCallback = null
    }

    @SuppressLint("MissingPermission")
    override fun stopScanning() {
        val callback = scanCallback ?: return
        if (!hasScanPermission()) {
            val message = "Missing Bluetooth scan permission while stopping discovery"
            logger.w("ble.scan", message)
            _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
            return
        }
        val scanner = scanner
        if (scanner == null) {
            val message = "BLE scanner became unavailable while stopping discovery"
            logger.w("ble.scan", message)
            _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
            return
        }
        runCatching { scanner.stopScan(callback) }
            .onSuccess { scanCallback = null }
            .onFailure { error ->
                val message = error.message ?: "BLE scan could not be stopped"
                logger.w("ble.scan", message)
                _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
            }
    }

    @SuppressLint("MissingPermission")
    private fun safeBluetoothName(result: ScanResult): String? {
        if (!hasConnectPermission()) return null
        return runCatching { result.device?.name ?: result.device?.address }.getOrNull()
    }

    private fun hasScanPermission(): Boolean =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) true
        else ContextCompat.checkSelfPermission(appContext, "android.permission.BLUETOOTH_SCAN") == PackageManager.PERMISSION_GRANTED

    private fun hasAdvertisePermission(): Boolean =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) true
        else ContextCompat.checkSelfPermission(appContext, "android.permission.BLUETOOTH_ADVERTISE") == PackageManager.PERMISSION_GRANTED

    private fun hasConnectPermission(): Boolean =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) true
        else ContextCompat.checkSelfPermission(appContext, "android.permission.BLUETOOTH_CONNECT") == PackageManager.PERMISSION_GRANTED

    companion object {
        @Volatile
        private var activeInstance: WeakReference<BleDiscoveryService>? = null

        fun cancelActiveScan() {
            activeInstance?.get()?.stopScanning()
        }
    }
}

internal object BleAdvertisementCodec {
    /**
     * Used only to filter scan results ("Service UUID" advertising-data
     * structure, carried in the primary packet); unaffected by
     * [serviceDataUuid] below.
     */
    val serviceUuid: ParcelUuid = ParcelUuid(UUID.fromString("ae9b7098-6835-4a39-948a-27da5d77fd6f"))

    /**
     * A 16-bit UUID used purely as a compact "Service Data" envelope key in
     * the scan-response packet: a 128-bit UUID there costs 18 bytes of
     * fixed overhead (1-byte length + 1-byte type + 16-byte UUID) before any
     * payload, which alone exceeds the legacy 31-byte scan-response limit
     * once combined with even a short host/session name. A 16-bit UUID
     * costs only 4 bytes of overhead, leaving enough room for both names.
     * `0xFFF0` falls in the Bluetooth SIG's block reserved for
     * non-interoperable/private use (never allocated to a real product), so
     * this deliberately does not claim a real assigned UUID. A collision
     * with an unrelated device advertising the same 16-bit UUID would only
     * ever fail this codec's version/size checks below and be silently
     * ignored -- an acceptable, low-risk tradeoff since BLE here is
     * discovery/metadata assistance only, never the primary data path.
     */
    val serviceDataUuid: ParcelUuid = ParcelUuid(UUID.fromString("0000fff0-0000-1000-8000-00805f9b34fb"))

    private const val PROTOCOL_VERSION = 2
    private const val SESSION_ID_BYTES = 6
    private const val MAX_HOST_NAME_BYTES = 10
    private const val MAX_SESSION_NAME_BYTES = 6

    // version + flags + truncated session id + host-name length + session-name length, before any name bytes.
    private const val MIN_PAYLOAD_BYTES = 1 + 1 + SESSION_ID_BYTES + 1 + 1

    /**
     * `hostDeviceName` is the OS-level Bluetooth adapter name (what
     * [android.bluetooth.le.AdvertiseData.Builder.setIncludeDeviceName]
     * would have exposed), passed in explicitly and truncated to a fixed
     * size here for deterministic packet sizing -- the adapter name's
     * actual length is user/OEM-controlled and cannot be trusted to fit on
     * its own. [BleAdvertisement.hostName] is a separate, app-level
     * identifier and is not what listeners need to resolve a Wi-Fi Direct
     * peer by device name.
     */
    fun encode(advertisement: BleAdvertisement, hostDeviceName: String): ByteArray {
        val sessionUuid = runCatching { UUID.fromString(advertisement.sessionId) }.getOrElse {
            UUID.nameUUIDFromBytes(advertisement.sessionId.encodeToByteArray())
        }
        val flags = ((if (advertisement.approvalRequired) 1 else 0) or
            (if (advertisement.inviteCodeRequired) 1 shl 1 else 0)).toByte()
        val sessionIdShort = ByteBuffer.allocate(16)
            .putLong(sessionUuid.mostSignificantBits)
            .putLong(sessionUuid.leastSignificantBits)
            .array()
            .copyOfRange(0, SESSION_ID_BYTES)
        val hostNameBytes = truncateUtf8(hostDeviceName, MAX_HOST_NAME_BYTES)
        val sessionNameBytes = truncateUtf8(advertisement.sessionName, MAX_SESSION_NAME_BYTES)
        return ByteBuffer.allocate(MIN_PAYLOAD_BYTES + hostNameBytes.size + sessionNameBytes.size).apply {
            put(PROTOCOL_VERSION.toByte())
            put(flags)
            put(sessionIdShort)
            put(hostNameBytes.size.toByte())
            put(hostNameBytes)
            put(sessionNameBytes.size.toByte())
            put(sessionNameBytes)
        }.array()
    }

    /**
     * The decoded `sessionId` is a hex encoding of the truncated
     * [SESSION_ID_BYTES]-byte identifier, not a reconstructed UUID string:
     * it is only ever used as a local, opaque list/display key (see
     * `SessionInfo.id`), never for protocol correctness, so the truncation
     * here is a one-way, display-only shortening.
     */
    fun decode(payload: ByteArray): BleAdvertisement? {
        if (payload.size < MIN_PAYLOAD_BYTES) return null
        val buffer = ByteBuffer.wrap(payload)
        val version = buffer.get().toInt()
        if (version != PROTOCOL_VERSION) return null
        val flags = buffer.get().toInt()
        val sessionIdBytes = ByteArray(SESSION_ID_BYTES)
        buffer.get(sessionIdBytes)
        val sessionId = sessionIdBytes.joinToString(separator = "") { "%02x".format(it) }
        val hostNameLength = buffer.get().toInt().coerceAtLeast(0)
        if (hostNameLength > buffer.remaining()) return null
        val hostNameBytes = ByteArray(hostNameLength)
        buffer.get(hostNameBytes)
        if (buffer.remaining() < 1) return null
        val sessionNameLength = buffer.get().toInt().coerceAtLeast(0)
        val sessionNameBytes = ByteArray(minOf(sessionNameLength, buffer.remaining()))
        buffer.get(sessionNameBytes)
        return BleAdvertisement(
            sessionId = sessionId,
            sessionName = sessionNameBytes.toString(StandardCharsets.UTF_8),
            hostName = hostNameBytes.toString(StandardCharsets.UTF_8),
            approvalRequired = flags and 0x1 != 0,
            inviteCodeRequired = flags and 0x2 != 0,
        )
    }

    private fun truncateUtf8(value: String, maxBytes: Int): ByteArray {
        val bytes = value.encodeToByteArray()
        return bytes.copyOfRange(0, minOf(bytes.size, maxBytes))
    }
}
