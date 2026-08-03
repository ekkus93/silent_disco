# Playback Review Fixes TODO

Fixes for defects found by two adversarial code reviews of the listener
playback Rust migration, run 2026-08-03 after
`docs/SILENT_DISCO_LISTENER_PLAYBACK_RUST_MIGRATION_TODO.md` was completed.

Read first:

- `docs/SILENT_DISCO_LISTENER_PLAYBACK_RUST_MIGRATION_TODO.md` — what was
  built and why; its "Verified requirements" table (R1–R11) still holds.
- `memory.md` entries dated 2026-08-02 and 2026-08-03.
- `CLAUDE.md` — several findings below are direct violations of its rules on
  hidden failures and competing owners.

## Status of the two paths

| Path | State |
|---|---|
| Manual connect (`ManualListenerTransportController`) | Migrated, **device-validated** 2026-08-03 (39.96s, zero gaps). Defects below still apply. |
| Discovered session / BLE + Wi-Fi Direct (`MainViewModelRustListener`) | Migrated but **never device-tested**. Item 1 means it is almost certainly **silent in its current state**. |

Two reproduction assets already exist:

- `an_outage_wider_than_the_reorder_window_does_not_permanently_wedge_the_stream`
  in `rust/silent-disco-core/src/audio/scheduler_tests.rs` — currently
  `#[ignore]`d and **failing on purpose**. It is the acceptance test for
  item 2. Un-ignore it when fixed.

Work items are ordered by severity. Items 1–4 are correctness or
core-purpose defects; do those before anything else.

---

## 1. CRITICAL — the discovered-session path never syncs, so it can never play

`MainViewModelRustListener.kt` `handleTransportSyncResponse` routes the
four-timestamp exchange into the legacy Kotlin `applySyncResponse` /
`ListenerSyncController`. `beginSyncProbe` / `observeSyncResponse` are called
**only** from `ManualListenerTransportController`. `PlaybackPump::tick`
returns `AwaitingSync` and releases zero frames until `sync_locked`, so on
this path Oboe opens, packets accumulate, and not one frame is ever written
to the ring. Verified by grep, not yet on a device.

- [x] 1.1 Route this path's sync through the runtime: on
      `SyncResponseReceived`, call `runtime.observeSyncResponse(...)` with t4
      from `runtime.nowMs()`.
- [x] 1.2 Send its probes from the runtime's clock. The outbound probe is
      currently stamped by `ListenerSyncController.newProbe()` using
      `SystemClock.elapsedRealtime()` — a different epoch from
      `runtime.nowMs()`. Register each probe with `beginSyncProbe` and send
      the same timestamp, as the manual path does.
- [x] 1.3 Decide what remains of `ListenerSyncController` on this path. It
      still feeds pre-stream connection-progress UI (the "synced" tick)
      before any runtime exists. Either keep it strictly for that and
      document it, or drive progress from `syncLocked` instead. **Do not
      leave two estimators both feeding playback.**
**1.3 resolution:** `ListenerSyncController` is kept **only** for the
pre-stream window, where no runtime exists yet but connection progress must
still show sync under way. The moment a runtime exists it is the sole
authority — both the probe and the response go through it, and nothing
derives playback timing from the Kotlin controller. `SyncSampleOutcome` now
carries the estimator's own `jitter_ms` and `confidence` so the UI reports
what the estimator actually computed instead of fabricating it.

- [ ] 1.4 Device-validate this path end to end. It has never been run. Note
      it cannot reach the desktop host (manual connect is the only route), so
      this needs a second Android device acting as host.

## 2. CRITICAL — an outage past the reorder window wedges the stream forever

During an outage the buffer is empty, so `missing_sequence_count()` is 0 and
the wide-hole skip never engages. Concealment advances `next_expected` by
`max_consecutive_concealed_packets`, then forces `AwaitingRebuffer`; the pump
re-arms into `Buffering`, where `poll` returns early and `next_expected`
freezes. When the host resumes far ahead, every packet exceeds
`max_reorder_window`, the buffer never fills, and nothing ever advances
`next_expected`. Permanent silence until the runtime is torn down.

