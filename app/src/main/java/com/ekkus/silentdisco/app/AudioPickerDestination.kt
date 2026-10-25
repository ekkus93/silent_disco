package com.ekkus.silentdisco.app

import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable

/** The document picker result can outlive the Activity that launched it. */
internal enum class AudioPickerDestination {
    SETUP,
    LIVE_QUEUE,
}

@Composable
internal fun rememberAudioPickerDestination(): MutableState<AudioPickerDestination> =
    rememberSaveable { mutableStateOf(AudioPickerDestination.SETUP) }

/**
 * Consume the saved destination exactly once, including on cancellation.
 * A cancelled system picker must not erase the setup playlist or enqueue an
 * empty live batch. Reset before invoking callbacks so a failed import cannot
 * route a later result to the previous destination.
 */
internal fun dispatchAudioPickerResult(
    destination: MutableState<AudioPickerDestination>,
    uris: List<Uri>,
    onSetup: (List<Uri>) -> Unit,
    onLiveQueue: (List<Uri>) -> Unit,
) {
    val selectedDestination = destination.value
    destination.value = AudioPickerDestination.SETUP
    if (uris.isEmpty()) return
    when (selectedDestination) {
        AudioPickerDestination.SETUP -> onSetup(uris)
        AudioPickerDestination.LIVE_QUEUE -> onLiveQueue(uris)
    }
}
