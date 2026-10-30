package com.ekkus.silentdisco.app

import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * An admitted listener can request authoritative control recovery after
 * packet loss or clock resynchronization. Never synthesize a new stream
 * timeline from mutable UI state: the Rust socket cache owns the active
 * StreamStart, pending boundaries and Pause snapshot.
 */
internal fun MainViewModel.replayHostControlsForResync(
    event: FfiHostTransportEvent.ResyncNotice,
    sourceHandle: FfiHostTransportHandle,
) {
    viewModelScope.launch(Dispatchers.IO) {
        try {
            // A retired generation returns null. Zero successful peers is
            // valid when the host is idle/Stopped: there is no playback to
            // replay, and the listener must not be resurrected.
            hostTransportController.replayCachedPlaybackForGeneration(
                expected = sourceHandle,
                listenerId = event.listenerId,
            )
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            reportRustHostCommandFailure("replay listener playback state", error)
        }
    }
}