Recoverable outage ceiling today is roughly
`(25 concealed + 64 window) × 20ms ≈ 1.8s`. The same defect means **a
listener joining an in-progress stream can never bootstrap** — the host is at
sequence 500, `next_expected` is 0, everything is rejected.

- [x] 2.1 Give `JitterBuffer` a way to resynchronise onto a far-ahead
      sequence rather than rejecting forever. Options to weigh: treat a
      run of `ReorderWindowExceeded` as a resync trigger; or expose an
      explicit `resynchronise_to(sequence)` the scheduler calls when it has
      been starved past a bound. Prefer an explicit, testable transition
      over a heuristic buried in `accept`.
- [x] 2.2 Make the mid-stream join case work: a fresh scheduler whose
      `next_expected` is 0 must adopt the first arriving sequence rather
      than demanding sequence 0.
- [x] 2.3 Un-ignore and pass
      `an_outage_wider_than_the_reorder_window_does_not_permanently_wedge_the_stream`.
- [x] 2.4 Add a test for mid-stream join (first packet at sequence 500).
- [x] 2.5 Surface the condition in diagnostics while it is happening —
      `reorder_window_rejections` climbing while `phase == Buffering` is the
      signature, and nothing reports it today.

**2 resolution and a trap avoided:** the first attempt adopted any
far-future sequence whenever the buffer was empty. That fixed the wedge but
broke `rejects_a_hostile_flood_of_far_future_sequences` — a single corrupt or
hostile packet could have moved the stream permanently, which is strictly
worse than the wedge. The shipped fix requires **corroboration**: three
consecutive far-future arrivals with nothing accepted in between, adopting
the *lowest* sequence of the run so an outlier cannot drag playback past the
real position. Any accepted packet resets the run. A stray packet is still
rejected; a genuinely advanced stream corroborates itself within ~60ms.
`resynchronisations` is exposed in diagnostics.

## 3. CRITICAL — a guaranteed-silent stream reports itself as healthy

The diagnostics loop in `MainViewModelRustListener.kt` never reads
`diagnostics.syncLocked` — the one field that says "this stream can never
play" — and writes `playbackState = PLAYING` (or `UNDERRUN`) into
`diagnosticsStore` unconditionally, regardless of `phase`. During item 1's
failure the UI reports **PLAYING with a healthy buffer while the device is
silent**. Direct violation of CLAUDE.md's "do not hide transport, sync,
playback, queue, or database failures behind generic success states".

- [x] 3.1 Derive the reported playback state from `phase` and `syncLocked`,
      not from a constant. `AwaitingSync` and `Buffering` must be visible as
      themselves.
- [x] 3.2 Surface a visible error when a stream has been open and unable to
      play for longer than a bounded time (no accepted sync sample, or
      `Buffering` that never resolves).
- [ ] 3.3 Audit the manual path's UI mapping for the same class of problem.

## 4. HIGH — cross-listener alignment is never actually established

`queue_alignment_prefill` is the only thing tying ring playout position to
the host presentation timeline, and under the shipping config it never fires:
with `startup_buffer_target_ms = 1000` and `write_lead_ms = 400`, sequence
0's deadline is ~900ms in the past by the time the buffer fills, so
`lead_ms == 0`. The device run confirms this — `prefillFrames=0`.

Steady state still self-corrects (depth converges to the lead, so play time
≈ deadline), but the *initial* offset is set by whenever that listener
locked sync and flushed its backlog. Two phones locking sync 50ms apart play
~50ms apart, permanently. For this project that is the core purpose, not a
detail.

Also: `awaiting_prefill` is never re-armed by `rebuffer`, `apply_sync_offset`,
or the `AwaitingRebuffer` path, and a ring underrun advances wall time
without advancing `read_index` — so an underrun shifts a listener permanently
later with nothing that detects or corrects it.

- [ ] 4.1 Make alignment hold under the real config, not only when
      `startup_buffer_target_ms` is 0. Every existing prefill test sets it to
      0, which is why this was never caught — fix the tests too.
- [x] 4.2 Re-arm alignment after any rebuffer or offset adoption.
- [ ] 4.3 Add a correction path for accumulated playout drift: compare
      intended vs actual playout position and correct gently, bounded well
      under the hard-resync threshold. This overlaps item 5.4 in the
      migration TODO (slew-limited correction) — do them together.
