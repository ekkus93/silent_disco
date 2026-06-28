# Silent Disco — Fix 3 Hardening Specification

Source reviewed: `silent_disco-master_2606281224.zip`  
Prior TODO baseline: `SILENT_DISCO_CODE_REVIEW2_TODO(1).md`

## 1. Purpose

This pass must finish the incomplete Code Review 2 hardening work and remove the remaining places where the app can quietly lie to the user. The app must not report scanning, hosting, streaming, playback, approval, pause, stop, or resync success unless the underlying production path actually succeeded or the UI explicitly says the behavior is local/debug/demo-only.

The highest-risk remaining issues are:

1. Listener playback can enter `PLAYING` and then crash the coroutine because `OboePlaybackEngine.write()` now throws when the engine is not started.
2. The Discover screen still derives scanning from `listenerState` instead of the new explicit `AppUiState.isScanning` field.
3. Join buttons are still enabled while another join is active.
4. `createHostSession()` can still return success after `WifiDirectTransportService.startHost()` internally fails without throwing.
5. Host audio packet broadcast failures are still log-only and can continue forever.
6. Host control broadcasts for approval/rejection/pause/stop/end-session still swallow failures or update the wrong diagnostics surface.
7. Diagnostics still displays raw enum names and ambiguous native/AudioTrack output labels.
8. Android-version permission checks can report missing permissions that were never requested on older Android versions.

## 2. Core rule for Fix 3

Do not fix these issues by wrapping more code in broad `try/catch` or `runCatching` and doing nothing in `onFailure`.

Every user-triggered operation must have one of these outcomes:

- success reflected in UI state and diagnostics;
- explicit failure reflected in `lastError` and the appropriate host/listener diagnostics;
- explicitly disclosed local/demo/debug behavior; or
- clearly disabled UI action with explanatory helper text.

A log line by itself is not a valid failure path for user-visible operations.

## 3. State and diagnostics invariants

### 3.1 Scan lifecycle

`isScanning` is the source of truth for whether the scan UI is busy. `listenerState == SCANNING` is an event/state-machine stage, not the UI spinner source of truth.

Required invariants:

- Starting a scan sets `isScanning = true`.
- Scan completion, empty scan, permission failure, BLE scan start failure, listener reset, cancel join, and leave session all set `isScanning = false`.
- The Scan / Refresh button must be re-enabled after every terminal scan outcome.
- The UI must not infer scanning from `listenerState`.

### 3.2 Join gating

A listener can only interact with the selected active join target while a join is in progress. Other session Join buttons must be disabled with clear helper text.

Required invariants:

- `AppUiState.isJoinInProgress()` must be the production helper.
- `AppUiState.canSelectSession(session)` must be used by the Discover UI.
- The selected session can remain enabled for idempotent re-selection only if that behavior is intentional.
- Different sessions must be disabled while the join path is active.

### 3.3 Host startup

Host startup must not navigate to Host Control unless both BLE advertising and Wi-Fi Direct host startup successfully began.

Required invariants:

- `createHostSession()` must return `false` when BLE advertising cannot start.
- `createHostSession()` must return `false` when Wi-Fi Direct host startup synchronously fails or immediately moves the transport snapshot into `FAILED`.
- If Wi-Fi Direct failure is asynchronous, the error must update host state/diagnostics and prevent a fake ready/hosting state.
- A blank invite code in invite-code mode must be treated as invalid input, not silently auto-generated during form update.

### 3.4 Playback engine

The playback engine is currently Android `AudioTrack`-backed. It must not be presented as native Oboe playback unless PCM output is actually routed through Oboe.

Required invariants:

- `write()` before `start()` must fail.
- Every production playback loop must catch write/start failures and transition to a visible error state.
- Listener playback must call `playbackEngine.start(format)` before the first write. Do not rely on host preview having already started the shared engine.
- Host preview playback and listener playback should not accidentally share a stale `AudioTrack` format.
- Volume changes must call the real playback engine setter, not only update UI state.

