# replies1.md — Silent Disco Code Review 2 Clarifications

These are the implementation clarifications for Claude Code after reviewing `responses1.md`.

The general rule for this pass is:

- Prefer explicit state over inferred state.
- Do not add fake-success fallbacks.
- Do not silently return from user-triggered actions.
- Do not expand scope into platform/library upgrades unless required to make the reviewed fixes correct.
- Add unit tests for state-machine behavior before or alongside the implementation.

---

## 1. Scan window timing (P0.4)

**Decision: expose the scan window through `TuningSettings`, with a default of `3_000L`.**

Do not leave this as an unexplained hard-coded value inside `scanForSessions()`. The app already has persisted tuning controls for sync/buffer behavior, and scan-window tuning is useful during real-device testing.

Use a bounded value so a bad setting cannot make the UI hang for a long time.

Recommended behavior:

- Add `scanWindowMs: Long = 3_000L` to `TuningSettings`.
- Enforce a reasonable range when accepting persisted/user-edited values.
- Suggested range: `1_000L..10_000L`.
- `scanForSessions()` should use the current tuning setting.
- The UI should still guarantee that `isScanning` is cleared on completion, timeout, cancellation, and error.
- The scan timeout must be implemented as state-machine behavior, not just a best-effort coroutine delay with no cleanup.

Suggested helper:

```kotlin
private const val DEFAULT_SCAN_WINDOW_MS = 3_000L
private const val MIN_SCAN_WINDOW_MS = 1_000L
private const val MAX_SCAN_WINDOW_MS = 10_000L

data class TuningSettings(
    // existing fields...
    val scanWindowMs: Long = DEFAULT_SCAN_WINDOW_MS,
) {
    fun normalized(): TuningSettings =
        copy(scanWindowMs = scanWindowMs.coerceIn(MIN_SCAN_WINDOW_MS, MAX_SCAN_WINDOW_MS))
}
```

In `scanForSessions()`:

```kotlin
fun scanForSessions() {
    val scanWindowMs = _uiState.value.tuningSettings.normalized().scanWindowMs

    scanJob?.cancel()
    scanJob = viewModelScope.launch {
        _uiState.update {
            it.copy(
                isScanning = true,
                listenerState = ListenerLifecycleState.SCANNING,
                lastError = null,
            )
        }

        try {
            discoveryService.startScan()
            delay(scanWindowMs)
            refreshDiscoveredSessions()
            _uiState.update {
                it.copy(
                    isScanning = false,
                    listenerState = if (it.selectedSession == null) {
                        ListenerLifecycleState.IDLE
                    } else {
                        ListenerLifecycleState.SESSION_SELECTED
                    },
                )
            }
        } catch (t: Throwable) {
            _uiState.update {
                it.copy(
                    isScanning = false,
                    listenerState = ListenerLifecycleState.ERROR,
                    lastError = "Scan failed: ${t.message ?: t::class.simpleName}",
                )
            }
        } finally {
            runCatching { discoveryService.stopScan() }
        }
    }
}
```

Adjust the exact service method names to match the repo, but preserve the semantics: scan starts visibly, ends visibly, and never leaves the UI stuck in `SCANNING`.

---

## 2. Playback error recovery mid-stream (P1.4)

**Decision: distinguish recoverable buffering from fatal playback errors.**

Do not treat every mid-stream problem the same.

### Recoverable case: underrun / insufficient buffered audio

If packets stop briefly, buffer depth drops below the playback threshold, or the listener needs to re-buffer while the transport is still alive:

- Set `listenerState = ListenerLifecycleState.BUFFERING`.
- Set `connectionProgress.buffering = true`.
- Set `connectionProgress.playing = false`.
- Keep earlier steps true: discovered, requested, approved, connected, synced.
- Do not clear the selected session.
- Auto-return to `PLAYING` only after the buffer threshold is met again.

Suggested transition:

```kotlin
private fun onPlaybackUnderrun() {
    _uiState.update {
        it.copy(
            listenerState = ListenerLifecycleState.BUFFERING,
            listenerPlaybackState = PlaybackState.BUFFERING,
            connectionProgress = it.connectionProgress.copy(
                discovered = true,
                requested = true,
                approved = true,
                connected = true,
                synced = true,
                buffering = true,
                playing = false,
            ),
            lastError = null,
        )
    }
}
```

### Fatal case: playback engine failure

If `AudioTrack` cannot initialize, write returns a hard error, the playback engine is stopped unexpectedly, or an exception indicates audio output is not usable:

- Set `listenerState = ListenerLifecycleState.ERROR`.
- Set `listenerPlaybackState = PlaybackState.STOPPED` or `PlaybackState.ERROR` if that enum exists.
- Set `connectionProgress.playing = false`.
- Set `connectionProgress.buffering = false`.
- Set a visible `lastError`.
- Do not silently fall back to fake playback.
- Require explicit Retry/Rejoin/Leave action.

Suggested transition:

```kotlin
private fun onFatalPlaybackError(t: Throwable) {
    _uiState.update {
        it.copy(
            listenerState = ListenerLifecycleState.ERROR,
            listenerPlaybackState = PlaybackState.STOPPED,
            connectionProgress = it.connectionProgress.copy(
                buffering = false,
                playing = false,
            ),
            lastError = "Playback failed: ${t.message ?: t::class.simpleName}",
        )
    }
}
```

If the app does not already have `PlaybackState.ERROR`, do not add it just for this pass unless doing so is low-risk. `ListenerLifecycleState.ERROR + lastError` is enough for this pass.

---

## 3. Host startup validation consolidation (P1.6-P1.7)

**Decision: extract host form validation into one private helper in `MainViewModel`.**

Do not leave validation scattered through `createHostSession()`. The pass is specifically trying to avoid partial side effects before validation completes. A helper makes that intent harder to break later.

Recommended helper:

```kotlin
private fun validateHostForm(state: AppUiState): String? {
    val form = state.hostForm

    if (form.sessionName.isBlank()) {
        return "Enter a session name before hosting."
    }

    if (form.selectedAudio == null) {
        return "Choose an audio file before hosting."
    }

    if (
        form.approvalMode == ApprovalMode.INVITE_CODE &&
        form.inviteCode.isBlank()
    ) {
        return "Enter an invite code or choose a different approval mode."
    }

    return null
}
```

Recommended `createHostSession()` shape:

```kotlin
fun createHostSession() {
    val validationError = validateHostForm(_uiState.value)
    if (validationError != null) {
        _uiState.update {
            it.copy(
                hostState = HostLifecycleState.ERROR,
                lastError = validationError,
            )
        }
        return
    }

    _uiState.update {
        it.copy(
            hostState = HostLifecycleState.CREATING_SESSION,
            isHostStarting = true,
            lastError = null,
        )
    }

    viewModelScope.launch {
        try {
            // Start Wi-Fi Direct / BLE / transport only after validation has passed.
            startHostTransportAndDiscovery()
            _uiState.update {
                it.copy(
                    hostState = HostLifecycleState.WAITING_FOR_LISTENERS,
                    isHostStarting = false,
                )
            }
        } catch (t: Throwable) {
            _uiState.update {
                it.copy(
                    hostState = HostLifecycleState.ERROR,
                    isHostStarting = false,
                    lastError = "Could not start hosting: ${t.message ?: t::class.simpleName}",
                )
            }
        }
    }
}
```

Use the actual repo method names, but preserve this structure:

1. Validate everything.
2. If invalid, update UI with a visible error and return before side effects.
3. Set `CREATING_SESSION`.
4. Start transport/discovery.
5. Set success or visible failure.

Do not start BLE advertising, Wi-Fi Direct, or TCP transport before the helper returns `null`.

---

## 4. Manual resync availability in `DISCONNECTED` state (P1.10)

**Decision: excluding `DISCONNECTED` is intentional.**

Manual resync is for clock/audio alignment over an active listener connection. It is not a reconnect button.

When the listener is `DISCONNECTED`, there is no reliable active transport for a sync probe. The user should use Retry/Rejoin/Cancel, not Manual Resync.

Implement this behavior:

- Enable Manual Resync only when there is an active selected session and a live transport/sync controller.
- Suggested eligible states:
  - `SYNCING_CLOCK`
  - `BUFFERING`
  - `PLAYING`
- Do not enable it in:
  - `IDLE`
  - `SESSION_SELECTED`
  - `JOIN_REQUESTED`
  - `AWAITING_APPROVAL`
  - `CONNECTING`
  - `DISCONNECTED`
  - `ERROR`
- If `manualResync()` is somehow called while invalid, do not silently return. Set a visible `lastError`.

Suggested helper:

```kotlin
fun AppUiState.canManualResync(): Boolean =
    selectedSession != null &&
        listenerState in setOf(
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.BUFFERING,
            ListenerLifecycleState.PLAYING,
        )
```

Suggested defensive implementation:

```kotlin
fun manualResync() {
    val state = _uiState.value
    if (!state.canManualResync()) {
        _uiState.update {
            it.copy(
                lastError = "Manual resync requires an active listener connection. Retry or rejoin the session first.",
            )
        }
        return
    }

    val controller = listenerSyncController
    if (controller == null) {
        _uiState.update {
            it.copy(lastError = "Manual resync is unavailable because sync has not been initialized.")
        }
        return
    }

    // Send probe / resync command here.
}
```

If the existing code has a better way to detect live transport than `listenerSyncController != null`, use that too.

---

## 5. `OboeBridge` diagnostics display (P2.1-P2.2)

**Decision: diagnostics must say AudioTrack is the playback output, and Oboe/native bridge is diagnostic-only until real native playback exists.**

The current naming is misleading if it implies Oboe is actually doing playback. For this pass:

- Playback output should be reported as `Android AudioTrack`.
- Native bridge should be reported separately.
- Native bridge availability is useful diagnostic information only.
- Native bridge unavailable should not crash the app.
- Native bridge unavailable should also not be hidden as if everything is equivalent.

Recommended UI lines:

```kotlin
Text("Playback output: Android AudioTrack")
Text("Native bridge: ${OboeBridge.statusSummary()}")
```

Recommended `OboeBridge` shape:

```kotlin
object OboeBridge {
    private val loadResult: Result<Unit> = runCatching {
        System.loadLibrary("silentdisco_oboe")
    }

    val isAvailable: Boolean
        get() = loadResult.isSuccess

    fun statusSummary(): String {
        return if (isAvailable) {
            val nativeStatus = runCatching { nativeBackendSummary() }
                .getOrElse { "loaded, but native status failed: ${it.message ?: it::class.simpleName}" }

            "Available — $nativeStatus; diagnostics only"
        } else {
            val reason = loadResult.exceptionOrNull()?.let {
                it.message ?: it::class.simpleName
            } ?: "native library not loaded"

            "Unavailable — $reason; playback uses AudioTrack"
        }
    }

    external fun nativeBackendSummary(): String
}
```

Do not call this an “Oboe playback engine” unless actual audio output goes through Oboe. If the class name remains `OboePlaybackEngine` for now, diagnostics and comments must be clear that it currently wraps Android `AudioTrack`.

---

## 6. Lint acceptance criteria

**Decision: do not upgrade AGP or compileSdk in this pass. Accept the known GradleDependency notices only if they are explicitly documented.**

The Gradle/compileSdk upgrade is a separate dependency-management pass. It should not be mixed into this UI/state-machine/silent-failure cleanup because it expands risk and makes review harder.

Acceptance criteria for this pass:

- `./gradlew lintDebug` should be run if the environment allows it.
- The existing 8 `GradleDependency` notices are acceptable only if documented with:
  - the exact dependency names,
  - the reason they are deferred,
  - the required AGP/compileSdk upgrade,
  - and a note that this pass must not introduce new lint categories.
- Do not suppress these notices with broad lint suppression.
- Do not create a lint baseline just to hide them.
- Any new lint failure introduced by this pass must be fixed before completion.

Add a short note to the implementation summary, for example:

```markdown
## Known deferred lint notices

`lintDebug` reports 8 existing `GradleDependency` notices for AndroidX / coroutine / Truth updates. These require AGP 9.1.0 and compileSdk 37. The project currently remains on AGP 8.9.1 and compileSdk 36. Dependency/platform upgrade is deferred to a separate pass. No new lint categories were introduced by this change.
```

If Claude Code cannot run lint because the environment cannot download Gradle or dependencies, document that honestly instead of claiming lint passed.

---

## 7. On-device integration test coverage

**Decision: do not make new `connectedDebugAndroidTest` coverage a blocker for this pass. Add or update manual real-device checklist items instead.**

This pass should focus on deterministic unit tests and state-machine tests. Physical-device coverage is valuable, but it is a separate validation pass because BLE/Wi-Fi Direct/audio behavior requires real devices and is slow to iterate.

Required for this pass:

- Unit tests for scan lifecycle:
  - scan sets `isScanning = true`,
  - scan timeout clears `isScanning`,
  - scan failure clears `isScanning` and sets visible error,
  - scan does not leave `listenerState = SCANNING` forever.
- Unit tests for invite-code validation:
  - invite-code mode with blank host code is rejected before side effects,
  - wrong listener invite code is rejected by host join handling,
  - correct invite code proceeds to pending/approved flow.
- Unit tests for manual resync:
  - invalid states produce visible error,
  - valid states call/send the resync operation.
- Unit tests for playback write behavior:
  - no fake-success write when audio output is unavailable,
  - fatal write failure produces visible error/state transition.
- Unit tests for connection progress:
  - buffering step appears between sync and playing,
  - fatal playback error does not pretend the listener is still playing.

Optional for this pass:

- Add fake-service or fake-transport instrumentation tests if they are cheap and do not require physical devices.
- Do not add physical-device-only `connectedDebugAndroidTest` items as required completion criteria.

Add these manual checklist items for the next real-device pass:

```markdown
## Manual real-device validation deferred to next pass

- [ ] Two-device BLE discovery: scan starts, indicator appears, scan ends, refresh button re-enables.
- [ ] Two-device invite-code rejection: wrong code is visibly rejected by host-side validation.
- [ ] Correct invite-code join: listener appears as pending/approved and can connect.
- [ ] Mid-stream underrun: listener moves to Buffering, then resumes Playing after buffer recovers.
- [ ] Fatal playback output failure, if reproducible: visible error, no fake playback success.
- [ ] Manual resync while connected sends a probe and shows visible progress/success.
- [ ] Manual resync while disconnected is disabled or produces a visible reconnect-required message.
```

---

## Final implementation guidance

Claude Code should implement this pass in small commits:

1. Scan lifecycle and `isScanning` correctness.
2. Host form validation and `CREATING_SESSION` startup state.
3. Invite-code enforcement.
4. Join progress buffering step.
5. Manual resync gating and visible invalid-state errors.
6. Playback volume/fake-write cleanup.
7. Oboe/native bridge diagnostics wording.
8. Unit tests and final lint/test documentation.

Do not bundle AGP/compileSdk upgrades into these commits.

Most importantly: any user-triggered action that cannot run must either be disabled in the UI or produce a visible error. No bare `return`, no fake success, and no log-only failure handling.