- [ ] 4.4 Add a test with **two schedulers on one simulated timeline** that
      lock sync at different moments and assert their playout positions
      converge. Nothing currently tests the property the product exists for.
- [ ] 4.5 Device-validate with two phones playing the same stream. This is
      the acceptance criterion for the whole project and has never been run.

**4 STATUS: ATTEMPTED, REGRESSED ON DEVICE, REVERTED (2026-08-03).**

The diagnosis below still stands and the acceptance test
(`two_listeners_...`, now `#[ignore]`d) still encodes the target. The
*implementation* was wrong and is reverted; `discard_already_late_head` is
retained, unused, with this explanation attached.

**What went wrong:** `poll` receives a time the pump has already advanced by
its 400ms write lead, so discarding "everything whose deadline has passed"
actually discarded a lead's worth of *future* audio too — and after every
rebuffer it emptied the buffer, concealed to the bound, forced another
rebuffer, and fell further behind the live stream each cycle. Device result:
playback ran ~11s of a 40s stream and never recovered
(`accepted=592 received=1809 reorderWindow=1124 ringSilenceFilled≈26.9s`),
versus a clean 33.28s with only a 12s hiccup before the change.

**What the redesign must do differently:** perform the discard against the
*true* current time, not the release horizon. The pump knows both; the
scheduler currently knows only the horizon. Options: pass both times into
`poll`, give the scheduler the configured lead so it can subtract, or move
the discard into the pump. Also ensure a rebuffer cannot empty the buffer it
just spent a second accumulating.

**Original diagnosis, still believed correct:**

**4 resolution — the real root cause was not the prefill.** A stream is heard
at `write time + ring depth`, and writing ahead into a FIFO preserves relative
timing exactly, so the whole stream is offset by however late its *first*
frame was when playback began. Starting on a stale head shifted everything by
that listener's own startup latency. Fixing the first frame fixes all of them.

`PlaybackScheduler` now drops buffered packets whose deadline has already
passed at the moment it enters `Playing`. The cost is the already-elapsed head
of the stream, bounded by the startup buffer — being late together is the
product; hearing the first second is not. Alignment is also re-armed on every
rebuffer and offset adoption, which it previously never was.

`two_listeners_locking_sync_at_different_moments_play_the_same_audio_together`
is the regression test: two schedulers on one timeline, one locking sync 300ms
after the other, must agree on when a shared sequence is heard to within one
packet. **Verified to fail with the fix disabled and pass with it enabled**,
so it constrains the behaviour rather than passing vacuously.

The inert startup prefill described above was a symptom, not the cause. Still
open: drift accumulated *during* a stream (4.3), and the two-device
measurement (4.5).

## 4b. HIGH — nothing accounts for the sync gate being closed (found on device 2026-08-03)

Device run after items 1–5: no popping, zero discontinuities, but a 6.3s
silent start and an audible hiccup at 12s. Diagnostics:
`accepted=1632 received=1973 reorderWindow=335 skipped=367
ringUnderruns=3140 ringSilenceFilled=301440` (6.28s).

Two independent causes, both latent before today and both exposed only
because sync happened to be slow on this run:

**(a) Sync acquisition is slow by construction.** Three probes were rejected
for RTT above the 200ms acceptance bound (the accepted one was 179ms) and the
probe cadence is a flat 2s, so three rejections cost six seconds. The pump
correctly plays nothing until an offset is accepted, so this is dead air.

**(b) Packets arriving while sync is unlocked are permanently lost.** They
accumulate against `next_expected = 0`, the reorder window admits only 64, and
everything past ~1.28s is rejected as unreorderable — 335 packets here. The
12s hiccup is the buffer draining and then resynchronising onto the live
position via the item-2 path.

- [ ] 4b.1 Probe fast until locked (~250ms), then fall back to the steady
      cadence. Applies to both listener paths.
- [ ] 4b.2 Do not submit packets to the scheduler while sync is unlocked.
      They cannot be scheduled without an offset and are stale by the time
      one exists; the item-2 resync adopts the live position instead. Count
      what is dropped so it stays visible.