### 3.5 Real transport delivery

Packet and control-message delivery must not be counted as success merely because the app attempted to send.

Required invariants:

- `TcpServerChannel.sendAll()` must return delivery stats.
- `WifiDirectTransportService.broadcastAudio()` must return delivery stats.
- Host stream loop must surface repeated audio broadcast failures and stop the stream after a threshold.
- Zero connected peers must be tracked honestly. It may be allowed for host preview, but it must not be counted as successful delivery.
- Control broadcasts for approval/rejection/pause/stop/disconnect must update host diagnostics if sending fails.

### 3.6 Buffering progress

The connection progress UI must represent the true pipeline order:

1. Discovering session
2. Sending join request
3. Awaiting host approval
4. Connecting transport
5. Syncing clock
6. Buffering audio
7. Playing

Required invariants:

- `buffered = true` before or at the same state transition that sets `playing = true`.
- Stopping, leaving, disconnecting, or erroring clears `buffered` and `playing` unless doing a full `ConnectionProgressState()` reset.
- Real transport playback and simulation playback must follow the same progress semantics.

### 3.7 Native bridge diagnostics

Native bridge load failure is allowed for diagnostics, but it must be explicit.

Required invariants:

- Diagnostics must say `Playback output: Android AudioTrack`.
- Diagnostics must separately say `Native bridge: ...`.
- `OboeBridge` must retain structured load availability state.
- Native bridge query failures may be summarized in diagnostics, but cannot be used to imply native playback is active.

### 3.8 Android permission model

The app has `minSdk = 29`, so required permission state must match the current SDK.

Required invariants:

- Android 13+ uses `NEARBY_WIFI_DEVICES` and Android 12+ Bluetooth runtime permissions.
- Android 12 and below must not require `NEARBY_WIFI_DEVICES`.
- Android 11 and below must not require Bluetooth runtime permissions that do not exist.
- `PermissionCatalogue.requiredPermissions()` and the Compose runtime permission launcher must agree.
- `hasHostTransportPermissions()` and `hasListenerTransportPermissions()` must evaluate only the permissions relevant to the device SDK.

## 4. Architecture decisions

### 4.1 Result types are preferred over internal fail-and-return

For production start/send methods, prefer explicit result objects over methods that call `fail(...)` internally and return `Unit`.

Recommended result shape:

```kotlin
data class TransportOperationResult(
    val started: Boolean,
    val message: String? = null,
) {
    companion object {
        val Started = TransportOperationResult(started = true)
        fun failed(message: String) = TransportOperationResult(started = false, message = message)
    }
}
```

For delivery:

```kotlin
data class SendAllResult(
    val peerCount: Int,
    val successCount: Int,
    val failureCount: Int,
) {
    val deliveredToAnyPeer: Boolean get() = successCount > 0
}
```

### 4.2 Use focused failure helpers in `MainViewModel`

Add small helpers instead of duplicating error-state transitions:

- `clearScanState()`
- `handleHostStartupFailure(message: String, sessionId: String? = null)`
- `handleHostControlFailure(action: String, error: Throwable)`
- `handleHostPlaybackEngineFailure(error: Throwable)`
- `handleListenerPlaybackEngineFailure(error: Throwable)`
- `handleRepeatedAudioBroadcastFailure(message: String)` if this makes the stream loop cleaner

The helper must update the correct surface. For example, a failed host approval broadcast is a host control failure, not a listener connection failure.

### 4.3 Debug/demo behavior must be gated and disclosed

Acceptable:

```kotlin
val shouldSimulate = BuildConfig.DEBUG && session.id.startsWith("demo-session-")
```

Not acceptable:

- generating random host/session IDs during production playback start;
- applying fake local resync for a real remote session without disclosure;
- hard-coded invite codes in non-demo paths;
- placeholder/trusted-device modes that sound production-ready without disclosure.

