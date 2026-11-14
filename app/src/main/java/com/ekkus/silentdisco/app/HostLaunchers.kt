package com.ekkus.silentdisco.app

import android.net.Uri
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.remember

/** The host's song picker: the system document picker, routed to setup or the live queue. */
internal class AudioPicker(
    private val destination: MutableState<AudioPickerDestination>,
    private val launcher: ManagedActivityResultLauncher<Array<String>, List<Uri>>,
) {
    fun launch(target: AudioPickerDestination) {
        destination.value = target
        launcher.launch(arrayOf("audio/*"))
    }
}

/**
 * The system document picker may return after an Activity recreation, so the
 * destination is saved: live additions never overwrite setup selections.
 */
@Composable
internal fun rememberAudioPicker(
    onSetup: (List<Uri>) -> Unit,
    onLiveQueue: (List<Uri>) -> Unit,
): AudioPicker {
    val destination = rememberAudioPickerDestination()
    val launcher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenMultipleDocuments(),
    ) { uris: List<Uri> ->
        dispatchAudioPickerResult(
            destination = destination,
            uris = uris,
            onSetup = onSetup,
            onLiveQueue = onLiveQueue,
        )
    }
    return remember(destination, launcher) { AudioPicker(destination, launcher) }
}
