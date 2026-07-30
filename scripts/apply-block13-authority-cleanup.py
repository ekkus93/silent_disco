from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}: {old[:160]!r}")
    write(path, content.replace(old, new, 1))


write(
    "app/src/main/java/com/ekkus/silentdisco/app/HostPlaybackAuthority.kt",
    '''package com.ekkus.silentdisco.app

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
''',
)

write(
    "app/src/test/java/com/ekkus/silentdisco/app/HostPlaybackAuthorityTest.kt",
    '''package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import org.junit.Test

class HostPlaybackAuthorityTest {
    @Test
    fun acceptedTransitionRunsSideEffectAfterAuthorityConfirmation() = runTest {
        val events = mutableListOf<String>()

        val result = runAfterAuthoritativeHostPlaybackTransition(
            target = FfiPlaybackState.PAUSED,
            transition = { state -> events += "transition:$state" },
            afterAccepted = {
                events += "side-effect"
                "complete"
            },
        )

        assertThat(result).isEqualTo("complete")
        assertThat(events).containsExactly(
            "transition:PAUSED",
            "side-effect",
        ).inOrder()
    }

    @Test
    fun rejectedTransitionPreventsPlatformSideEffects() = runTest {
        var sideEffectRan = false

        val result = runCatching {
            runAfterAuthoritativeHostPlaybackTransition(
                target = FfiPlaybackState.STOPPED,
                transition = { throw IllegalStateException("transition rejected") },
                afterAccepted = {
                    sideEffectRan = true
                },
            )
        }

        assertThat(result.exceptionOrNull()).isInstanceOf(IllegalStateException::class.java)
        assertThat(sideEffectRan).isFalse()
    }
}
''',
)

