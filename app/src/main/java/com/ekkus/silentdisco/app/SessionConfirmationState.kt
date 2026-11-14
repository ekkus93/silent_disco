package com.ekkus.silentdisco.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue

/**
 * The session dialogs (God-file refactor TODO §6.4): confirm ending the host
 * session, confirm leaving the listener session, and the host's invite
 * dialog. Saved across rotation and process death, so an open confirmation
 * stays open.
 */
internal class SessionConfirmationState(
    endSession: Boolean = false,
    leaveSession: Boolean = false,
    invite: Boolean = false,
) {
    var showEndSessionConfirmation by mutableStateOf(endSession)
    var showLeaveSessionConfirmation by mutableStateOf(leaveSession)
    var showInviteDialog by mutableStateOf(invite)

    companion object {
        val Saver = listSaver<SessionConfirmationState, Boolean>(
            save = { listOf(it.showEndSessionConfirmation, it.showLeaveSessionConfirmation, it.showInviteDialog) },
            restore = { SessionConfirmationState(it[0], it[1], it[2]) },
        )
    }
}

@Composable
internal fun rememberSessionConfirmationState(): SessionConfirmationState =
    rememberSaveable(saver = SessionConfirmationState.Saver) { SessionConfirmationState() }
