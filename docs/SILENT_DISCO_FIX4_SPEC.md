# Silent Disco — Fix 4 Hardening Spec

Source reviewed: `silent_disco-master_2606281313.zip`  
Previous implementation plan: `SILENT_DISCO_FIX3_TODO(1).md`

## Purpose

Fix 4 is a focused correctness pass. Fix 3 added the right concepts: explicit transport results, playback engine injection, SDK-aware permissions, explicit scan state, and more honest diagnostics. The remaining problem is that several result objects and state helpers are still not enforcing production truth. The app can still report or imply success when a command/audio/sync operation was not delivered, and the listener/host resync flows are now coupled in a way that can break real joins.

This pass must make the app fail visibly and accurately when production operations fail. Do not hide failures by adding catch/log wrappers. Do not report success unless the operation actually succeeded or the UI explicitly says it was local/demo/diagnostic only.

## Scope

Fix the following areas:

1. Listener sync flow after approval and manual resync availability.
2. Host/listener separation for periodic sync behavior.
3. Host control/audio/sync broadcast result handling.
4. Wi-Fi Direct startup result honesty.
5. Host playback session/stream identity truth.
6. ViewModel-level guardrails for join/session selection.
7. BLE async failure surfacing.
8. Remaining diagnostic and test gaps.

## Non-goals

- Do not redesign the entire transport stack.
- Do not implement full streaming packetization yet; only remove the worst repeated-copy memory issue if not already removed.
- Do not add broad UX redesign or visual styling.
- Do not add production fallbacks that simulate network success.
- Do not silently ignore result objects because tests pass.

## Core invariants

### Invariant 1 — Result objects must be consumed

If a method returns `TransportOperationResult`, `BleOperationResult`, or `SendAllResult`, the caller must use that result to decide whether the user-facing state should say success, warning, or error. Merely changing a method to return a result type is not enough.

### Invariant 2 — Zero recipients is not successful delivery

For host control and sync broadcasts, `SendAllResult(peerCount = 0, successCount = 0, failureCount = 0)` means the command was delivered to no listeners. That is not successful command delivery. It may not always be fatal, but it must be visible in host diagnostics and/or UI state.

For host audio preview, zero peers may be normal before listeners join. It should not kill local host preview, but it must be disclosed as `0 connected listeners` or `No connected listeners for audio broadcast` rather than counted as a successful network delivery.

### Invariant 3 — Partial delivery is a warning immediately

If a broadcast reaches some listeners and fails for others, host diagnostics must show the partial delivery immediately. Do not wait until a threshold is reached before telling the host.

### Invariant 4 — Listener manual resync is listener-only

`manualResync()` is a listener action. It must not be called by host playback or host periodic jobs. Host code should respond to sync probes; it should not request listener-style manual resync through `selectedSession`.

### Invariant 5 — Initial listener sync must be allowed after approval

After a listener receives join approval, the listener must be able to send the initial sync probe while it is in `APPROVED`, `CONNECTING`, or `SYNCING_CLOCK`. Do not gate initial sync behind `BUFFERING` or `PLAYING` only.

### Invariant 6 — UI gating is not sufficient

If a UI disables an action, the ViewModel must also enforce the rule. A stale click, alternate UI path, test call, or future screen must not bypass the state machine.

### Invariant 7 — Production identity must not be invented silently

Host playback must not invent a new session ID when there is no active host session. If `currentSessionId` is null, fail visibly. If a stream ID is created, assign it to `currentStreamId` so pause/stop/end commands reference the actual stream.

### Invariant 8 — Async platform failures must surface

BLE scan/advertise callbacks and Wi-Fi Direct async callbacks can fail after synchronous start returns. Those failures must update UI/diagnostics, not just logs.

## Desired behavior by flow

### Discover / join flow

- Scan state is driven by `AppUiState.isScanning`.
- Scan state is cleared on cancel, leave, listener error, listener disconnect, and BLE scan failure.
- Join buttons are disabled while another join is active.
- `selectDiscoveredSession()` must enforce `canSelectSession()` in the ViewModel.
- If a session switch is rejected, set a visible `lastError` explaining that the current join must be finished or cancelled.

### Join approval / sync flow

