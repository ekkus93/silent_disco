package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import kotlinx.coroutines.delay

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
        listenerPlaybackController.launchStatusPoll {
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
            diagnostics.updateListener {
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
        diagnostics.updateListener {
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
        listenerPlaybackController.cancelStatusPoll()
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
        diagnostics.updateListener {
            it.copy(
                playbackState = PlaybackState.ERROR,
                lastError = message,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshListenerDiagnostics()
    }
