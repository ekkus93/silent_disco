# Audio playback: state of play as of 2026-08-10

Written at the end of a long physical-device session on the LG G6 (Android
8.0 / API 26). Read this before touching the listener audio path, the clock
sync, or the render ring — it records what is fixed, what is still wrong,
how to measure any of it, and several dead ends that cost real time.

**Short version:** audio is now *acceptable but not clean*. A human listener
describes it as "not bad, a little popping and crackling". The catastrophic
failures (streams stalling ~26 seconds before any sound, whole tracks
scratchy end to end) are gone. What remains is intermittent: roughly 14% of
playback seconds carried some defect in the worse of two back-to-back runs,
two of them clearly audible.

---

## 1. How the audio path works today

Host (desktop) → network → listener (Android):

1. **Desktop** decodes and packetizes (`rust/silent-disco-core/src/audio/packetizer.rs`,
   5 ms packets = 240 frames at 48 kHz). A pump thread
   (`desktop/src-tauri/src/platform/playback_streamer.rs`) paces sends against a
   1000 ms send-ahead horizon and hands frames to the transport worker
   (`desktop/src-tauri/src/platform/host_transport.rs`), which broadcasts over UDP.
2. **Listener transport** (`rust/silent-disco-core/src/transport/socket/listener.rs`)
   receives on its own threads into a bounded queue.
3. **Audio never crosses into Kotlin.** The transport submits each received
   datagram straight into the playback runtime
   (`FfiListenerTransportHandle::attach_playback` / `forward_audio` in
   `rust/silent-disco-ffi/src/listener_transport/handle.rs`). Kotlin sees
   control-plane events only. **This is load-bearing — see §4.1.**
4. **Scheduler** (`rust/silent-disco-core/src/audio/scheduler.rs`) orders,
   conceals, and schedules against host presentation time mapped through the
   clock offset. A pump writes due frames into the render ring.
5. **Render ring → Oboe.** A C++ callback (`app/src/main/cpp/OboeOutputAdapter.cpp`)
   reads the ring through the narrow C ABI. One stream is opened per
   *session* and **rebound** per track, never reopened (§4.4).

Clock sync is NTP-style four-timestamp exchange; the listener probes, the
host echoes. Playback cannot start until one sample is accepted, and
packets arriving before lock are deliberately discarded.

---

## 2. Current measured quality (LG G6, parity config)

From two back-to-back listening runs on identical code — note how much they
differ, which is the single most important thing to internalise:

| | run 1 | run 2 (heard by a human) |
|---|---|---|
| song-a concealed / late | 2 / 0 | 168 / 92 |
| sync acceptance | 31/31 (100%) | 35/40 (87%) |
| `PLAYING` seconds with any underrun/silence | ~0 | 10 of 71 |
| hard resyncs | 0 | 0 |

Human verdict on run 2: *"wasn't bad, a little popping and crackling"* —
which tracks its numbers closely. **The counters are a usable proxy for
perception**, so you can iterate without a listener present, but confirm by
ear before declaring anything fixed.

Distribution at this config (n=8 streams, before the last fix):
`ringSilenceFilled` min 71,376 / median 172,920 / max 1,281,840.

---

## 3. What was fixed in this session

Each was verified on the real device, not just reasoned about.

| Fix | Commit | Evidence |
|---|---|---|
| `minSdk` 29 → 26 so the LG G6 can run at all | earlier | lint clean, real install/launch |
| Blocking UDP sends (200-700 ms) → `SO_SNDTIMEO` | `a37e190` | sends fell to ~5-7 ms |
| `WouldBlock` send timeout no longer counted as a peer failure | `a37e190` | listener stopped being dropped mid-stream |
| Pause/resume presentation-timeline drift | `a37e190` | `queue_overflows` flat across pause/resume |
| Start-of-stream burst overflowing the broadcast queue | `a37e190` | queue capacity 64 → 256; overflows 59 → 0 |
| Mid-session track switching (actor rejected it outright) | `a37e190` | full two-song run completes |
| Oboe granted-config validation + retained diagnostics | `484b570` | exposed `sharing=Shared` |
| Oboe stream **rebound** per track instead of reopened | `11c06d3` | `opens=1 rebinds=1` |
| **Audio submitted inside Rust, not via Kotlin** | `11c06d3` | sync acceptance 6% → 90%, RTT 143-177 → 25-40 ms |
| **`recv_event(&self)` so a poll cannot delay a sync probe** | `c5dc8b2` | acceptance 19% → 98%, worst-case silence 1,281,840 → 121,968 |

