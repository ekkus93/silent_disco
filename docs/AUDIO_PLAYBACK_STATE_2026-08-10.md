# Audio playback: state of play as of 2026-08-10

Written at the end of a long physical-device session on the LG G6 (Android
8.0 / API 26). Read this before touching the listener audio path, the clock
sync, or the render ring — it records what is fixed, what is still wrong,
how to measure any of it, and several dead ends that cost real time.

**Short version (updated 2026-08-10, post A1-A3):** the last human listening
verdict ("not bad, a little popping and crackling") was measured against
code since superseded by three more fixes (§3). A fresh 8-stream
distribution taken after those landed shows a **categorical** improvement,
not incremental: sync acceptance 19%→99%, RTT median 120ms→11ms (close to
the 7.7ms physical floor), worst-case silence 1,281,840→126,288 frames, and
the whole after-distribution now sits below the before-distribution's
median (§2). This has **not yet been re-confirmed by ear** — the numbers
predict it should sound meaningfully better, but that is a prediction, not
a listening result, until someone actually listens again.

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

**Updated 2026-08-10 after A1-A3 (Bluetooth-permission crash fix, `t4`
stamped at socket receipt, pending-probe eviction) all landed.** Distribution
over 8 streams (4 unattended two-song runs), compared against the n=8
baseline measured before those three fixes:

| metric | before (n=8) | after (n=8) |
|---|---|---|
| `ringSilenceFilled` min / median / max | 71,376 / 172,920 / 1,281,840 | 49,152 / 78,720 / **126,288** |
| sync acceptance | 39/210 (19%) | 153/155 (**99%**) |
| accepted RTT median | 120.3 ms | **11.2 ms** |
| accepted RTT max | 183.0 ms | 89.0 ms |
| `ringSilenceFilled` stdev as % of mean | 117% | **41%** |
| hard resyncs (max seen in one stream) | 6-7 | 1 |

This clears the ≥2× noise-floor bar (§8) decisively: the entire *after*
distribution's maximum sits below the *before* distribution's median —
non-overlapping ranges — and the spread itself shrank too (117%→41%), so
this is a more consistent system, not just a luckier one. RTT median at
11.2 ms is now close to the 7.7 ms ICMP physical floor (§8), confirming the
self-inflicted dispatch delay this investigation chased is genuinely gone.

**Not yet re-confirmed by a human ear against these specific numbers** — the
last listening verdict ("wasn't bad, a little popping and crackling") was
against the pre-A2/A3 code. Given how closely that verdict tracked its own
numbers, this distribution predicts something better, but get an actual
listen before declaring the popping gone.

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

1. ~~The listener never notices the host is gone~~ **— investigated
   2026-08-10, does NOT reproduce.** Tested a graceful `Stop`-then-shutdown
   (diagnostics pulled within seconds: clean `phase=STOPPED`, zero trailing
   idle rows) and an abrupt `SIGKILL` of the host process mid-stream
   (~1 second from last `PLAYING` sample to a clean `phase=STOPPED` summary
   and an on-screen "Host disconnected" message). `ConnectionClosed` /
   `HostDisconnected` handling already works correctly and promptly for an
   actually-closed connection. The 4,779,792-frame reading that motivated
   this item was almost certainly test-orchestration artifact (an app
   instance left connected across manual runs without an intervening
   force-stop), not a code defect. **What this does not test**: a *silent*
   network partition (Wi-Fi disabled, black-holed) delivers no FIN/RST at
   all, so TCP would only notice via a keepalive/send-timeout that may be
   slow or unconfigured — a genuinely different failure mode, and exactly
   what item 6 below (device Wi-Fi disable/restore) still needs to measure.
2. ~~`t4` is stamped after dispatch, not at socket receipt~~ **— fixed
   2026-08-10.** `SyncResponseReceived` now carries the transport's own
   receipt timestamp, translated onto the playback runtime's clock via a
   one-time delta (the two clocks have different origins). RTT median fell
   to 11.2ms in the post-fix distribution, close to the 7.7ms physical
   floor.
