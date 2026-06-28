# Silent Disco — replies3.md

Prepared: 2026-06-28  
Responding to: `responses3(20).md`  
Scope: FIX5 clarification replies for Claude Code

## Summary decisions

Proceed with FIX5 as a **correctness-only hardening pass**.

For the two unresolved TODO decisions:

1. **P1.4 `zeroPeerBroadcastCount`**: choose **Option A — remove it entirely**.
2. **P2.3 BLE async failure tests**: choose **Option A — add internal test hooks to the concrete `BleDiscoveryService`**. Do **not** extract a new service interface in this pass unless compilation/testing proves the concrete class cannot be constructed at all.

---

## Q1 — Is FIX5 correctness-only, or should styling/visual polish be included?

FIX5 is intentionally **correctness-only**.

Do not add styling, typography, color, layout, or visual polish work in this pass. The earlier wording about styling was not the intended scope for FIX5. This pass exists to close the remaining hardening gaps:

- false success states;
- result objects being ignored;
- zero-recipient sends being treated as success;
- listener sync/playback state regressions;
- stale `buffered` / `playing` flags;
- join rejection delivery ordering;
- weak tests that do not exercise production behavior.

If separate UI polish is desired later, it should be handled in a separate pass after the correctness state machine is trustworthy.

Implementation rule: **do not broaden FIX5**. Keep changes targeted to the TODO.

---

## Q2a — P1.4: Remove or expose `zeroPeerBroadcastCount`?

Choose **Option A: remove `zeroPeerBroadcastCount` entirely**.

Rationale:

- The current FIX5 requirement is to make zero-listener broadcast behavior visible immediately.
- We do not need a cumulative counter to satisfy that requirement.
- Keeping a counter that is incremented but not surfaced creates dead diagnostic state.
- Wiring it into `summarizeMetrics()` would expand scope and requires deciding formatting, reset behavior, and whether it should be per-stream or per-session.
- The simple, correct behavior is: every zero-listener audio broadcast should set a visible message/diagnostic at the point it happens.

### Required edit

In `MainViewModel.kt`, inside `startHostStreamingLoop()`, delete:

```kotlin
var zeroPeerBroadcastCount = 0
```

and delete any increment like:

```kotlin
zeroPeerBroadcastCount += 1
```

Keep the zero-peer branch, but make sure it updates both host diagnostics and UI state:

```kotlin
result.peerCount == 0 -> {
    consecutiveAudioSendFailures = 0
    val message = "No connected listeners for audio broadcast"
    _uiState.value = _uiState.value.copy(lastError = message)
    diagnosticsStore.updateHost {
        it.copy(
            lastError = message,
            metricsSummary = summarizeMetrics(),
        )
    }
    refreshHostDiagnostics()
}
```

Acceptance:

- No `zeroPeerBroadcastCount` variable remains.
- Zero-peer audio broadcast is disclosed through `_uiState.lastError`.
- Zero-peer audio broadcast is disclosed through host diagnostics.
- Zero-peer audio broadcast does not stop local host preview.

---

## Q2b — P2.3: BLE async failure test: internal callbacks or fake service interface?

Choose **Option A: add internal test hooks to the concrete `BleDiscoveryService`**.

Do **not** extract a new `BleDiscoveryService` interface in FIX5. That would be an architecture change beyond this focused cleanup pass.

The goal is to stop the current tautological test pattern where the test creates its own `MutableSharedFlow` and proves only that `MutableSharedFlow` works. The test should exercise the production service’s actual failure emission path, or at least a production helper used by the callbacks.

### Preferred implementation shape

In `BleDiscoveryService.kt`, factor failure emission into small private helpers and call those helpers from the Android callbacks and from internal test hooks.