The last two are the substantive ones; everything before them was necessary
but not sufficient.

---

## 4. Things you must not undo

### 4.1 Audio must not round-trip through Kotlin
Surfacing one `AudioReceived` event per packet (200/s), each copying the
payload out of Rust and straight back in via `submitPacket`, saturates the
single Kotlin event-loop coroutine. Control traffic — including sync
responses — queues behind it, so `t4` is stamped ~140 ms late and that delay
is counted as network RTT. That inflated RTT then fails the estimator's
200 ms acceptance gate. This was the dominant defect.

### 4.2 `recv_event` takes `&self` deliberately
`poll_event` used to hold the handle's lock across its whole blocking
receive, and the send methods need that same lock. A probe stamped `t1`,
then waited — again counted as network latency. Now polls and sends take
concurrent read guards on an `RwLock`; only teardown takes the write guard.
`mpsc::Receiver` is `Send` but **not `Sync`**, so each listener's receiver
has its own mutex, deliberately separate from the send path.
Reproduction test: `a_sync_probe_is_not_blocked_by_a_concurrent_poll`
(measured 900 ms before the fix).

### 4.3 `stopPlayback` order is deliberate
`runtime.stop()` **then** `nativeOboeClose()`. The drain runs through the
still-live Oboe callback; closing first truncates every stream's tail. See
`await_ring_drain`, which documents exactly this.

### 4.4 One Oboe stream per session, rebound per track
Closing and reopening was tried; the device never grants `Exclusive` anyway,
but reopening churn is real and the rebind is architecturally right — the
output device belongs to the connection, not to one track.

---

## 5. Known remaining problems, in priority order

1. **The listener never notices the host is gone.** After the host shuts
   down, the stream stays open and silence-fills indefinitely — burning CPU
   and radio, with the UI still showing a live stream. It also *contaminates
   the metrics*: one manual run logged 4,779,792 silence frames of which
   ~4.7 M was post-run idle. **Highest value, and cheap.**
2. **`t4` is stamped after dispatch, not at socket receipt.** The listener's
   sync receiver already captures an accurate `received_at`
   (`socket/listener.rs`), but `map_event` discards it and Kotlin substitutes
   `runtime.nowMs()`. Now a smaller effect, but the last known RTT
   contaminant. Watch the clock origins: `PumpClock` and the transport clock
   have different bases.
3. **Pending sync probes never age out** (`sync/estimator.rs`). The map only
   shrinks in `observe_response`, so 64 lost responses permanently brick
   `beginSyncProbe`, after which the Kotlin loop silently stops sending for
   the rest of the stream. Latent (peak observed ~12) but it converts a bad
   stall into *permanent* silence.
4. **Residual arrival gaps.** `ringQueued` was seen dropping to 384-720
   frames while emitting at full rate. Startup buffering (`STARTUP_BUFFER_MS`
   = 1000 ms) is now the dominant remaining silence, by design.
5. **`rebuffer_target_ms` is unturned.** The knob exists and is clamped to
   the startup target, but the Android value is left at parity. Lowering it
   to 400 ms was tried and is **indistinguishable** from parity given the
   noise — do not re-litigate without several runs per config.

---

## 6. Not yet tested: multiple simultaneous listeners

Everything above is **one listener**. Multi-client is unvalidated on
physical devices. What is known:

