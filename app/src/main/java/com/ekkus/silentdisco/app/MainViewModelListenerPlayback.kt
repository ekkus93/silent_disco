package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

    /**
     * Walks the demo session through its listener progress states.
     *
     * This is a debug-only affordance for exercising the UI without a host
     * (gated on `BuildConfig.DEBUG` and the demo session id prefix). It
     * deliberately produces no audio: synthesizing packets and running them
     * through the real playback pipeline would make a fake session
     * indistinguishable from a real one in every diagnostic that matters.
     */
    internal fun MainViewModel.startListenerPlaybackSimulation(sessionId: String) {
        logger.i("listener.demo", "Simulating listener progress for demo session $sessionId")
        playbackJob?.cancel()
        playbackJob = viewModelScope.launch {
            delay(300)
            _uiState.value = _uiState.value.copy(
                listenerState = ListenerLifecycleState.PLAYING,
                listenerPlaybackState = PlaybackState.PLAYING,
                connectionProgress = _uiState.value.connectionProgress.copy(
                    currentState = ListenerLifecycleState.PLAYING,
                    connected = true,
                    approved = true,
                    synced = true,
                    buffered = true,
                    playing = true,
                ),
                lastMessage = "Demo session playing (no audio is produced)",
            )
            diagnosticsStore.updateListener {
                it.copy(playbackState = PlaybackState.PLAYING, metricsSummary = summarizeMetrics())
            }
            refreshListenerDiagnostics()
        }
        startPeriodicListenerResync()
    }

    internal fun MainViewModel.propagateListenerPlaybackState(
        playbackState: PlaybackState,
        listenerState: ListenerLifecycleState,
        message: String,
    ) {
        val isPlaying = playbackState == PlaybackState.PLAYING
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = playbackState,
            listenerState = listenerState,
            connectionProgress = _uiState.value.connectionProgress.copy(
                playing = isPlaying,
                buffered = if (!isPlaying) false else _uiState.value.connectionProgress.buffered,
            ),
            lastMessage = message,
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = playbackState,
                metricsSummary = summarizeMetrics(),
                lastError = if (listenerState == ListenerLifecycleState.DESYNCED) {
                    "Listener sync trouble detected"
                } else {
                    it.lastError
                },
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.handleListenerPlaybackEngineFailure(error: Throwable) {
        val message = error.message ?: "Playback engine failed"
        logger.e("playback.listener", message, error)
        playbackJob?.cancel()
        // Reported into Rust (rather than written locally) so a later Rust
        // snapshot cannot silently revert this back to an earlier state.
        listenerCoreController?.transportFailed(message, retryable = true)
        _uiState.value = _uiState.value.copy(
            listenerPlaybackState = PlaybackState.ERROR,
            connectionProgress = _uiState.value.connectionProgress.copy(
                buffered = false,
                playing = false,
            ),
        )
        diagnosticsStore.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }

    internal fun MainViewModel.generateSyntheticPackets(sessionId: String): List<AudioPacket> {
        val packetizer = PcmPacketizer(
            sessionId = SessionId(sessionId),
            streamId = StreamId("synthetic-stream"),
            format = AudioFormatSpec(),
        )
        return packetizer.packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = ByteArray(48_000 / 25 * 4 * 8),
                firstSampleIndex = 0,
                frameCount = 48_000 / 25 * 8,
            ),
            hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
        )
    }