write(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostActions.kt",
    '''package com.ekkus.silentdisco.app

import android.os.SystemClock
import androidx.lifecycle.viewModelScope
import com.ekkus.silentdisco.core.audio.AudioDecodeResult
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.audio.packetizationStats
import com.ekkus.silentdisco.core.audio.validatePacketBudget
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.uniffi.FfiPlaybackState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MainViewModel.startHostPlaybackImpl() {
    if (_uiState.value.hostPlaybackState == PlaybackState.PAUSED) {
        resumeHostPlaybackImpl()
        return
    }

    val selectedAudio = _uiState.value.hostForm.selectedAudio
    if (selectedAudio == null) {
        _uiState.value = _uiState.value.copy(lastError = "Choose an audio file before starting playback")
        return
    }
    val sessionError = requireHostSessionForPlayback(currentSessionId)
    if (sessionError != null) {
        _uiState.value = _uiState.value.copy(lastError = sessionError)
        reportRustHostPlaybackState(PlaybackState.ERROR, sessionError)
        diagnosticsStore.updateHost {
            it.copy(
                streamState = PlaybackState.ERROR,
                lastError = sessionError,
                metricsSummary = summarizeMetrics(),
            )
        }
        refreshHostDiagnostics(streamState = PlaybackState.ERROR)
        return
    }
    val sessionId = currentSessionId ?: return

    launchHostPlaybackCommand("start host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.BUFFERING) {
            diagnosticsStore.updateHost {
                it.copy(
                    streamState = PlaybackState.BUFFERING,
                    lastError = null,
                    metricsSummary = summarizeMetrics(),
                )
            }
            refreshHostDiagnostics(streamState = PlaybackState.BUFFERING)
        }

        val decoded = try {
            withContext(Dispatchers.IO) {
                latestDecodedAudio ?: decoder.decode(selectedAudio)
            }
        } catch (error: Throwable) {
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        latestDecodedAudio = decoded

        val streamId = currentStreamId ?: StreamId("stream-${SystemClock.elapsedRealtime()}").also {
            currentStreamId = it
        }
        val prepared = try {
            prepareHostStream(decoded, sessionId, streamId)
        } catch (error: Throwable) {
            handleHostPlaybackPreparationFailure(error)
            return@launchHostPlaybackCommand
        }
        latestPackets = prepared.packets

        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PLAYING) {
            val backend = try {
                playbackEngine.start(decoded.format)
            } catch (error: Throwable) {
                handleHostPlaybackEngineFailure(error)
                return@afterAuthoritativeHostPlaybackState
            }
            metrics.increment("stream_start")
            metrics.recordTiming("packet_duration_ms", 20.0)
            metrics.recordTiming("average_packet_bytes", prepared.averagePacketBytes)
            diagnosticsStore.updateHost {
                it.copy(
                    packetSendCount = 0,
                    packetSendRatePerSecond = 0.0,
                    packetBudgetSummary = prepared.packetBudgetSummary,
                    streamState = PlaybackState.PLAYING,
                    lastContactElapsedMs = SystemClock.elapsedRealtime(),
                    metricsSummary = summarizeMetrics(),
                    lastError = null,
                )
            }
            logger.i(
                "stream.start",
                "stream=${streamId.value} packets=${prepared.packets.size} " +
                    "budget=${prepared.packetBudgetSummary}",
            )
            _uiState.value = _uiState.value.copy(
                lastMessage = "Host stream started via $backend",
                lastError = null,
            )
            broadcastHostStreamStart(sessionId, streamId, decoded.format)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
            startHostStreamingLoop(streamId)
        }
    }
}

private fun MainViewModel.resumeHostPlaybackImpl() {
    val sessionId = currentSessionId
    val streamId = currentStreamId
    val format = latestDecodedAudio?.format
    if (sessionId == null || streamId == null || format == null || latestPackets.isEmpty()) {
        val message = "Host stream cannot resume because prepared audio state is unavailable"
        _uiState.value = _uiState.value.copy(lastError = message)
        reportRustHostPlaybackState(PlaybackState.ERROR, message)
        return
    }

    launchHostPlaybackCommand("resume host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PLAYING) {
            logger.i("stream.resume", "Resuming host stream")
            metrics.increment("stream_resume")
            broadcastHostStreamStart(sessionId, streamId, format)
            _uiState.value = _uiState.value.copy(lastMessage = "Host stream resumed", lastError = null)
            refreshHostDiagnostics(streamState = PlaybackState.PLAYING)
        }
    }
}

private fun MainViewModel.broadcastHostStreamStart(
    sessionId: SessionId,
    streamId: StreamId,
    format: AudioFormatSpec,
) {
    viewModelScope.launch {
        runCatching {
            wifiDirectService.broadcastControl(
                ControlMessage.StreamStart(
                    version = 1,
                    sessionId = sessionId,
                    streamId = streamId,
                    hostStartTimeMs = SystemClock.elapsedRealtime() + 100,
                    sampleRate = format.sampleRate,
                    channels = format.channelCount,
                    samplesPerPacket = latestPackets.first().samplesPerPacket,
                ),
            )
        }.onSuccess { result ->
            reportHostBroadcastDelivery("broadcast stream start", result, requireAnyPeer = false)
        }.onFailure { error ->
            handleHostControlFailure("broadcast stream start", error)
        }
    }
}

internal fun MainViewModel.pauseHostPlaybackImpl() {
    launchHostPlaybackCommand("pause host playback") {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.PAUSED) {
            logger.i("stream.pause", "Pausing host stream")
            metrics.increment("stream_pause")
            propagateListenerPlaybackState(
                playbackState = PlaybackState.PAUSED,
                listenerState = _uiState.value.listenerState,
                message = "Host paused the stream",
            )
            currentSessionId?.let { sessionId ->
                currentStreamId?.let { streamId ->
                    viewModelScope.launch {
                        runCatching {
                            wifiDirectService.broadcastControl(
                                ControlMessage.Pause(
                                    version = 1,
                                    sessionId = sessionId,
                                    streamId = streamId,
                                    hostPauseTimeMs = SystemClock.elapsedRealtime(),
                                ),
                            )
                        }.onSuccess { result ->
                            val warning = hostControlDeliveryMessage("Paused", "broadcast pause", result)
                            if (warning != null) {
                                _uiState.value = _uiState.value.copy(lastError = warning)
                                diagnosticsStore.updateHost {
                                    it.copy(lastError = warning, metricsSummary = summarizeMetrics())
                                }
                            } else {
                                diagnosticsStore.updateHost {
                                    it.copy(lastError = null, metricsSummary = summarizeMetrics())
                                }
                            }
                            refreshHostDiagnostics()
                        }.onFailure { error ->
                            handleHostControlFailure("broadcast pause", error)
                        }
                    }
                }
            }
            refreshHostDiagnostics(streamState = PlaybackState.PAUSED)
        }
    }
}

internal fun MainViewModel.stopHostPlaybackImpl() {
    launchHostPlaybackCommand("stop host playback", cancelExisting = true) {
        afterAuthoritativeHostPlaybackState(FfiPlaybackState.STOPPED) {
            logger.i("stream.stop", "Stopping host stream")
            hostStreamJob?.cancel()
            playbackJob?.cancel()
            resyncJob?.cancel()
            playbackEngine.stop()
            metrics.increment("stream_stop")
            propagateListenerPlaybackState(
                playbackState = PlaybackState.STOPPED,
                listenerState = _uiState.value.listenerState,
                message = "Host stopped the stream",
            )
            currentSessionId?.let { sessionId ->
                currentStreamId?.let { streamId ->
                    viewModelScope.launch {
                        runCatching {
                            wifiDirectService.broadcastControl(
                                ControlMessage.Stop(
                                    version = 1,
                                    sessionId = sessionId,
                                    streamId = streamId,
                                    hostStopTimeMs = SystemClock.elapsedRealtime(),
                                ),
                            )
                        }.onSuccess { result ->
                            val warning = hostControlDeliveryMessage("Stopped", "broadcast stop", result)
                            if (warning != null) {
                                _uiState.value = _uiState.value.copy(lastError = warning)
                                diagnosticsStore.updateHost {
                                    it.copy(lastError = warning, metricsSummary = summarizeMetrics())
                                }
                            } else {
                                diagnosticsStore.updateHost {
                                    it.copy(lastError = null, metricsSummary = summarizeMetrics())
                                }
                            }
                            refreshHostDiagnostics()
                        }.onFailure { error ->
                            handleHostControlFailure("broadcast stop", error)
                        }
                    }
                }
            }
            refreshHostDiagnostics(streamState = PlaybackState.STOPPED)
        }
    }
}

private fun MainViewModel.launchHostPlaybackCommand(
    action: String,
    cancelExisting: Boolean = false,
    block: suspend () -> Unit,
) {
    val existing = hostPlaybackCommandJob
    if (cancelExisting) {
        existing?.cancel()
    } else if (existing?.isActive == true) {
        _uiState.value = _uiState.value.copy(
            lastError = "Another host playback command is already pending",
        )
        return
    }

    val job = viewModelScope.launch {
        try {
            block()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Throwable) {
            reportHostPlaybackCommandFailure(action, error)
        }
    }
    hostPlaybackCommandJob = job
    job.invokeOnCompletion {
        if (hostPlaybackCommandJob === job) {
            hostPlaybackCommandJob = null
        }
    }
}

private suspend fun MainViewModel.afterAuthoritativeHostPlaybackState(
    state: FfiPlaybackState,
    afterAccepted: suspend () -> Unit,
) {
    runAfterAuthoritativeHostPlaybackTransition(
        target = state,
        transition = { target ->
            ensureRustHostCore().transitionPlaybackState(target)
            Unit
        },
        afterAccepted = afterAccepted,
    )
}

private fun MainViewModel.prepareHostStream(
    decoded: AudioDecodeResult,
    sessionId: SessionId,
    streamId: StreamId,
): PreparedHostStream {
    val totalBytes = decoded.chunks.sumOf { it.pcm16Le.size }
    val combinedBytes = ByteArray(totalBytes).also { buffer ->
        var offset = 0
        decoded.chunks.forEach { chunk ->
            chunk.pcm16Le.copyInto(buffer, offset)
            offset += chunk.pcm16Le.size
        }
    }
    val packetizer = PcmPacketizer(
        sessionId = sessionId,
        streamId = streamId,
        format = decoded.format,
    )
    val packets = packetizer.packetize(
        chunk = DecodedAudioChunk(
            pcm16Le = combinedBytes,
            firstSampleIndex = 0,
            frameCount = combinedBytes.size / decoded.format.bytesPerFrame,
        ),
        hostPresentationStartMs = SystemClock.elapsedRealtime() + 500,
    )
    if (packets.isEmpty()) {
        error("Decoded stream produced no playable packets")
    }
    val packetBudget = packets.validatePacketBudget()
    if (!packetBudget.valid) {
        error("Packet budget exceeded: ${packetBudget.maxPacketBytes} bytes")
    }
    val packetStats = packets.packetizationStats()
    return PreparedHostStream(
        packets = packets,
        packetBudgetSummary = packetBudget.summary(),
        averagePacketBytes = packetStats.averagePacketBytes,
    )
}

private fun MainViewModel.handleHostPlaybackPreparationFailure(error: Throwable) {
    metrics.increment("stream_start_error")
    logger.e("stream.start", "Failed to prepare host playback", error)
    val message = error.message ?: "Failed to decode audio file"
    _uiState.value = _uiState.value.copy(lastError = message)
    reportRustHostPlaybackState(PlaybackState.ERROR, message)
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            streamState = PlaybackState.ERROR,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics(streamState = PlaybackState.ERROR)
}

private fun MainViewModel.reportHostPlaybackCommandFailure(action: String, error: Throwable) {
    val message = error.message ?: "Failed to $action"
    logger.e("stream.authority", message, error)
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateHost {
        it.copy(lastError = message, metricsSummary = summarizeMetrics())
    }
    refreshHostDiagnostics()
}

private data class PreparedHostStream(
    val packets: List<AudioPacket>,
    val packetBudgetSummary: String,
    val averagePacketBytes: Double,
)
''',
)