- [ ] 4b.3 Re-run the device test and confirm the silent start and the
      mid-stream hiccup both disappear.

## 5. HIGH — every listener failure path leaks a live runtime

`stopListenerPlaybackForFailure` (`MainViewModelTransport.kt`) cancels
`playbackJob` and calls the *legacy* `playbackEngine.stop()`, but never calls
`stopListenerPlayback()`. It is the shared cleanup for
`handleListenerDisconnect` and `handleListenerConnectionFailure`, including
the disconnect raised from inside the new diagnostics loop.

Each connect/drop cycle leaks a pump thread and a ring registration, and
`FfiListenerPlaybackHandle.close()` is never called. Compounding it,
`OboePlaybackEngine.stop()` calls `nativeOboeClose()` unconditionally on the
single process-global adapter, so this path also closes the Oboe stream out
from under any other live runtime — the manual path goes silent while still
reporting `Playing`. The same unconditional close exists in
`stopHostPlaybackImpl` and `stopAdvertisingForRust`.

- [x] 5.1 Route every listener failure/disconnect path through
      `stopListenerPlayback()`.
- [x] 5.2 Stop the discovered-session runtime in `onCleared()` — rotating the
      device or backgrounding the Activity currently leaks the pump thread
      and the Oboe stream past the ViewModel.
- [x] 5.3 Make `ManualListenerTransportController.connect()` stop existing
      playback before reconnecting. Its early-return failure paths currently
      leave the previous stream audibly playing behind a "connection failed"
      message.
- [x] 5.4 Fix ownership of `nativeOboeClose()`. Only the component that
      opened the stream should close it; a global unconditional close from
      three unrelated call sites cannot be correct once two paths exist.
- [ ] 5.5 Add a test or instrumentation asserting no runtime outlives its
      stream (thread count / registry entries stable across N connect-drop
      cycles).

## 6. HIGH — unsynchronised shared state across three threads

`ManualListenerTransportController` correctly uses `AtomicReference` for
`handleRef` but leaves `playbackRuntime`, `currentStreamId`, and `sessionId`
as plain `var`s, written from the event-loop coroutine, read from the
sync-probe coroutine (a different `Dispatchers.IO` thread), and written from
`reset()`/`close()` on the main thread. No happens-before edge.

Concrete: `handleStreamStarted` sets `playbackRuntime` on IO-thread-A, then
launches the probe loop on IO-thread-B, which may still observe `null`,
`break`, and never send a probe for that stream — permanent silence, with the
probe job appearing to complete normally. Mirror case: B reads a stale
reference to an already-`close()`d handle and the exception is swallowed by
`runCatching`.

- [ ] 6.1 Make the shared fields safely published (`AtomicReference`, or
      confine all mutation to a single dispatcher).
- [ ] 6.2 Re-check the same pattern in `MainViewModel.listenerPlayback`,
      which is touched from the main thread and the diagnostics loop.

## 7. MEDIUM-HIGH — burst packet loss produces a packet-rate buzz

Each concealed frame fades its tail to zero and the next concealed frame
blends *from that zero*, so a run of 4 lost packets emits four 20ms blips
with hard zero-crossings — amplitude modulation at 50Hz, the same perceptual
family as the popping this work removed. Isolated losses are fine (all 8 in
the validated run were isolated); bursts are not.

Note the existing test `a_concealment_run_starts_from_the_previous_concealed_tail_not_the_real_packet`
asserts the *wrong* behaviour and must change with the fix.

- [x] 7.1 Keep concealment continuous across a run. Sketch: only fade a
      concealed frame's tail when it is the last of its run, or carry the
      pre-fade tail forward so successive frames continue the waveform and
      the decaying amplitude reaches silence on its own.
      *Done: `conceal` no longer fades every frame; the un-faded tail carries
      forward and only a run's last audible frame lands on silence. "Last"
      covers the bound frame and the one before it, because the scheduler
      discards the bound frame in favour of a rebuffer rather than playing it.*
