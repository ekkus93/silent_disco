# Silent Disco — Fix 5 Hardening Spec

Source reviewed: `silent_disco-master_2606281350.zip`
Previous pass reviewed: `SILENT_DISCO_FIX4_TODO(1).md`

## Purpose

Fix 5 is a focused cleanup pass after Fix 4. Fix 4 added most of the correct primitives: listener sync helper, broadcast delivery reporting, BLE async failure events, Wi-Fi Direct startup results, and ViewModel join guarding. The remaining issue is that a few production paths still mutate UI state before delivery is known, force misleading lifecycle states, or leave stale progress flags after failure.

This pass is not a styling pass and not a general refactor. It is a correctness/hardening pass whose goal is:

1. Do not show success before transport delivery is confirmed.
2. Do not let periodic sync corrupt active playback state.
3. Do not leave progress flags saying `buffered`/`playing` after disconnect, sync failure, or listener error.
4. Do not let listener-side transport failures leave the app stuck in `CONNECTING`.
5. Do not keep tests that only validate copied local logic while production behavior remains untested.

## Non-goals

- Do not redesign the full transport layer.
- Do not implement chunk-by-chunk audio streaming unless it is already small and obvious.
- Do not replace Wi-Fi Direct or BLE.
- Do not add broad `try/catch` wrappers that only log.
- Do not add new fake/demo paths unless they are debug-gated and visibly disclosed.

## Current high-risk bugs to fix

### 1. Periodic listener resync regresses active listener state

`requestListenerSyncProbe(source)` currently forces `listenerState = SYNCING_CLOCK` whenever the transport is connected. That is right for initial sync, but wrong for periodic resync while the listener is already `PLAYING`, `BUFFERING`, `DESYNCED`, or `RECONNECTING`.

A periodic resync should send a probe and update `lastMessage`, but it should not change a playing listener back to `SYNCING_CLOCK` unless the app is truly entering the initial clock-sync phase.

Expected behavior:

- Initial sync from `APPROVED`, `CONNECTING`, or `SYNCING_CLOCK`: set listener state/current progress to `SYNCING_CLOCK`.
- Manual/periodic resync from `BUFFERING`, `PLAYING`, `RECONNECTING`, or `DESYNCED`: keep the current listener state and progress state.
- Demo-local resync remains debug-only and says it is local/demo.

### 2. Listener transport failure is visible but not terminal

When listener connection or join transport fails, the transport snapshot can enter `FAILED`, but the ViewModel only sets `lastError` in some cases. It can leave the listener in `CONNECTING`, `AWAITING_APPROVAL`, `APPROVED`, `SYNCING_CLOCK`, `BUFFERING`, or `PLAYING` even though transport has failed.

Expected behavior:

- Listener-side `TransportConnectionState.FAILED` with `lastError` must call `handleListenerConnectionFailure(...)` or equivalent.
- The listener state becomes `ERROR`.
- `listenerPlaybackState` becomes `ERROR`.
- `connectionProgress.buffered` and `connectionProgress.playing` become `false`.
- `pendingJoinRequestMessage` is cleared.
- `isScanning` is false and `scanJob` is cancelled.

### 3. Join rejection still claims success before delivery

`approveJoinRequest()` was fixed in Fix 4: approval is only applied locally after delivery succeeds. `rejectJoinRequest()` and the invite-code rejection path still update local state or diagnostics before delivery is known.

Expected behavior:

- `rejectJoinRequest(request)` sends `JoinRejection`, consumes `SendAllResult`, and only removes the pending request if delivery succeeded.
- Wrong invite-code rejection should not say “Rejected X” as if the listener was notified unless the rejection was actually delivered.
- If zero peers or partial delivery occurs, host diagnostics must show the delivery failure.

### 4. Pause/stop/end-session continue to ignore delivery truth

Fix 4 consumes `SendAllResult` in many broadcast paths, but pause/stop/end local state changes still happen before delivery and ignore the boolean result from `reportHostBroadcastDelivery(...)`.

This pass should make the behavior explicit:

- If the operation is a local host playback operation that can happen even with zero listeners, the UI message must say local state changed and listeners may not have received it.
- If the operation is supposed to notify an existing listener, zero peers/partial delivery must be visible in host diagnostics.
- Do not clear `lastError` immediately after `reportHostBroadcastDelivery(...)` has set a delivery warning.