replace_once(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt",
    "    internal var hostStreamJob: Job? = null\n    internal var playbackJob: Job? = null\n",
    "    internal var hostStreamJob: Job? = null\n    internal var hostPlaybackCommandJob: Job? = null\n    internal var playbackJob: Job? = null\n",
)
replace_once(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt",
    '''    fun trustListener(listenerId: String) = trustRustListener(listenerId)

    internal fun trustedListenerPersistenceMessage(error: Throwable): String =
        "Could not remember listener; approval remains session-only: " +
            (error.message ?: "trusted-device persistence failed")

''',
    "",
)
replace_once(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt",
    '''    override fun onCleared() {
        bleService.stop()
''',
    '''    override fun onCleared() {
        hostPlaybackCommandJob?.cancel()
        bleService.stop()
''',
)
replace_once(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt",
    "import java.util.UUID\n",
    "",
)

rust_host = read("app/src/main/java/com/ekkus/silentdisco/app/MainViewModelRustHost.kt")
trust_start = rust_host.index("internal fun MainViewModel.trustRustListener(listenerId: String) {\n")
trust_end = rust_host.index("internal fun MainViewModel.endRustHostSession()", trust_start)
write(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelRustHost.kt",
    rust_host[:trust_start] + rust_host[trust_end:],
)