- [x] 7.2 The resuming real frame currently fades in from zero
      (`apply_fade_in`), which is a step if the preceding concealed frame did
      not end at zero. Blend it from the previous emitted tail instead, the
      way concealment already does.
      *Done: `apply_blend_in` replaced `apply_fade_in` outright — an empty
      `from` degrades to the old fade, so one path serves both cases. The
      scheduler tracks `resume_blend_tail`, cleared wherever playback really
      does resume from silence (stream start, abandoned gap, rebuffer).*
- [x] 7.3 Replace the test that locks in the current behaviour; assert
      continuity across a 4-packet run instead.
      *Done: three tests asserted the old fade-to-zero and were rewritten.
      Added `a_burst_of_losses_decays_continuously_without_returning_to_silence`
      and `the_last_audible_frame_of_a_bounded_run_lands_on_silence`.*
- [x] 7.4 Verify by burst-loss simulation in the WAV analysis, not by ear
      alone.
      *Done: `a_burst_of_lost_packets_renders_as_one_continuous_decay` drives a
      4-packet burst through the real pump into a captured WAV and measures the
      envelope off the file. Confirmed non-vacuous by restoring the old fade
      condition and watching it fail.*

**How this was found.** Per-second device diagnostics (run 13) showed ring
underruns confined entirely to startup — zero for all 35 s of playback, ring
depth steady at the 400 ms target — which ruled out a pacing defect. The
remaining audible artefacts lined up 1:1 with concealment bursts: a `+4`
concealment second matched the WAV's only mid-stream silence gaps, at 19.100 s
and 19.119 s.

## 8. MEDIUM — the drained tail is queued but never played

`stop()` sets `running = false` *before* calling `finish()`, so the pump loop
may already have exited and the pending remainder can never be flushed —
`finish()`'s return value is discarded. Then `release_render_ring` runs
immediately, so even the frames that did reach the ring are never read.
Kotlin compounds it by calling `nativeOboeClose()` right after `stop()`
returns. The whole `drain_remaining` / `drain_all` tail-preservation
machinery is defeated at the call site.

Related: the debug WAV records frames at `enqueue_frame` — before the ring
accepts them — so the capture contains audio that was never rendered, which
is exactly the disagreement the recorder exists to rule out.

- [ ] 8.1 Drain, then let the ring actually play out before releasing:
      finish, flush pending, wait for `queued_frames()` to reach ~0 with a
      bounded timeout, then release and close Oboe.
- [ ] 8.2 Report, don't discard, whatever `finish()` could not queue.
- [ ] 8.3 Snapshot `last_diagnostics` *after* the join, not before — a final
      tick can currently mutate counters after the "final" summary.
- [ ] 8.4 Decide what the debug capture represents and document it: either
      record only frames the ring accepted, or keep recording releases and
      state plainly that it is "released toward the ring, not rendered".

## 9. MEDIUM-HIGH — a panicking pump thread is invisible until teardown

`run_pump` discards every `PumpTick`, and `lock_pump` uses
`PoisonError::into_inner`, so a panic in `tick` kills the thread silently:
`running` stays true, `submit_packet` keeps succeeding, `diagnostics()`
returns a frozen but plausible snapshot, and the UI keeps reporting PLAYING.
The `PumpThread` error surfaces only at `stop()`. CLAUDE.md lists contained
panics as a mandatory diagnostic.

- [ ] 9.1 Add a liveness signal to `PlaybackDiagnostics` (tick counter or
      last-tick timestamp) and surface a stalled pump as a visible failure.
- [ ] 9.2 Record a contained panic explicitly rather than inheriting it
      through mutex poisoning.

## 10. MEDIUM — silent rejection classes and swallowed failures

- [ ] 10.1 `listener_playback.rs` discards the `JitterBufferRejection` with
      `let _ =`. The justifying comment covers duplicate/late/reorder, but
      the same line swallows `WrongSession`, `WrongStream`, and
      `BufferedDurationExceeded`. A stream-generation change gives 100%
      rejection, total silence, and every exposed counter reads normal.
      Expose these three in `FfiPlaybackDiagnostics`.
- [ ] 10.2 Fix the `runCatching {}` sites with no `.onFailure`:
      `submitPacket` (both paths), `beginSyncProbe` + `sendSyncRequest`,
      `sendDisconnect`, `handle.shutdown()`. Forbidden by CLAUDE.md; here it
      is not even log-only.