- Two Android **emulators** streamed together successfully once (see
  `memory.md`, 2026-08-08), which found and fixed a real device-identity
  bug — every install previously shared one hardcoded `device_id`, so the
  host only ever saw one listener.
- That run is **not** acceptance for Block 29: emulators are not physical
  devices, and neither emulator completed a clock-sync exchange.
- The host's broadcast queue is now 256 frames and its per-peer send path
  has a 5 ms write timeout; neither has been exercised with two real phones.

Expect to re-measure everything in §2 per-listener, and to compare listeners
against each other — the project's success criterion is that listeners hear
the *same* thing, which none of the single-listener work here tests.

---

## 7. How to run and measure (this tooling saves hours)

### Manual device test
```
cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml \
  manual_real_android_listener_plays_a_song_change -- --ignored --nocapture
```
Prints a connection payload, waits up to 8 minutes for a real join, then
plays song-a (15 s, 1 s pause, 20 s) and song-b (40 s).
FLAC/MP3 variants: `..._plays_flac`, `..._plays_mp3` (need `ffmpeg`).

### Listener-side diagnostics — **logcat does not work on this device**
`adb logcat` is entirely dead on the LG G6 (`ro.logdumpd.enabled=0`, every
buffer empty), and the in-app diagnostics screen **cannot be opened
mid-stream without tearing the session down**. So the listener writes a
durable file next to its debug WAV captures:

```
adb pull /sdcard/Android/data/com.ekkus.silentdisco/files/manual-listener-diagnostics.log
```
Per-second `sample` rows plus a per-stream `summary`, including the Oboe
granted config (which survives stream close). Debug PCM captures land in the
same directory as `manual-listener-<streamId>.wav`.

**Force-stop the app promptly after a manual run**, or trailing idle
silence-fill (§5.1) makes the totals meaningless.

### Automating repeat runs
A driver that starts the test, auto-connects the phone over `adb`, waits,
and repeats is what made distributions practical. Pattern: poll the test log
for the payload, escape it (`{`, `}`, `"`, `,` must be backslash-escaped or
`adb shell input text` silently fails on this device), then drive the UI by
`uiautomator dump` + exact-text bounds lookup rather than fixed coordinates.

---

## 8. Pitfalls that cost time in this session

- **Single-run comparisons are worthless here.** Measured noise floor at one
  fixed configuration (n=8): `ringSilenceFilled` spans 18×, standard
  deviation 117% of the mean; `concealed` spans 330×. **Differences under
  ~2× are indistinguishable from noise.** Compare distributions over ≥4 runs
  per config. Four separate hypotheses in this session were confidently
  wrong because they generalised from one run.
- **A debug capture cannot show missing time.** The WAV contains only frames
  actually written to the ring, so a dropout is *absent duration*, not
  corrupted samples. Compare captured duration against expected duration; a
  zero-run/discontinuity scan will look clean during severe dropouts.