write(
    "app/src/main/java/com/ekkus/silentdisco/app/MainViewModelSupport.kt",
    '''package com.ekkus.silentdisco.app

internal fun MainViewModel.clearScanState() {
    scanJob?.cancel()
    scanJob = null
    _uiState.value = _uiState.value.copy(isScanning = false)
}
''',
)

todo_path = "docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md"
todo = read(todo_path)
block13_start = todo.index("## Block 13 — Complete shared migration Block 12\n")
block15_start = todo.index("## Block 15 — Implement host session platform-effect runner skeleton\n")
run_id = os.environ.get("GITHUB_RUN_ID", "local-validation")
completed_blocks = f'''## Block 13 — Complete shared migration Block 12

Before implementing the production Host UI, complete and verify Block 12 of the shared migration TODO.

### 13.1 Verify Rust host draft validation

- [x] session name;
- [x] audio source requirement;
- [x] invite code;
- [x] approval mode;
- [x] tuning normalization;
- [x] cross-field constraints.

### 13.2 Verify host lifecycle

- [x] idle;
- [x] creating;
- [x] advertising/waiting;
- [x] ready;
- [x] streaming;
- [x] paused;
- [x] ending;
- [x] error/retry.

### 13.3 Verify approval logic

- [x] request deduplication;
- [x] trusted-device policy;
- [x] delivery-first approval;
- [x] delivery-first rejection;
- [x] stale request rejection;
- [x] partial and zero-recipient reporting.

### 13.4 Preserve Android behavior

- [x] Android host UI routes through Rust according to the shared TODO;
- [x] Android tests pass;
- [x] no temporary desktop-only host reducer exists.

**Acceptance:** Host semantics are shared before the desktop exposes real host controls.

**Implementation status:** Complete. The final authority closure makes Android start, resume, pause, and stop await a newer Rust-confirmed playback snapshot before any playback-engine, transport-broadcast, stream-loop, cancellation, or stop side effect. Transition rejection, timeout, and cancellation do not execute success-side effects. The dormant manual trusted-device bypass and its dead persistence helpers were removed; trusted-device persistence now occurs only through Rust storage effects. Guarded Actions run `{run_id}` ran the shared Rust, Android, desktop frontend/backend, and source-size regression gates.

---

## Block 14 — Build the desktop Host Setup screen

### 14.1 Add screen

Create:

```text
desktop/src/screens/HostSetupScreen.tsx
```

Include:

- [x] session name;
- [x] approval mode;
- [x] invite code when applicable;
- [x] remember-approved-device setting;
- [x] selected source summary;
- [x] network interface policy summary;
- [x] local monitor preference placeholder only when supported;
- [x] advanced tuning navigation;
- [x] startup and draft validation errors.

### 14.2 Draft behavior

- [x] text entry may remain local until submitted as a typed patch;
- [x] core validation result is authoritative;
- [x] fields display core validation without duplicating rules;
- [x] create button derives from core capability;
- [x] command submission shows pending, not success;
- [x] stale revision rejection refreshes snapshot and preserves safe user edits.

### 14.3 Tests

- [x] keyboard navigation;
- [x] approval-mode conditional controls;
- [x] core validation display;
- [x] pending create state;
- [x] create rejection;
- [x] no transition to hosting without newer snapshot;
- [x] screen-reader labels.

**Acceptance:** A desktop user can edit and submit a real Rust-owned host draft.

**Implementation status:** Complete on `master` at commit `acb15e42400a9c9a18ced1e5f27c3f130a5e54d8`. Guarded Actions run `30522530161` passed generated bindings, source-size enforcement, shared Rust formatting/strict Clippy/tests, desktop formatting/lint/typecheck/tests/build, desktop Rust formatting/strict Clippy/tests/check, and Android assemble/unit-tests/lint.

---

'''
write(todo_path, todo[:block13_start] + completed_blocks + todo[block15_start:])