### 5. Zero-listener audio broadcast is diagnostics-only, not UI-visible

Fix 4 discloses zero audio listeners in host diagnostics, but not always through `_uiState.lastError`. The host should see this clearly without digging into diagnostics.

Expected behavior:

- Audio broadcast with `peerCount == 0` sets host diagnostics and `_uiState.lastError` to `No connected listeners for audio broadcast`.
- This should not stop local host preview.
- Partial audio delivery should update both diagnostics and `_uiState.lastError` immediately.

### 6. Failure handlers leave stale progress flags

Some failure/disconnect paths still update lifecycle/playback state without clearing `connectionProgress.buffered` and `connectionProgress.playing`.

Expected behavior:

- `handleSyncFailure(...)` clears both flags.
- `handleListenerDisconnect(...)` clears both flags.
- `handleListenerConnectionFailure(...)` already clears them; verify it still does.
- Any listener stop/error/disconnect path should never leave the progress UI saying “Buffering audio” or “Playing” after failure.

### 7. Demo simulation still calls user-facing `manualResync()`

The demo simulation path still calls `manualResync()`. That is not host misuse, but it confuses validation grep and reuses a user-action wrapper internally.

Expected behavior:

- Demo simulation should call `requestListenerSyncProbe(source = "Demo clock sync")`.
- Any demo-local result message must remain debug-gated and disclosed.

### 8. Tests still avoid production behavior

Several tests are still tautological:

- Tests that create an `AppUiState`, copy it to a desired state, then assert that copied state.
- Tests that define a local validator duplicating production validation.
- Tests that test `MutableSharedFlow` directly instead of BLE/ViewModel behavior.

Expected behavior:

- Prefer ViewModel tests using fakes where practical.
- If full ViewModel construction is too heavy, extract small production helpers and test those helpers.
- Do not keep copied local validators as the only coverage for production behavior.

## Required implementation style

### Result objects must be consumed

If a method returns `SendAllResult`, the call site must inspect it unless there is a nearby comment explaining why the result is intentionally irrelevant.

Good:

```kotlin
val delivered = runCatching { wifiDirectService.broadcastControl(message) }
    .map { result -> reportHostBroadcastDelivery("broadcast pause", result, requireAnyPeer = false) }
    .getOrElse { error ->
        handleHostControlFailure("broadcast pause", error)
        false
    }
```

Bad:

```kotlin
runCatching { wifiDirectService.broadcastControl(message) }
    .onFailure { logger.w("transport", "failed") }
```

### State changes must happen in the right order

For operations where delivery defines success, send first and mutate local state after delivery succeeds.

Applies to:

- Approving a join request.
- Rejecting a join request.
- Invite-code rejection if the diagnostic claims the listener was notified.

For local host-only controls, state can change locally before network delivery, but messaging must be honest:

- “Paused locally; no connected listeners received the pause command.”
- “Stopped locally; some listeners may not have received the stop command.”

### Internal helpers should not be user-action wrappers

Use private/internal helpers for internal workflows:

- Initial clock sync.
- Periodic listener resync.
- Demo clock sync.

Keep public user actions like `manualResync()` as validation wrappers around the internal helper.

### No stale progress flags

Whenever listener playback is stopped, disconnected, desynced into error, or sync fails, `buffered` and `playing` must be false.

## Acceptance criteria

- Periodic resync while playing does not change `listenerState` away from `PLAYING`.
- Initial sync after join approval still enters `SYNCING_CLOCK` and sends a sync probe.
- Listener transport failure maps to listener `ERROR`, not stale `CONNECTING`.
- Rejection is not removed from pending list unless rejection delivery succeeds.
- Invite-code rejection diagnostics do not claim delivered rejection if delivery failed.
- Pause/stop/end-session delivery failures are visible and not immediately overwritten by success messages.
- Zero-listener audio broadcast updates `_uiState.lastError` and host diagnostics but keeps host preview alive.
- `handleSyncFailure()` and `handleListenerDisconnect()` clear `buffered` and `playing` flags.
- Demo simulation uses `requestListenerSyncProbe("Demo clock sync")`, not `manualResync()`.
- Tests verify production helpers or real ViewModel state transitions, not copied local validators.
- `./gradlew test` passes.
- `./gradlew lintDebug` passes, or every failure is listed with a reason and follow-up plan.
