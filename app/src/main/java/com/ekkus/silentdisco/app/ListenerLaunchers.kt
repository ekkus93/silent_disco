package com.ekkus.silentdisco.app

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions

/** The listener's QR scanner for signed invitations, with the camera permission it needs. */
internal class QrScanner(
    private val context: Context,
    private val scanner: ManagedActivityResultLauncher<ScanOptions, ScanIntentResult>,
    private val cameraPermission: ManagedActivityResultLauncher<String, Boolean>,
    private val onRationaleNeeded: () -> Unit,
) {
    /** Opens the scanner, or asks for the rationale first when the camera is not yet allowed. */
    fun launch() {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            scanner.launch(qrScanOptions())
        } else {
            onRationaleNeeded()
        }
    }

    /** Asks for the camera after the rationale; a grant opens the scanner. */
    fun requestCameraPermission() = cameraPermission.launch(Manifest.permission.CAMERA)
}

internal fun qrScanOptions(): ScanOptions = ScanOptions().apply {
    setDesiredBarcodeFormats(ScanOptions.QR_CODE)
    setPrompt("Scan a signed Silent Disco invitation")
    setBeepEnabled(false)
    setOrientationLocked(false)
}

@Composable
internal fun rememberQrScanner(
    onScanned: (payload: String) -> Unit,
    onCancelled: () -> Unit,
    onRationaleNeeded: () -> Unit,
    onCameraDenied: () -> Unit,
): QrScanner {
    val context = LocalContext.current
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val payload = result.contents
        if (payload.isNullOrBlank()) onCancelled() else onScanned(payload)
    }
    val cameraPermission = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            scanner.launch(qrScanOptions())
        } else {
            onCameraDenied()
        }
    }
    return remember(context, scanner, cameraPermission) {
        QrScanner(context, scanner, cameraPermission, onRationaleNeeded)
    }
}