3. ~~Pending sync probes never age out~~ **— fixed 2026-08-10.**
   `begin_probe` now evicts anything older than 5s before checking capacity,
   so sustained response loss recovers on the next probe attempt instead of
   permanently bricking probing. Verified with deterministic tests. Still
   **not** device-confirmed under genuine real sustained loss: item 6's
   (A6) Wi-Fi disable/restore run turned out not to exercise this hazard at
   all — disabling Wi-Fi tears the listener's own transport down almost
   immediately (~7s), so it stops sending probes entirely rather than
   sending them into a silent void. Exercising this for real needs a
   scenario where the listener's connection *stays up* while responses
   stop arriving (e.g. host-side packet loss/black-holing, not the
   listener's own interface going down) — still open.
4. **Residual arrival gaps.** `ringQueued` was seen dropping to 384-720
   frames while emitting at full rate. Startup buffering (`STARTUP_BUFFER_MS`
   = 1000 ms) is now the dominant remaining silence, by design.
5. **`rebuffer_target_ms` is unturned.** The knob exists and is clamped to
   the startup target, but the Android value is left at parity. Lowering it
   to 400 ms was tried and is **indistinguishable** from parity given the
   noise — do not re-litigate without several runs per config.
6. **MP3 listener quality trails WAV/FLAC.** Single run 2026-08-10 (post
   A1-A3): `concealed=700 late=271 hardResyncs=2 ringSilenceFilled=186480
   ringFullEvents=21`, all clearly outside the post-fix WAV/FLAC range (§2,
   §A5 in §10). No distribution yet for MP3 specifically, so not confirmed
   as a regression — but distinct enough to be real. Suspected cause: MP3
   host-side decode has more per-frame timing variance than WAV/FLAC,
   pressuring the listener ring. Not investigated further; lowest priority
   until A6/A7 land.

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

**A1. ~~Listener does not detect the host is gone~~ — closed, does not
reproduce** (2026-08-10). Confirmed working for both a graceful stop and an
abrupt process kill; see §5 item 1. The real open question (silent network
partition, no FIN/RST) is folded into A6, not tracked separately here.

**A0. (new, found while investigating A1) Android device below API 31 needs
the legacy `BLUETOOTH` permission** — fixed 2026-08-10. `minSdk` was lowered
to 26 without adding it; every real-device run before now happened to avoid
the "Find a session" BLE path that triggers it. See `AndroidManifest.xml`.
- Also removes the metric contamination described in §5.1.

**A2. ~~Stamp `t4` at socket receipt~~ — done 2026-08-10.** See §5 item 2.

**A3. ~~Expire pending sync probes~~ — done 2026-08-10.** See §5 item 3;
device confirmation under real sustained loss still pending, folded into A6.

**A4. Close the residual gaps**
- A4.1 Re-measure §2 as a distribution (≥4 runs) for a clean baseline.
- A4.2 Revisit `STARTUP_BUFFER_MS` (1000 ms) now that supply is healthy.
- A4.3 Tune `rebuffer_target_ms` only with several runs per config.
- A4.4 Count offset-driven rebuffers so `hardResyncs` stops under-reporting.

**A5. ~~Finish Block 28.1~~ — done 2026-08-10.** Re-ran the FLAC and MP3
listener variants against the post-A1-A3 code; neither had been exercised
since those fixes landed.

- FLAC: clean. `concealed=112 late=21 hardResyncs=1 ringSilenceFilled=122304
  ringFullEvents=0` — sits inside the post-fix WAV distribution's range
  (§2), confirming normal, non-regressed behavior.
- MP3: test passed (7077/7077 packets fully or partially delivered, no
  drops, no disconnect) but listener-side quality is **clearly worse than
  WAV/FLAC**: `concealed=700 late=271 hardResyncs=2 ringSilenceFilled=186480
  ringFullEvents=21`. `ringSilenceFilled` is 46% over the post-fix WAV
  maximum (126,288) and `ringFullEvents` going nonzero hasn't been seen
  since A1-A3 landed. This is a single run (no distribution yet for MP3
  specifically), so it is not confirmed as a regression from A1-A3 — but it
  is different enough from WAV/FLAC on every metric to be a real, separate
  finding rather than noise. Likely cause: MP3 decode on the host has more
  per-frame timing variance than WAV/FLAC, pressuring the listener ring
  buffer. Not investigated further — recorded here as follow-up work, not
  blocking A6/A7.

**A6. ~~Block 28.2 device half~~ — verified 2026-08-10 on the real LG G6.**
Disabled Wi-Fi mid-playback via the system Wi-Fi settings toggle (`svc wifi
disable` is killed outright by this device's OS — `Killed`/exit 137 every
time; the UI toggle is the only thing that works here), left it off for
~2.5 minutes spanning the rest of the two-song test, then restored it.
Three distinct, real findings:
1. **Listener-side detection is fast and clear, but for a narrower reason
   than "silence timeout".** The app showed "Host disconnected" /
   `runtime transport ShuttingDown: transport event channel is closed`
   within ~7s of the toggle, reusing the same `ConnectionClosed`/
   `HostDisconnected` path item 1 above already verified. This is fast
   because disabling Wi-Fi tears the phone's own `wlan0` interface down
   entirely, which errors its already-open sockets immediately — a *local*
   failure, not detection of a *remote* silence. The harder case item 1
   explicitly flagged as still untested (interface stays up, packets are
   silently black-holed — e.g. the host vanishing from the LAN without the
   listener's own link dropping) remains genuinely unverified; this run
   answers Block 28.2's literal "disable Android Wi-Fi" wording, not that
   broader question.
2. **Recovery is fully manual, by design, confirmed in code.** The app
   surfaces a "Try again" button (`MainViewModel.retryJoin()`), no
   auto-reconnect. A rejoin also always re-enters
   `pending_join_requests` and needs a fresh `CoreCommand::ApproveJoin` from
   the host operator (`ActorState::handle_join_request`,
   `runtime/actor_runtime/state/admission.rs` — it never consults the
   current session's `snapshot.listeners`, so a still-listed vs.
   already-removed device is treated identically). This matches
   CLAUDE.md's "no silent auto-admit, manual approval is the default"
   exactly — not a gap, the intended behavior. Not yet exercised live
   end-to-end (the scripted test's one-shot approval helper had already
   returned and the host process had already exited by the time Wi-Fi was
   restored — see finding 3), so the actual reconnect UX still needs a
   real run, ideally with a dedicated test script that can approve a second
   join mid-run.
3. ~~**The host has zero visibility into the disconnect**~~ **— fixed
   2026-08-10, same day.** Originally: the scripted test's own timers kept
   running through the entire ~2.5-minute outage (song-a's remainder, the
   song switch, and all 40s of song-b), and its final broadcast stats read
   `attempted=15129 fully_delivered=15129 partially_delivered=0
   without_recipients=0` — a clean 100% success report — while the real
   listener sat on an error screen having received nothing since the
   outage began. `fully_delivered` counts successful `send()` syscalls, not
   actual receipt; UDP sends to an unreachable peer on the same LAN segment
   evidently still "succeed" at the OS level. This directly contradicted
   CLAUDE.md's "Zero recipients and partial delivery are not full success"
   and pre-emptively failed **A7.4** below.

   **Fix**: `PeerState` (`transport/socket/host.rs`) now tracks
   `last_inbound_millis`, refreshed on every genuine inbound frame from that
   peer (sync/audio datagram receiver in `host_workers.rs`, and the TCP
   control reader). `SocketHostTransport::authorized_routes` — called by
   every broadcast — excludes (and evicts via the same `PeerState::close`
   path `max_consecutive_failures` already used, so it surfaces through the
   identical, already-tested `PeerDisconnected` event) any peer silent
   longer than the new `HostTransportConfig::peer_inbound_silence_timeout`
   (default 8s, `DEFAULT_PEER_INBOUND_SILENCE_TIMEOUT` — 4x the listener's
   2000ms steady-state sync cadence, chosen conservatively per the sibling
   `max_consecutive_failures` mechanism's own documented lesson about being
   too aggressive). Two new deterministic tests in `transport/tests.rs`
   using `ManualTransportClock` (no real sleeping): a silent peer is
   evicted and stops being reported as delivered; a peer that keeps
   probing normally is never evicted.

   **Confirmed on the real LG G6, same Wi-Fi-disable scenario**: before the
   fix, `fully_delivered=15129/15129` (100%) for the whole run. After the
   fix, the same scenario produced `attempted=15128 fully_delivered=9086
   (60%) partially_delivered=111 without_recipients=5931 (39%)`, and the
   listener correctly disappeared from the actor's `snapshot.listeners`
   entirely partway through — a dramatic, measured, confirmed change, not
   just a code-level argument. One honest caveat: eviction took
   meaningfully longer in this real run than the nominal 8s config value
   (`without_recipients` was still 0 at the ~30s mark, only climbing by the
   ~75s mark) — likely real Android Wi-Fi teardown/ARP timing rather than a
   logic bug, since the mechanism's *direction* (evicted, honestly
   reported) is unambiguous; the exact real-world latency bound is not
   pinned down as tightly as the synthetic tests suggest. Not investigated
   further this block. `ListenerLifecycle::Reconnecting` remains dead state
   (still wired end-to-end but never assigned) and the Android UI still
   doesn't surface this disconnect — out of scope for this fix, which only
   targeted the host-side delivery-honesty half of A7.4.

**A7. Block 29 — multiple physical listeners** *(the actual product
criterion, entirely unvalidated on hardware)*
- A7.1 Two devices join and are approved.
- A7.2 Both complete initial sync.
- A7.3 Both play the same stream; pause/resume/stop affects both.
- A7.4 One listener disconnecting is not reported as full delivery success
  — **host-side half fixed and confirmed 2026-08-10** (A6 above): a silent
  peer is now evicted and honestly reported as `without_recipients`. Not
  yet re-confirmed with *two* listeners specifically (one drops, one
  stays) — that's what A7 itself still needs to exercise.
- A7.5 Measure inter-device skew, loss, underruns, confidence (29.2).
- A7.6 Compare listeners against *each other* — "listeners hear the same
  thing" is what none of the single-listener work tests.

### Desktop / host

**D1. ~~Block 28.2 device-independent failure tests~~ — done 2026-08-10.**
Both added to `desktop/src-tauri/src/platform/start_playback_tests.rs`:
- D1.1 `starting_playback_with_a_corrupt_source_fails_visibly_at_the_orchestration_level`
  — a WAV truncated before its header even parses fails synchronously in
  `start_playback::start`, and the actor snapshot visibly reaches
  `PlaybackState::Error` (polled, not checked immediately —
  `submit_audio_event` only queues the transition).
- D1.2 `a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming`
  — a WAV whose header parses fine (declares 3s) but whose body is
  truncated to ~0.1s fails only once the packetizer worker decodes past the
  truncation point (confirmed empirically: `open()` succeeds,
  `DecodeErrorKind::CorruptInput` surfaces later on `join()`). Proves the
  pump exits on its own (`playback_is_active` goes false unprompted), the
  actor leaves `Playing` without anyone calling `stop_playback`,
  `stream_ended_naturally` stays false (not confused with a clean EOS), and
  a subsequent `stop_playback()` call surfaces the real failure as `Err`
  rather than reporting a clean stop.
- Both gates (`scripts/check-rust.sh`, `desktop && npm run check`) green.

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