- [ ] 10.3 `apply_sync_offset` accepts a non-finite offset. NaN maps every
      deadline to 0, dumps the buffer as due, and can never be corrected
      because `NaN > threshold` is false so it reports `SoftCorrected`
      forever. Reject non-finite input and make the soft-correct branch an
      explicit `<=` comparison.

## 11. MEDIUM — main-thread contention on the audio path

`ListenerTransportController.events` is collected on `Dispatchers.Main`, so
`handleTransportAudioReceived` → `submitPacket` runs on the **main thread
~50×/s**, each call marshalling ~3840 bytes across UniFFI and taking the same
mutex the pump thread holds every 10ms. The diagnostics loop adds a second
main-thread acquisition at 10Hz, and `stopListenerPlayback` blocks the main
thread on the pump thread join. The manual controller does this correctly on
`Dispatchers.IO`.

- [ ] 11.1 Move the discovered-session event collection and packet
      forwarding off the main thread.
- [ ] 11.2 Make teardown not block the main thread on a thread join.

## 12. LOW — correctness and hygiene

- [ ] 12.1 `packet_duration_ms` is derived by truncating integer division in
      **two** Kotlin call sites. Exact for 960 @ 48kHz, but 1024 @ 48kHz
      gives 21ms instead of 21.33ms, and the scheduler multiplies it by the
      sequence number — cumulative drift of ~16ms/s. Move this derivation
      into Rust and stop duplicating it in the platform layer.
- [ ] 12.2 `ringUnderruns` is cumulative but pins `diagnosticsStore` to
      `UNDERRUN` forever after a single startup underrun. Report an
      instantaneous condition, or rate.
- [ ] 12.3 `drain_all` counts drained packets as `emitted` but never counts
      holes inside the drained range as `skipped`; `sequences_skipped`
      under-reports at stop.
- [ ] 12.4 Debug recorder: the RIFF header arithmetic can overflow `u32`
      once `data_bytes` saturates (~6h of 48kHz stereo), and a post-`finish`
      `append` returns `Ok(())` while dropping samples.
- [ ] 12.5 Remove dead imports and fields left by the rewire in
      `ManualListenerTransportController` (`SystemClock`, `AudioFormatSpec`,
      `OboePlaybackEngine`, `PlaybackEngine`, `PlaybackFrame`,
      `PlaybackThresholds`, `AudioPacket`, `SyncResponsePacket`,
      `HostTimeMapper`, `ListenerSyncController`, `SyncMaintenanceConfig`,
      `SyncQualityBadge`, `SyncState`; fields `writtenCount`,
      `lastWrittenFrameConcealed`).

## What the reviews found clean

Recorded so a later pass does not redo this analysis:

- Ramp arithmetic (`ramp.rs`) — endpoints match their doc comments;
  `clamp_to_i16` saturates correctly; indices provably in bounds.
- Pump ordering, duplication, and unbounded growth — `pending` is
  append-only, drained exactly, and `tick` refuses to poll while non-empty.
- `drain_remaining` sequence handling and hole fade pairing.
- Wide-hole skip vs. concealment counters and the fade-in flag.
- Render-ring SPSC atomics and Acquire/Release pairing.
- `stop()` / pump-thread deadlock — the pump guard is scoped to end before
  the join, so no lock-ordering inversion exists.
- `stop()` then `close()` does not double-release; token aliasing is
  impossible; two `nativeOboeOpen` calls fail loudly rather than silently.
- Packet field mapping across the FFI boundary is 1:1.
- Manual-path sync timestamps correctly use `runtime.nowMs()` for both t1
  and t4.

## Validation required before calling this done

- [ ] Rust gate green (`bash scripts/check-rust.sh`).
- [ ] Android gate green (`./gradlew test lintDebug`).
- [ ] Manual-connect device run, re-measured against the 2026-08-03 baseline
      (39.96s, zero silence gaps, zero discontinuities, `ringUnderruns=68`).
      No regression on any of those.
- [ ] Discovered-session device run — **never performed**; needs a second
      Android device as host.
- [ ] Two-listener sync measurement (item 4.5). The project's success
      criterion is that listeners hear the same thing ~99% of the time, and
      nothing has ever measured it.
