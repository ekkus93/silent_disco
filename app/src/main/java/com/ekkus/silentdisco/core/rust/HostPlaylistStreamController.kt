package com.ekkus.silentdisco.core.rust

import com.ekkus.silentdisco.core.uniffi.FfiHostPlaylistStreamer
import com.ekkus.silentdisco.core.uniffi.FfiCoreHandle
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistFrame
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistSource
import com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart

/** Owns the Rust bounded decoder/packetizer worker for one Android host stream. */
class HostPlaylistStreamController internal constructor(
    private val handle: FfiHostPlaylistStreamer,
) : AutoCloseable {
    val streamStart: FfiPlaylistStreamStart = handle.streamStart()

    fun nextFrame(timeoutMs: ULong = DEFAULT_POLL_TIMEOUT_MS): FfiPlaylistFrame? =
        handle.nextFrame(timeoutMs)

    fun replaceUpcoming(sources: List<FfiPlaylistSource>) = handle.replaceUpcoming(sources)

    fun cancel() = handle.cancel()

    override fun close() = handle.close()

    companion object {
        fun openAuthoritative(
            sessionId: String,
            streamId: String,
            hostStartTimeMs: ULong,
            sources: List<FfiPlaylistSource>,
            authority: FfiCoreHandle,
        ): HostPlaylistStreamController = HostPlaylistStreamController(
            FfiHostPlaylistStreamer.openAuthoritative(
                sessionId, streamId, hostStartTimeMs, sources, authority,
            ),
        )

        /** Direct packetization is reserved for source validation and tests. */
        fun open(
            sessionId: String,
            streamId: String,
            hostStartTimeMs: ULong,
            sources: List<FfiPlaylistSource>,
        ): HostPlaylistStreamController = HostPlaylistStreamController(
            FfiHostPlaylistStreamer.open(sessionId, streamId, hostStartTimeMs, sources),
        )
    }
}

private const val DEFAULT_POLL_TIMEOUT_MS: ULong = 20uL
