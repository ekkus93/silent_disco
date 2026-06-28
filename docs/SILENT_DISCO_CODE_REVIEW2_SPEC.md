# Silent Disco — Code Review 2 Hardening Spec

Generated from review of `silent_disco-master_2606280240.zip` after the Code Review 1 UI/UX pass.

## 0. Purpose

This pass must convert the UI/UX improvements from Code Review 1 from "looks finished" into real, reliable app behavior.

The main theme is: **no user-visible state may claim success, progress, playback, advertising, scanning, sync, or transport delivery unless the underlying operation actually happened or is explicitly simulated in a debug/demo path.**

Claude Code should treat every silent return, swallowed exception, fake success value, and log-only failure as suspicious. If an operation cannot complete, the app must either:

1. show a clear user-facing error via `lastError`,
2. disable/hide the action before the user can tap it,
3. expose an honest diagnostic state, or
4. restrict the behavior to `BuildConfig.DEBUG` with explicit debug/demo labeling.

## 1. Current problems this spec addresses

### 1.1 Scan lifecycle is not explicit

`DiscoverSessionsScreen` currently derives `isScanning` from `listenerState == SCANNING`. `MainViewModel.scanForSessions()` sets `listenerState = SCANNING`, but there is no reliable transition back to a non-scanning state after the scan window completes. This can leave the "Scan / Refresh" button disabled and the progress indicator visible forever.

Expected behavior:

- Scanning must have its own `AppUiState.isScanning` flag.
- Scan start, scan completion, scan cancellation, scan failure, and retry must all explicitly update `isScanning`.
- `listenerState` may still reflect the listener flow, but it must not be the only source of truth for a transient scan operation.

### 1.2 Host startup loading state is mostly dead code

`HostSetupScreen` looks for `hostState == CREATING_SESSION`, but `createHostSession()` does not set that state before starting BLE/Wi-Fi Direct. The UI path for "Starting…" may never render.

Expected behavior:

- `createHostSession()` must set `hostState = CREATING_SESSION` before starting BLE and Wi-Fi Direct.
- On success, transition through `ADVERTISING` or directly to `WAITING_FOR_LISTENERS`, but only after the start operations have not failed synchronously.
- On failure, transition to `ERROR`, set `lastError`, and update diagnostics.

### 1.3 Some Code Review 1 TODOs are still incomplete

The next pass must finish the incomplete pieces:

- Replace the raw diagnostic playback enum in `DiagnosticsScreen`.
- Add the missing "Buffering audio" step to the join progress stepper.
- Disable Join buttons on discovered session cards while a join is already active.
- Make the scan progress indicator use explicit `uiState.isScanning`.
- Make host startup loading state actually reachable.

### 1.4 Volume slider is likely visual-only

`setLocalVolume()` only updates `AppUiState.localVolume`. The playback engine does not appear to apply this value to `AudioTrack` or to PCM samples. This is a silent no-op.

Expected behavior:

- The listener volume slider must change actual playback output volume.
- The host preview/local playback volume should either use the same value if that is intended, or the UI text must say this is listener-local volume only.
- If no playback engine is active, changing volume may update stored state, but the next engine start must apply the current volume.

### 1.5 Audio writes fake success when there is no active `AudioTrack`

`OboePlaybackEngine.write()` currently returns `frame.packet.payload.size` when `audioTrack` is null. That is dangerous because higher-level playback code can treat a dropped frame as successful output.

Expected behavior:

- Writing before `start()` must not report success.
- A failed or short `AudioTrack.write()` must be visible to the caller.
- Playback loops must catch write failures, set `PlaybackState.ERROR`, update diagnostics, and stop or recover explicitly.

### 1.6 Oboe naming/status is misleading

The class is named `OboePlaybackEngine`, but real playback is `AudioTrack`. `OboeBridge` only reports native strings and swallows native load failures.

Expected behavior:

- The UI and diagnostics must honestly say which playback path is in use.
- Either rename the engine to reflect `AudioTrack`, or clearly document that it is an `AudioTrack` playback engine with optional Oboe/native status diagnostics.
- Do not imply that native Oboe playback is active unless it actually is.

### 1.7 BLE discovery/advertising failures are log-only

`BleDiscoveryService.startAdvertising()` and `startScanning()` can fail due to missing permissions, unavailable BLE advertiser/scanner, callback failures, or thrown platform errors. Several of these currently log and return without propagating failure.

Expected behavior:

- Start methods must return an explicit result for synchronous failures.
- Async BLE callback failures must update an observable event/state that the ViewModel consumes.
- Host/session UI must not show successful advertising if BLE advertising failed.
- Listener UI must not show endless scanning if BLE scan start failed.

### 1.8 Invite-code mode is not enforced by the real host path

The demo path rejects a wrong hardcoded invite code, but `handleJoinRequestMessage()` accepts real join requests into pending requests without validating the code against the host form.