- **Reconcile two independent data sources before believing anything.**
  Every correct conclusion here came from cross-checking (WAV against
  counters, ICMP against the app's own RTT). Every wrong one came from
  reasoning inside a single view.
- **The network was never the problem.** ICMP to the host during active
  streaming: 7.7 ms average, 0% loss (idle is *worse*, ~16.7 ms — normal
  Wi-Fi power-save). Any app-measured RTT far above that is self-inflicted.
- **`hardResyncs` does not count every rebuffer.** Only the
  concealment-bound path increments it; the offset-jump path's result is
  discarded in `observe_sync_response`, so those rebuffers are invisible.
- **Host-side "listener has not completed a sync exchange" is a red
  herring.** `AudioEvent::SynchronizationUpdated` is defined but never
  submitted by the desktop, so that field is never populated.
- When editing files where host and listener share method names, anchor
  edits on the enclosing type — blind string replacement hit the host trait,
  host impls, and host constructor in three consecutive attempts.

---

## 9. Suggested order of work when resuming

1. Fix §5.1 (host-gone detection) — cheap, removes a real robustness gap and
   un-contaminates the metrics.
2. Fix §5.2 (`t4` at receipt) and §5.3 (probe expiry) — both small, both
   remove known hazards.
3. Re-measure §2 as a distribution (≥4 runs) to establish a clean baseline.
4. Then move to multi-listener (§6), which is the actual product criterion
   and is entirely unvalidated on real hardware.

---

## 10. Remaining work, split Android-first

Android (listener) work is listed first because that is where every
remaining audio defect lives — the desktop host was measured clean
throughout (exact packet cadence, zero queue overflows, full delivery).

### Android / listener

**A1. Listener does not detect the host is gone** *(highest value, small)*
- A1.1 Treat host shutdown / transport closure as end-of-stream: stop the
  runtime and close the Oboe stream instead of silence-filling forever.
- A1.2 Surface it in the UI as disconnected rather than still streaming.
- A1.3 Regression test: `ConnectionClosed` / `HostDisconnected` must end
  playback.
- Also removes the metric contamination described in §5.1.

**A2. Stamp `t4` at socket receipt, not after dispatch**
- A2.1 Carry the receiver's existing `received_at` through
  `SyncResponseReceived` (`map_event` currently discards it).
- A2.2 Reconcile clock origins — `PumpClock` and the transport clock have
  different bases; share one or translate by a one-time delta.
- A2.3 Kotlin uses the carried value instead of `runtime.nowMs()`.
- A2.4 Test that a queued response does not inflate measured RTT.

**A3. Expire pending sync probes** *(latent but severe)*
- A3.1 Age-evict in `sync/estimator.rs`; today the map only shrinks on a
  matching response.
- A3.2 Test that probing survives >64 lost responses instead of bricking
  permanently.

**A4. Close the residual gaps**
- A4.1 Re-measure §2 as a distribution (≥4 runs) for a clean baseline.
- A4.2 Revisit `STARTUP_BUFFER_MS` (1000 ms) now that supply is healthy.
- A4.3 Tune `rebuffer_target_ms` only with several runs per config.
- A4.4 Count offset-driven rebuffers so `hardResyncs` stops under-reporting.

**A5. Finish Block 28.1** — re-run the FLAC and MP3 listener variants, which
have not been exercised since any of these fixes.

**A6. Block 28.2 device half** — disable Android Wi-Fi mid-playback, restore
it, and verify the disconnect/recovery policy. A1 likely changes this
behaviour, so do it after A1.

**A7. Block 29 — multiple physical listeners** *(the actual product
criterion, entirely unvalidated on hardware)*
- A7.1 Two devices join and are approved.
- A7.2 Both complete initial sync.
- A7.3 Both play the same stream; pause/resume/stop affects both.
- A7.4 One listener disconnecting is not reported as full delivery success.
- A7.5 Measure inter-device skew, loss, underruns, confidence (29.2).
- A7.6 Compare listeners against *each other* — "listeners hear the same
  thing" is what none of the single-listener work tests.

### Desktop / host

**D1. Block 28.2 device-independent failure tests** *(no phone needed)*
- D1.1 Corrupt source fixture fails visibly at the `start_playback`
  orchestration level.
- D1.2 A host source read failure does not claim continued normal streaming.
- Only decoder-unit-level coverage exists today.

**D2. Wire up `AudioEvent::SynchronizationUpdated`** — defined but never
submitted, so the host's per-listener sync diagnostics are always empty and
read as "listener has not completed a sync exchange". Actively misleading
during debugging.

**D3. Host-side multi-listener capacity (29.3)** — record CPU, memory, queue
high-water marks and delivery failures with every available listener. The
256-frame broadcast queue and the 5 ms per-peer write timeout have never
been exercised with two real phones.

**D4. Block 28.3 bookkeeping** — audit that each defect fixed this session
has a regression test, tick the 28.1 boxes now substantively satisfied for
WAV, and record device results.

### Suggested order

A1 → A2 → A3 → A4.1 (clean baseline) → A5 → D1 (fills in while no device is
needed) → A6 → A7 (+ D2, D3 alongside multi-listener).
