# Silent Disco — Claude Code Questions on FIX3 Spec

Prepared: 2026-06-28  
Spec reviewed: `docs/SILENT_DISCO_FIX3_SPEC.md`  
TODO reviewed: `docs/SILENT_DISCO_FIX3_TODO.md`

---

## Q1 — File naming

The spec and TODO files on disk are `SILENT_DISCO_FIX3_SPEC.md` and `SILENT_DISCO_FIX3_TODO.md`. The original request referred to them as `SILENT_DISCO_CODE_REVIEW3_*`. Is `FIX3` the canonical name for this pass? No action needed if so — just want the naming confirmed before committing work under a heading.

---

## Q2 — "Styling" vs hardening (most important)

The original request described this as "adding some more styling." Both documents are entirely about correctness, silent-failure elimination, and state-machine hardening — a direct continuation of CODE_REVIEW2. There is no visual styling, typography, color, layout, or UX polish work anywhere in the spec or TODO.

**Is there a separate styling/UI-polish spec that should also be reviewed? Or was "styling" the wrong word for this pass?**

If there is a styling spec, I need to see it before starting. If not, I'll proceed with FIX3 as a pure correctness/hardening pass.

---

## Q3 — `ControlMessage.StreamStart` fields (P0.4)

P0.4 tells me to pass `message.sampleRate` and `message.channels` from the `StreamStart` message into `startTransportListenerPlayback()` as an `AudioFormatSpec`. The spec assumes these fields exist on `ControlMessage.StreamStart`.

**Do `sampleRate` and `channels` exist on `StreamStart` today, or do I need to add them to the protocol model first?**

If they don't exist, I'll need to add them to `ControlMessage.StreamStart` (and update any serialization/deserialization) before P0.4 can be implemented.

---

## Q4 — `refreshHostDiagnostics(streamState = ...)` overload (P0.2, P0.5, P0.11, P0.13, P0.15)

Several new failure helpers in the spec call `refreshHostDiagnostics(streamState = PlaybackState.ERROR)` with a named `streamState` parameter. The existing function may not accept this parameter.

**Does `refreshHostDiagnostics()` currently accept a `streamState` named parameter, or do I need to add that overload?**

If it needs to be added, I'll add it as part of P0.2.

---

## Q5 — `summarizeMetrics()` in new failure helpers (P0.2, P0.15, P1.1, P1.3, P1.4)

The new failure helpers (`handleListenerPlaybackEngineFailure`, `handleHostPlaybackEngineFailure`, `handleHostControlFailure`, `propagateListenerPlaybackState`) all reference `summarizeMetrics()` when updating diagnostics snapshots.

**Does `summarizeMetrics()` already exist in `MainViewModel`, or is it a new helper I need to add?**

If it needs to be added, what should it summarize (sync stats, buffer depth, packet loss)?

---

## Q6 — `PlaybackEngine` interface for testability (P2.5)

P2.5 suggests introducing a `PlaybackEngine` interface and injecting it into `MainViewModel` to make write-failure tests possible without spinning up a real `AudioTrack`. This is a larger architectural change than the other P2 items — it touches the `MainViewModel` constructor signature and all test setup.

**Should I add the `PlaybackEngine` interface as part of FIX3, or defer it?**

If deferred, the P2.5 write-failure tests (listener/host playback write failure → visible error state) cannot be unit-tested without Android instrumentation. I can still implement the runtime behavior and cover it with a comment noting testability is pending the interface.

---

## Q7 — `TcpServerChannel.channelName` for logging in `sendAll()` (P0.13)

P0.13 shows a log line inside the new `sendAll()` implementation:

```kotlin
logger.w("transport.$channelName.send", "Failed to send to peer: ${error.message}")
```

This assumes `TcpServerChannel` has both a `logger` reference and a `channelName` property.

**Do these exist on `TcpServerChannel` today, or do I need to add them?**

If `channelName` doesn't exist, I'll use a fallback tag like `"transport.tcp.send"`.

---

## Q8 — `latestDecodedAudio` in P0.3

P0.3 tells me to start the playback engine inside `startListenerPlaybackSimulation()` using:

```kotlin
val playbackFormat = latestDecodedAudio?.format ?: AudioFormatSpec()
```

**Does `latestDecodedAudio` (or an equivalent decoded audio reference) exist in `MainViewModel`, or is `AudioFormatSpec()` the right default for the simulation path?**

If the simulation path always uses generated/fake PCM packets, the default `AudioFormatSpec()` may be sufficient and `latestDecodedAudio` may not be needed here.

---

## Q9 — `stopAdvertising()` vs `stop()` on BleDiscoveryService (P0.11)

P0.11 says that if Wi-Fi Direct startup fails after BLE advertising has already started, the code should call `bleService.stopAdvertising()`. However, `BleDiscoveryService` currently exposes `stop()` (which stops both advertising and scanning) rather than a dedicated `stopAdvertising()`.

**Should I add a `stopAdvertising()` method to `BleDiscoveryService`, or is calling `stop()` acceptable here since scanning won't have started at that point in host startup?**

Calling `stop()` at that point should be safe (no scan is running during host startup), but I want to confirm rather than assume.

---

## Q10 — Scope of `typealias OboePlaybackEngine` (P0.1)

P0.1 introduces `typealias OboePlaybackEngine = AudioTrackPlaybackEngine` as a low-churn migration aid, but also says the acceptance criterion is that `MainViewModel` uses `AudioTrackPlaybackEngine` directly. The typealias would then exist only in `PlaybackScheduling.kt` with no callers.

**Should I update all callers (MainViewModel, test files) to use `AudioTrackPlaybackEngine` directly and leave only the deprecated typealias as a marker — or keep the typealias as the primary name until a separate cleanup pass?**

I'll default to updating all callers and keeping the deprecated typealias as a single-line comment trail, unless you'd prefer otherwise.