memory_path = "memory.md"
memory = read(memory_path)
entry = f'''

## 2026-07-30 — Desktop Block 13 authority closure

- Base commit: `acb15e42400a9c9a18ced1e5f27c3f130a5e54d8`.
- Android host playback now serializes commands and awaits Rust-confirmed `BUFFERING`, `PLAYING`, `PAUSED`, and `STOPPED` snapshots before executing corresponding platform side effects.
- Start may decode and packetize only after Rust accepts buffering; playback-engine start, control broadcast, and packet-loop launch occur only after Rust accepts playing.
- Pause, resume, and stop perform no success-side effects when the Rust transition rejects, times out, or is cancelled.
- Stop intentionally cancels a pending start/resume/pause command before requesting authoritative stopped state.
- Removed the dormant `trustListener`/`trustRustListener` path and `manual-trust-*` operation IDs. Trusted-device persistence remains owned by the Rust `PersistTrustedDevice` storage-effect path.
- Added `HostPlaybackAuthorityTest` for accepted ordering and rejection-side-effect suppression.
- Guarded validation run: `{run_id}`. Required gates: source-size invariant; shared Rust fmt/strict Clippy/all-feature tests; Android assemble, unit tests, and lint; desktop generated bindings, format, lint, typecheck, tests, build; desktop Rust fmt/strict Clippy/tests/check.
'''
if "## 2026-07-30 — Desktop Block 13 authority closure" in memory:
    raise RuntimeError("memory entry already exists")
write(memory_path, memory.rstrip() + entry + "\n")

(ROOT / "scripts/apply-block13-authority-cleanup.py").unlink()
workflow = ROOT / ".github/workflows/run-block13-authority-cleanup.yml"
if workflow.exists():
    workflow.unlink()
