package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState

/** Runs platform playback work only after the Rust actor confirms the requested state. */
internal suspend fun <T> runAfterAuthoritativeHostPlaybackTransition(
    target: FfiPlaybackState,
    transition: suspend (FfiPlaybackState) -> Unit,
    afterAccepted: suspend () -> T,
): T {
    transition(target)
    return afterAccepted()
}