## 5. Required implementation areas

### 5.1 Discover and scan UI

Files:

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`

Required changes:

- Replace derived scanning state with `uiState.isScanning`.
- Use `canSelectSession(session)` in each session row.
- Clear scan state in cancel/leave/reset paths.

### 5.2 Playback engine and listener loops

Files:

- `app/src/main/java/com/ekkus/silentdisco/core/audio/PlaybackScheduling.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/audio/OboeBridge.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

Required changes:

- Rename `OboePlaybackEngine` to `AudioTrackPlaybackEngine`, or add a compatibility typealias for low churn.
- Ensure every playback loop calls `start(format)` before writing.
- Catch write/start failures and update visible error state.
- Split diagnostics into playback output vs native bridge.

### 5.3 Host startup and transport result flow

Files:

- `app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Required changes:

- Change `SessionTransport.startHost(session)` to return `TransportOperationResult`, or add a `startHostResult(session)` method and migrate production caller.
- `WifiDirectTransportService.startHost()` must return failure when manager/channel are missing or permission is missing.
- `createHostSession()` must check the returned result before updating UI to `WAITING_FOR_LISTENERS`.

### 5.4 Host streaming send failure handling

Files:

- `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Required changes:

- `TcpServerChannel.sendAll()` returns `SendAllResult`.
- Broadcast methods return `SendAllResult` where applicable.
- Host stream loop tracks consecutive audio send failures.
- Stop stream and show error after threshold.
- Treat zero-peer broadcast separately from failure.

### 5.5 Host control broadcast failure handling

Files:

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Required changes:

- `approveJoinRequest()` failure must call a host-side failure helper.
- `rejectJoinRequest()`, `pauseHostPlayback()`, `stopHostPlayback()`, stream-start broadcast, stream-stop broadcast, and `endSession()` must not silently swallow broadcast failure.
- The user-facing state can remain locally updated for pause/stop/end, but `lastError` and host diagnostics must disclose that listener notification failed.

### 5.6 Permission model

Files:

- `app/src/main/java/com/ekkus/silentdisco/core/permissions/Permissions.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Required changes:

- Add SDK-aware required permission helpers.
- Use the same helper for UI requests and state checks.
- Verify Android 29, 30, 31, 32, 33+ behavior.

## 6. Testing requirements

Run or manually explain failures for:

```bash
./gradlew test
./gradlew lintDebug
```

Add/adjust tests for:

1. Discover UI uses `isScanning`, not `listenerState`.
2. Join buttons disable during active join.
3. `cancelJoin()` and `leaveSession()` clear `isScanning`.
4. `createHostSession()` returns false when Wi-Fi Direct start returns failure.
5. Host setup invite-code mode cannot start blank.
6. `write()` before start throws.
7. Listener playback engine write failure updates listener error state.
8. Host playback engine write failure updates host error state.
9. `sendAll()` returns peer/success/failure counts.
10. Host stream loop stops after repeated audio send failures.
11. Diagnostics uses labels, not raw enum display.
12. Permission required list differs correctly by SDK.

## 7. Definition of done

Fix 3 is done only when all of these are true:

- No P0 item from the TODO is left partially wired.
- Scan button always re-enables after scan terminal states.
- Join buttons cannot start a second join while one is active.
- Host session creation cannot navigate on immediate BLE or Wi-Fi Direct startup failure.
- Playback cannot silently die in a coroutine after the UI says `PLAYING`.
- Host stream send failures are counted and surfaced.
- Host control broadcast failures are visible in host diagnostics.
- Diagnostics labels are honest about AudioTrack vs native bridge.
- No raw playback enum text remains in UI.
- Runtime permission checks match the current Android SDK.
- Every remaining `runCatching`/`onFailure` in user-triggered paths updates UI/diagnostics or is documented as intentionally internal/idempotent.
