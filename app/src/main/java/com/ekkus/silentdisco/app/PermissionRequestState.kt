package com.ekkus.silentdisco.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue

/**
 * An explicit, restorable continuation for Android runtime permission results.
 * Lambdas held by a composition disappear when the Activity is recreated.
 */
internal enum class PermissionAction(val context: PermissionRequestContext) {
    HOST_SETUP(PermissionRequestContext.HOST_NEARBY),
    LISTENER_DISCOVERY(PermissionRequestContext.LISTENER_NEARBY),
    QR_LISTENER_DISCOVERY(PermissionRequestContext.LISTENER_NEARBY),
    DIRECT_QR_CONNECT(PermissionRequestContext.LISTENER_NEARBY),
    LISTENER_PERMISSION_ONLY(PermissionRequestContext.LISTENER_NEARBY),
    PICK_SETUP_AUDIO(PermissionRequestContext.AUDIO_FILE),
    PICK_LIVE_AUDIO(PermissionRequestContext.AUDIO_FILE),
}

internal class PermissionRequestState(
    pending: PermissionAction? = null,
    launched: Boolean = false,
    granted: PermissionAction? = null,
) {
    var pendingAction by mutableStateOf(pending)
        private set
    var requestLaunched by mutableStateOf(launched)
        private set
    var grantedAction by mutableStateOf(granted)
        private set

    fun request(action: PermissionAction) {
        pendingAction = action
        requestLaunched = false
        grantedAction = null
    }

    fun markLaunched() {
        if (pendingAction != null) requestLaunched = true
    }

    fun complete(granted: Boolean) {
        val action = pendingAction
        pendingAction = null
        requestLaunched = false
        if (granted) grantedAction = action
    }

    fun dismiss() {
        pendingAction = null
        requestLaunched = false
    }

    fun consumeGranted(): PermissionAction? {
        val action = grantedAction
        grantedAction = null
        return action
    }

    companion object {
        private fun actionNamed(name: String): PermissionAction? =
            PermissionAction.entries.firstOrNull { it.name == name }

        val Saver = listSaver<PermissionRequestState, String>(
            save = {
                listOf(
                    it.pendingAction?.name.orEmpty(),
                    it.requestLaunched.toString(),
                    it.grantedAction?.name.orEmpty(),
                )
            },
            restore = {
                PermissionRequestState(
                    pending = actionNamed(it[0]),
                    launched = it[1] == "true",
                    granted = actionNamed(it[2]),
                )
            },
        )
    }
}

@Composable
internal fun rememberPermissionRequestState(): PermissionRequestState =
    rememberSaveable(saver = PermissionRequestState.Saver) { PermissionRequestState() }
