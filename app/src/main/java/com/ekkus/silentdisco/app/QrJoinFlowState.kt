package com.ekkus.silentdisco.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue

/**
 * The listener's signed-QR join flow (God-file refactor TODO §6.4): the
 * camera rationale and denial dialogs, the verified-invitation dialog, and a
 * join waiting for its session to appear nearby. Saved across rotation and
 * process death like each flag was on its own.
 */
internal class QrJoinFlowState(
    pendingJoin: Boolean = false,
    scanObserved: Boolean = false,
    cameraRationale: Boolean = false,
    cameraDenied: Boolean = false,
    verifiedDialog: Boolean = false,
) {
    /** A verified invitation's join is waiting for its session to be found nearby. */
    var pendingQrJoin by mutableStateOf(pendingJoin)

    /** The discovery scan started for [pendingQrJoin] has been seen running. */
    var qrJoinScanObserved by mutableStateOf(scanObserved)

    var showCameraRationale by mutableStateOf(cameraRationale)
    var showCameraDenied by mutableStateOf(cameraDenied)
    var showVerifiedQrDialog by mutableStateOf(verifiedDialog)

    companion object {
        val Saver = listSaver<QrJoinFlowState, Boolean>(
            save = {
                listOf(
                    it.pendingQrJoin,
                    it.qrJoinScanObserved,
                    it.showCameraRationale,
                    it.showCameraDenied,
                    it.showVerifiedQrDialog,
                )
            },
            restore = { QrJoinFlowState(it[0], it[1], it[2], it[3], it[4]) },
        )
    }
}

@Composable
internal fun rememberQrJoinFlowState(): QrJoinFlowState =
    rememberSaveable(saver = QrJoinFlowState.Saver) { QrJoinFlowState() }
