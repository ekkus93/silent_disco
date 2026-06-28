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
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.util.UUID
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
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

class BleDiscoveryService(
    context: Context,
    private val logger: AppLogger = AppLogger(),
) {
    private val appContext = context.applicationContext
    private val bluetoothManager = appContext.getSystemService(BluetoothManager::class.java)
    private val bluetoothAdapter: BluetoothAdapter? = bluetoothManager?.adapter
    private val advertiser: BluetoothLeAdvertiser?
        get() = bluetoothAdapter?.bluetoothLeAdvertiser
    private val scanner: BluetoothLeScanner?
        get() = bluetoothAdapter?.bluetoothLeScanner

    private val _discoveredSessions = MutableStateFlow<List<SessionInfo>>(emptyList())
    val discoveredSessions: StateFlow<List<SessionInfo>> = _discoveredSessions.asStateFlow()
    private val _advertisement = MutableStateFlow<BleAdvertisement?>(null)
    val advertisement: StateFlow<BleAdvertisement?> = _advertisement.asStateFlow()

    private val seenSessions = linkedMapOf<String, SessionInfo>()
    private var advertiseCallback: AdvertiseCallback? = null
    private var scanCallback: ScanCallback? = null

    @SuppressLint("MissingPermission")
    fun startAdvertising(advertisement: BleAdvertisement): BleOperationResult {
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
        val serviceData = BleAdvertisementCodec.encode(advertisement)
        val callback = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                logger.i("ble.advertise", "Advertising session ${advertisement.sessionId.take(8)}")
            }

            override fun onStartFailure(errorCode: Int) {
                logger.w("ble.advertise", "BLE advertise failed with code=$errorCode")
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
                    .setIncludeDeviceName(true)
                    .addServiceData(BleAdvertisementCodec.serviceUuid, serviceData)
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
    fun startScanning(): BleOperationResult {
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
                val serviceData = result.scanRecord?.getServiceData(BleAdvertisementCodec.serviceUuid) ?: return
                val parsed = BleAdvertisementCodec.decode(
                    payload = serviceData,
                    fallbackHostName = result.scanRecord?.deviceName ?: safeBluetoothName(result) ?: "Nearby host",
                ) ?: return
                val session = SessionInfo(
                    id = parsed.sessionId,
                    name = parsed.sessionName.ifBlank { "Silent Disco ${parsed.sessionId.take(4)}" },
                    hostDeviceName = parsed.hostName.ifBlank { safeBluetoothName(result) ?: "Nearby host" },
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
                logger.w("ble.scan", "BLE scan failed with code=$errorCode")
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

    fun stop() {
        stopAdvertising()
        stopScanning()
        _advertisement.value = null
        seenSessions.clear()
        _discoveredSessions.value = emptyList()
    }

    @SuppressLint("MissingPermission")
    private fun stopAdvertising() {
        val callback = advertiseCallback ?: return
        if (hasAdvertisePermission()) {
            runCatching { advertiser?.stopAdvertising(callback) }
        }
        advertiseCallback = null
    }

    @SuppressLint("MissingPermission")
    private fun stopScanning() {
        val callback = scanCallback ?: return
        if (hasScanPermission()) {
            runCatching { scanner?.stopScan(callback) }
        }
        scanCallback = null
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
}

internal object BleAdvertisementCodec {
    val serviceUuid: ParcelUuid = ParcelUuid(UUID.fromString("ae9b7098-6835-4a39-948a-27da5d77fd6f"))

    fun encode(advertisement: BleAdvertisement): ByteArray {
        val sessionUuid = runCatching { UUID.fromString(advertisement.sessionId) }.getOrElse {
            UUID.nameUUIDFromBytes(advertisement.sessionId.encodeToByteArray())
        }
        val flags = ((if (advertisement.approvalRequired) 1 else 0) or
            (if (advertisement.inviteCodeRequired) 1 shl 1 else 0)).toByte()
        val nameBytes = advertisement.sessionName.encodeToByteArray()
        val truncatedName = nameBytes.copyOfRange(0, minOf(nameBytes.size, 8))
        return ByteBuffer.allocate(1 + 1 + 16 + 1 + truncatedName.size).apply {
            put(1)
            put(flags)
            putLong(sessionUuid.mostSignificantBits)
            putLong(sessionUuid.leastSignificantBits)
            put(truncatedName.size.toByte())
            put(truncatedName)
        }.array()
    }

    fun decode(payload: ByteArray, fallbackHostName: String): BleAdvertisement? {
        if (payload.size < 19) return null
        val buffer = ByteBuffer.wrap(payload)
        val version = buffer.get().toInt()
        if (version != 1) return null
        val flags = buffer.get().toInt()
        val sessionId = UUID(buffer.long, buffer.long).toString()
        val nameLength = buffer.get().toInt().coerceAtLeast(0)
        val nameBytes = ByteArray(minOf(nameLength, buffer.remaining()))
        buffer.get(nameBytes)
        return BleAdvertisement(
            sessionId = sessionId,
            sessionName = nameBytes.toString(StandardCharsets.UTF_8),
            hostName = fallbackHostName,
            approvalRequired = flags and 0x1 != 0,
            inviteCodeRequired = flags and 0x2 != 0,
        )
    }
}