```kotlin
private fun emitAdvertiseFailure(errorCode: Int) {
    val message = "BLE advertise failed with code=$errorCode"
    logger.w("ble.advertise", message)
    _failures.tryEmit(BleOperationFailure(BleOperation.ADVERTISE, message))
}

private fun emitScanFailure(errorCode: Int) {
    val message = "BLE scan failed with code=$errorCode"
    logger.w("ble.scan", message)
    _discoveredSessions.value = emptyList()
    _failures.tryEmit(BleOperationFailure(BleOperation.SCAN, message))
}
```

Then update the Android callbacks:

```kotlin
override fun onStartFailure(errorCode: Int) {
    emitAdvertiseFailure(errorCode)
}
```

```kotlin
override fun onScanFailed(errorCode: Int) {
    emitScanFailure(errorCode)
}
```

Add test hooks:

```kotlin
internal fun emitAdvertiseFailureForTest(errorCode: Int) {
    emitAdvertiseFailure(errorCode)
}

internal fun emitScanFailureForTest(errorCode: Int) {
    emitScanFailure(errorCode)
}
```

Use `internal`, not `public`. Do not add UI-facing behavior or debug-only production branches for this.

### Test expectation

The test should collect from the real `BleDiscoveryService.failures` flow and trigger the hook:

```kotlin
@Test
fun emitScanFailureForTest_emitsScanFailure() = runTest {
    val service = createBleDiscoveryServiceForTest()
    val events = mutableListOf<BleOperationFailure>()
    val job = launch {
        service.failures.take(1).toList(events)
    }

    service.emitScanFailureForTest(2)

    job.join()
    assertEquals(BleOperation.SCAN, events.single().operation)
    assertTrue(events.single().message.contains("BLE scan failed with code=2"))
}
```

```kotlin
@Test
fun emitAdvertiseFailureForTest_emitsAdvertiseFailure() = runTest {
    val service = createBleDiscoveryServiceForTest()
    val events = mutableListOf<BleOperationFailure>()
    val job = launch {
        service.failures.take(1).toList(events)
    }

    service.emitAdvertiseFailureForTest(3)

    job.join()
    assertEquals(BleOperation.ADVERTISE, events.single().operation)
    assertTrue(events.single().message.contains("BLE advertise failed with code=3"))
}
```

Adjust `createBleDiscoveryServiceForTest()` to match the current constructor. If the concrete service needs an Android `Context`, use Robolectric or the existing project test pattern for Android-dependent services.

### If concrete construction is genuinely blocked

Only if the concrete `BleDiscoveryService` cannot be constructed in JVM tests without a large Robolectric setup, do this fallback:

- Keep the internal hooks anyway.
- Add a small pure production helper for classification/message formatting.
- Test that helper.
- Add a TODO comment explaining that callback-to-flow integration requires Robolectric/instrumented coverage.

Do **not** extract a broad service interface just for this one test in FIX5.

---

## Implementation notes for Claude Code

### Keep FIX5 narrow

Do not use this clarification as permission to refactor the transport, BLE, or ViewModel architecture broadly. The desired outcome is a small patch that makes the current code truthful and testable enough.

### Avoid “done because tests pass”

The important behavior is production correctness:

- periodic listener resync must not knock a playing listener back to `SYNCING_CLOCK`;
- listener transport failure must transition listener UI to `ERROR`;
- join rejection should only remove the pending request after rejection delivery succeeds;
- zero-recipient sends must be visible;
- stale `buffered` / `playing` flags must be cleared on error/disconnect;
- tests must exercise production helpers or real ViewModel/service behavior.

### Validation

After implementation, run:

```bash
./gradlew test
./gradlew lintDebug
```

Also run focused greps:

```bash
grep -R "zeroPeerBroadcastCount" app/src/main/java/com/ekkus/silentdisco -n
grep -R "manualResync()" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
grep -R "broadcastControl\|broadcastSyncResponse\|broadcastAudio" app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt
```

Expected:

- no `zeroPeerBroadcastCount`;
- no host-side playback path calls listener `manualResync()`;
- broadcast result call sites either consume `SendAllResult` or have a narrow comment explaining why the result is intentionally non-blocking.
