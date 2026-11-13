package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId

/** Host local monitor, owned by [MainViewModel.hostMonitor]. */
internal fun MainViewModel.startHostMonitorPlayback(
    sessionId: SessionId,
    streamId: StreamId,
    format: AudioFormatSpec,
    firstPacket: AudioPacket,
) = hostMonitor.start(sessionId, streamId, format, firstPacket, volume = uiState.value.localVolume)

internal fun MainViewModel.submitHostMonitorPacket(packet: AudioPacket) = hostMonitor.submit(packet)

internal fun MainViewModel.reanchorHostMonitorPlayback(hostStartTimeMs: Long) = hostMonitor.reanchor(hostStartTimeMs)

internal fun MainViewModel.stopHostMonitorPlayback() = hostMonitor.stop()