Expected behavior:

- If `hostForm.approvalMode == INVITE_CODE`, the host must compare the incoming `message.inviteCode` against the current host invite code.
- Wrong or missing codes must be rejected with `ControlMessage.JoinRejection` and must not appear in pending approvals.
- The listener must show the rejection reason.
- The code must not be hardcoded to `"1234"` outside a debug/demo session.

### 1.9 Manual resync can silently do nothing

`manualResync()` returns silently if there is no sync controller and no selected session.

Expected behavior:

- The Diagnostics screen must disable "Manual Resync" unless there is a selected listener session or active sync controller.
- `manualResync()` must set `lastError` if called without a valid session.
- Manual resync should produce a visible success/progress message when a probe is sent or a local fallback response is applied.

### 1.10 Host stream send failures are swallowed per packet

`startHostStreamingLoop()` catches `broadcastAudio(packet)` failures, logs a warning, and keeps going. That can result in the host UI saying streaming is active while no listeners are receiving packets.

Expected behavior:

- Packet send failures must update host diagnostics and user-visible state.
- Repeated failures must transition host stream state to `ERROR` or `PAUSED` with clear recovery behavior.
- Sending to zero peers must be explicitly represented in diagnostics. It may be allowed for host preview, but it must not be counted as successful listener delivery.

## 2. State model requirements

### 2.1 `AppUiState` additions

Add explicit transient and capability fields rather than deriving everything from lifecycle enums:

- `isScanning: Boolean`
- `isJoinInProgress: Boolean`
- `canManualResync: Boolean` or a production helper that derives it from selected session / sync state
- optionally `hostStartupInProgress: Boolean` if `hostState == CREATING_SESSION` is not enough

`isJoinInProgress` should be true for these listener states:

- `JOIN_REQUESTED`
- `AWAITING_APPROVAL`
- `APPROVED`
- `CONNECTING`
- `SYNCING_CLOCK`
- `BUFFERING`
- `PLAYING`
- `RECONNECTING`
- `DESYNCED`

It should be false for:

- `IDLE`
- `SCANNING`
- `SESSION_SELECTED`
- `DISCONNECTED`
- `ERROR`

### 2.2 `ConnectionProgressState` additions

Add an explicit buffering flag:

- `buffered: Boolean = false`

Then add:

- `bufferingStep()`

The order must be:

1. Discovering session
2. Sending join request
3. Awaiting host approval
4. Connecting transport
5. Syncing clock
6. Buffering audio
7. Playing

### 2.3 State transition expectations

#### Scan

- Before scan: `isScanning = false`
- On scan start: `isScanning = true`, `listenerState = SCANNING`, clear stale scan errors
- On scan complete with sessions: `isScanning = false`, `listenerState = IDLE` unless a session is already selected
- On scan complete with no sessions: `isScanning = false`, `listenerState = IDLE`, `lastMessage = "No sessions found"` or no message if UI already displays empty state
- On scan failure: `isScanning = false`, `listenerState = ERROR`, `lastError = meaningful error`
- On leaving listener flow: cancel active scan job and set `isScanning = false`

#### Host startup

- Validate form before side effects.
- On valid start request: `hostState = CREATING_SESSION`, clear `lastError`.
- If BLE advertising cannot start: `hostState = ERROR`, `lastError = ...`, do not navigate to Host Control.
- If Wi-Fi Direct host start cannot start: `hostState = ERROR`, `lastError = ...`, do not navigate to Host Control.
- On success: `hostState = WAITING_FOR_LISTENERS`, `lastMessage = "Hosting <name>"`.

#### Join request

- Selecting a discovered session sets `listenerState = SESSION_SELECTED` and `isJoinInProgress = false`.
- Sending a join request sets `listenerState = JOIN_REQUESTED` or `CONNECTING` only after transport send has either been queued or simulated intentionally.
- Real invite-code rejection sets `listenerState = ERROR`, `listenerPlaybackState = ERROR`, and an actionable `lastError`.

#### Buffering/playback

- On stream start: `listenerState = BUFFERING`, `listenerPlaybackState = BUFFERING`, `connectionProgress.buffered = false`.
- When `scheduler.canStart()` becomes true: set `connectionProgress.buffered = true`, then `listenerState = PLAYING`, `listenerPlaybackState = PLAYING`, `connectionProgress.playing = true`.
- If buffering times out: set `listenerState = ERROR` or `DESYNCED`, with clear error text.

## 3. Error-handling policy

Claude Code must remove or justify every one of these patterns:

```kotlin
?: return
```

when the function was triggered by a user action.

```kotlin
runCatching { ... }.onFailure { logger.w(...) }
```

when failure affects user-visible behavior.

```kotlin
getOrDefault("Unavailable")
```

when the default can make the app appear to work despite a missing implementation.

```kotlin
?: frame.packet.payload.size
```

when returning bytes written / packets sent / successful operation counts.