- When `JoinApproval` is received:
  - set `selectedSession` and progress state;
  - transition toward `SYNCING_CLOCK` or `CONNECTING` as appropriate;
  - send an initial sync probe using listener-specific sync logic;
  - do not call a host-only or UI-only path.
- `manualResync()` is user-facing listener logic and must allow resync from approved/connecting/syncing/buffering/playing/reconnecting/desynced states.
- Demo/local resync fallback is allowed only for `BuildConfig.DEBUG && demo-session-*`, and the message must explicitly say it was local/demo.
- Real sessions without an active host connection must set a visible error: `Manual resync requires an active host connection`.

### Host startup

- `createHostSession()` validates form first.
- BLE advertising start failure returns false and blocks navigation.
- Wi-Fi Direct synchronous startup failure returns false and blocks navigation.
- If Wi-Fi Direct fails after BLE advertising has already started, BLE advertising is stopped.
- Wi-Fi Direct async failure after navigation must transition host UI/diagnostics to error immediately.

### Host control broadcasts

For `JoinApproval`, `JoinRejection`, `Pause`, `Stop`, `Disconnect/End Session`, stream start, stream stop, and sync responses:

- catch thrown errors and call host diagnostics helper;
- inspect `SendAllResult` on success;
- surface zero-peer and partial-delivery results;
- avoid claiming command delivery when nobody received it.

For approval specifically, do not remove a pending request or add an approved listener until the approval is actually delivered to at least one peer, unless there is an explicitly disclosed demo path.

### Host audio broadcast

- Host preview may continue when zero listeners are connected.
- Zero listeners must be visible in diagnostics or UI.
- Partial delivery must update diagnostics immediately.
- Consecutive failures should stop the host stream at a threshold, e.g. 10 consecutive failed/partial delivery cycles. Zero peers should not count as a transport failure that kills preview, but should be counted separately as `zeroPeerBroadcastCount` if useful.

### Playback engine behavior

- Do not collapse negative `AudioTrack.write()` error codes into `0` before error handling.
- Preserve the actual write result in the error message.
- Listener and host playback write failures must update visible UI state and diagnostics.

### BLE async failure behavior

- `BleDiscoveryService` should expose async scan/advertise failures through a `SharedFlow` or `StateFlow`.
- `MainViewModel` should collect those failures.
- Scan failure clears `isScanning`, sets listener error state, updates listener diagnostics.
- Advertise failure during host startup/hosting sets host error state, stops host transport/advertising if necessary, updates host diagnostics.

## Acceptance criteria

The pass is complete only when all of these are true:

- Initial sync after join approval works; it is not rejected by `canManualResync()`.
- Host playback no longer calls listener `manualResync()` periodically.
- Every `broadcastControl`, `broadcastSyncResponse`, and `broadcastAudio` result is either consumed or explicitly documented as safe to ignore.
- Zero connected listeners are not treated as successful network audio delivery.
- Partial host broadcast delivery is visible immediately.
- Wi-Fi Direct missing permission returns a failed startup result.
- Wi-Fi Direct async group failure transitions host UI/diagnostics to error.
- `selectDiscoveredSession()` enforces join gating in ViewModel.
- Starting host playback without an active host session sets host error state and host diagnostics.
- Created stream IDs are stored in `currentStreamId`.
- BLE async scan/advertise failures update UI/diagnostics.
- No user-triggered operation has a `runCatching { ... }.onFailure { logger... }` path without UI/diagnostic state.
- Tests cover production helpers or ViewModel behavior, not copied local validators.

## Validation commands

Run:

```bash
./gradlew test
./gradlew lintDebug
```

If either fails, list every failure and fix or document it before handing back.

Useful grep checks:

```bash
# Host/listener resync separation
 grep -R "startPeriodicResync\|manualResync()" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# All broadcast result call sites must inspect SendAllResult
 grep -R "broadcastControl\|broadcastSyncResponse\|broadcastAudio" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt

# No log-only runCatching failure handlers for user-visible operations
 grep -R "onFailure.*logger\.\|logger\.w" app/src/main/java/com/ekkus/silentdisco -n

# No invented production session IDs
 grep -R "UUID.randomUUID\|stream-\${SystemClock.elapsedRealtime" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
```
