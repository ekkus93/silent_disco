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
    // Duplicate recovery notices for the same listener/session are
    // coalesced: the in-flight native replay reads the authoritative Rust
    // snapshot at send time, so new redundant jobs cannot pile up.
    hostRecoveryDispatch.launchResync(
        viewModelScope, Dispatchers.IO, sourceHandle, event.listenerId,
    ) {
        try {
            // A retired host generation returns null. A stopped host replays
            // its terminal Stop to an admitted listener, never a retired
            // StreamStart; a newly joining listener receives no stale replay.
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