Allowed exceptions:

- Debug/demo-only paths gated by `BuildConfig.DEBUG`.
- Best-effort diagnostics text where failure is itself the diagnostic result.
- Cleanup functions where idempotent no-op is correct, such as stopping an already-stopped service.

## 4. UI requirements

### 4.1 Discover Sessions

- Use `uiState.isScanning` for progress indicator and button enabled state.
- Disable session-card Join buttons when `uiState.isJoinInProgress` is true and the card is not the selected active session.
- If a join is in progress, show a short helper text: `"Finish or cancel the current join before joining another session."`

### 4.2 Join Progress

- Add "Buffering audio" step.
- Show active spinner for the exact active step.
- Keep status text human-readable.
- Keep action visibility rules from Code Review 1.
- Disable "Request Join" when invite code is required and blank.

### 4.3 Diagnostics

- No raw enum strings.
- `Playback state` must use `PlaybackState.label()`.
- `Manual Resync` must be disabled when there is no active listener session to resync.
- Add text explaining why Manual Resync is unavailable.
- Audio backend card must be honest: `AudioTrack playback active`, `Native Oboe bridge available`, `Native Oboe bridge unavailable`, etc.

### 4.4 Host Setup

- Show actual startup loading while host session creation is running.
- Do not navigate to Host Control if BLE or Wi-Fi Direct start failed.
- Show validation helper text for missing session name, missing audio file, and invite-code mode with blank invite code.

### 4.5 Listener Playback

- Volume slider must change actual playback output.
- If playback is buffering, show progress as Code Review 1 requested.
- If playback engine errors, show error state and diagnostics.

## 5. Transport/discovery requirements

### 5.1 BLE operation results

Introduce an explicit result/event model. The exact shape can vary, but it must support at least:

- started successfully / start attempted
- missing permission
- advertiser/scanner unavailable
- platform start exception
- async callback failure code

The ViewModel must consume this and update `lastError`, diagnostics, and lifecycle state.

### 5.2 TCP send semantics

`sendAll()` should return a send result rather than `Unit`, or throw/report when zero peers receive a message.

Recommended semantics:

- `SendAllResult(peerCount, successCount, failureCount)`
- Zero peers is not an exception by itself, but it must be surfaced to diagnostics and not counted as delivered audio.
- Failures to all peers during active stream should trigger visible host error/recovery behavior.

## 6. Audio requirements

### 6.1 Honest playback engine

Either:

- rename `OboePlaybackEngine` to `AudioTrackPlaybackEngine`, or
- keep the name temporarily but update comments, status strings, and diagnostics to say it uses Android `AudioTrack`.

### 6.2 No fake write success

- `write()` before `start()` must fail loudly.
- Non-positive write result must fail or return an explicit error result.
- Short writes must be counted and surfaced.

### 6.3 Volume control

- Add `setVolume(volume: Float)` to the playback engine.
- Store current volume inside the engine so it can be applied to future `AudioTrack` instances.
- `MainViewModel.setLocalVolume()` must call the engine.
- Tests must verify the ViewModel invokes the engine or a testable abstraction.

## 7. Testing requirements

Add or update tests for:

1. Scan lifecycle: starts, completes, failure clears `isScanning`.
2. Discover session Join buttons disabled during active join.
3. Diagnostics playback state uses label helper.
4. Join progress includes Buffering audio and step state transitions.
5. Host startup sets `CREATING_SESSION` before transport start and `ERROR` on BLE/Wi-Fi failure.
6. Wrong invite code is rejected by real `handleJoinRequestMessage()` path.
7. Manual resync without selected session sets `lastError` or is disabled by helper.
8. Playback engine write before start fails; no fake success.
9. Volume update reaches the playback engine.
10. Packet broadcast failures update diagnostics and do not silently continue forever.

Prefer production helper functions for UI-gating tests instead of tests that duplicate constants. Example: test `AppUiState.canJoinSessionCard(session)` rather than re-creating the state list inside the test.

## 8. Acceptance criteria

This pass is complete only when:

- `./gradlew test` passes.
- `./gradlew lintDebug` either passes or every failure is explicitly documented with a real fix plan. Do not suppress lint without a reason.
- No user action silently returns without feedback.
- No scan can remain stuck forever.
- No raw enum names are visible in normal UI.
- Join progress shows all seven required steps, including buffering.
- Invite-code mode is enforced by the host for real join messages.
- Volume slider changes actual playback volume.
- Audio writes do not claim success when the engine is not started.
- BLE scan/advertise failures are surfaced to UI and diagnostics.
- Host streaming does not hide repeated transport send failures.

## 9. Non-goals

Do not attempt a full architecture rewrite. Do not replace the entire transport layer. Do not implement real native Oboe playback unless it is small and well-tested. The goal is to harden the current app, finish the incomplete UI/UX pass, and remove dangerous silent behavior.
