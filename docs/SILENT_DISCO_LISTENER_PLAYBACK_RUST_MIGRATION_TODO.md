# Listener Playback Rust Migration TODO

Replace the legacy Kotlin listener playback pipeline
(`ListenerPlaybackScheduler` / `AudioPacketBuffer` and the Kotlin-side
pacing loop in `ManualListenerTransportController`) with the already-built
Rust core pipeline (`audio::JitterBuffer`, `audio::ConcealmentPolicy`,
`audio::PlaybackScheduler`, `audio::RenderRing`), so scheduling,
concealment, and ring pacing have exactly one authoritative owner and the
future desktop/iOS listeners inherit them for free.

Read first:

- `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md` (boundaries, real-time
  rules)
- `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` Blocks 15, 16, 18 and
  their implementation notes (what exists in Rust, and why the Kotlin
  pipeline was deliberately left in place at Block 18)
- `memory.md` entries dated 2026-08-02 (every requirement below was found
  and verified on a physical device that day; do not re-litigate them)

## Why this exists

On 2026-08-02 a long real-device debugging session fixed audio popping,
static, dropouts, and pacing bugs **in the Kotlin path** — which is the
deprecated implementation. The Rust core already contains stricter
equivalents of several of those fixes (built 2026-08-01, unwired), and the
rest must be ported once, into Rust, instead of being re-implemented per
platform. Until this migration lands, Kotlin and Rust are competing owners
of playback scheduling, which the architecture spec forbids.

## Verified requirements the Rust pipeline must satisfy

Every row was empirically confirmed on a physical Android device
(SM-A546E) against a real desktop host stream on 2026-08-02. The WAV/
telemetry methodology to re-verify them is in "Device validation
protocol" below.

| # | Requirement | Where it stands in Rust today |
|---|---|---|
| R1 | Reject stale/duplicate/already-emitted arrivals (never play out of order) | Done: `JitterBuffer::accept` |
| R2 | Bound consecutive concealment during arrival outages | Done: `ConcealmentPolicy` (`max_consecutive_concealed_packets`) |
| R3 | Arrival-continuity loss accounting (not playback-head distance) | Done: `JitterBuffer::missing_sequence_count` |
| R4 | Concealment by **decaying repetition of the last real packet**, not silence: entry ramp starting at the previously played sample values, per-repeat halving (shift +1 per consecutive concealment, capped at 8), 5 ms tail fade to zero | **Port** — `PlaybackScheduler`/`ConcealmentPolicy` currently synthesize silence (`ScheduledFrame.samples` doc: "real or silence") |
| R5 | Fade-in (5 ms) on the first real frame after any concealment and on the first frame of a stream | **Port** |
| R6 | Wide sequence holes (> 10 packets) are skipped, not concealed frame-by-frame — bridging a multi-second hole queues the whole outage as dead air ahead of the resume content and drags playback seconds late | **Port** as policy; `JitterBuffer::skip_expected_sequence` is the primitive |
| R7 | Stop-time drain plays buffered tail content with hole-edge fades (fade-out into each hole, fade-in out of it) and a final-tail fade to zero | **Port** — see Kotlin `drainRemaining()` |
| R8 | Ring pacing: frames written ~400 ms ahead of presentation deadline (write-lead), ring depth capped at target fill (never rely on full-ring backpressure), deadline-aligned silence prefill at stream start so the first frame plays at its deadline | **Build** — this is the pump in Phase 2; constants below |
| R9 | Playback start gated on a genuinely accepted sync sample (estimator confidence != UNKNOWN), never on a sync *event* having arrived | Exists in Kotlin manual path; carry into the new wiring |
| R10 | Diagnostics: loss, late-drop, concealed count, ring depth, underruns, silence-filled frames, stall equivalents queryable at any time and logged in one end-of-stream summary | Partially (ring telemetry exists); the runtime must aggregate and expose it |
| R11 | Debug PCM capture of exactly what was handed toward the ring (WAV), toggleable, for objective offline analysis | **Port** — Kotlin `DebugPcmRecorder`; this workflow found every bug above, do not lose it |

Constants proven on-device (keep names greppable):

| Constant | Value | Current home |
|---|---|---|
| Packet duration | 20 ms | protocol |
| Format | PCM16LE, stereo, 48 kHz wire; f32 interleaved in ring | fixed |
| Ring capacity / target fill | 48 000 / 19 200 frames (1 s / 400 ms) | `OboePlaybackEngine.kt` (move to Rust config) |
| `RING_WRITE_LEAD_MS` | 400 | `ManualListenerTransportController.kt` |
| `MAX_RING_PREFILL_MS` | 800 | same |
| `CONCEALMENT_RAMP_MS` | 5 | `PlaybackScheduling.kt` |
| `CONCEALMENT_BRIDGE_MAX_PACKETS` | 25 | same (Rust equivalent exists as policy bound) |
| `CONCEALMENT_SKIP_THRESHOLD_PACKETS` | 10 | same |
| Startup buffer | 1 000 ms (manual path experiment) vs `PlaybackScheduler` default 400 ms | reconcile in Phase 2 (make it config, keep 1 000 ms for the manual path until device evidence says otherwise) |

## Phase 1 — Port the audio-quality behaviors into the Rust core

Pure-Rust, no wiring changes, fully unit-testable. Mirror the Kotlin tests
in `app/src/test/java/com/ekkus/silentdisco/core/audio/ListenerPlaybackSchedulerTest.kt`
— they encode the verified behaviors.

- [x] 1.1 Replace silence synthesis in `PlaybackScheduler`/`ConcealmentPolicy`
      with decaying-repetition concealment (R4):
  - [x] Track the last real packet's samples and the consecutive-concealment
        generation inside the scheduler.
  - [x] Synthesize: source samples attenuated by `>> (generation + 1)`
        (cap 8), entry ramp over 5 ms blending from the previously played
        frame's final sample values, tail fade over the final 5 ms to zero.
        Integer PCM16 math; port from Kotlin
        `pcm16LeConcealmentPayload` / `pcm16LeLastFrame`
        (`PlaybackScheduling.kt`) — but operate on `i16` samples directly,
        no byte packing.
  - [x] `ConcealmentOutcome::HardResyncRequired` semantics unchanged.

  **Done:** state lives in `ConcealmentPolicy` (`last_real_samples`,
  `previous_tail`), not the scheduler, so the repetition source and both
  seam values stay with the policy that owns concealment. New
  `audio/ramp.rs` holds the shared integer-PCM shaping (`blend_sample`,
  `apply_fade_out_tail`, `last_frame`, `scale_sample`). `ConcealmentPolicy::new`
  now takes `ramp_frames` (validated, new `RampFramesOutOfRange` error kind)
  and `record_delivery` now takes the delivered samples; `SchedulerConfig`
  gained `concealment_ramp_ms` (default 5) and derives ramp frames from the
  stream's validated packet geometry. **Pre-existing bug found and fixed
  while getting the gate green:** `audio_output.rs` tests mutated the
  process-global engine registry without holding `audio_abi`'s
  `registry_test_guard`, while `token_space_exhaustion_...` deliberately
  rewrites the shared token counter — an intermittent
  `STOPPING`-instead-of-`PARTIAL` failure in
  `partial_read_fills_remaining_frames_with_silence_and_reports_partial`.
  The guard is now crate-visible and held by every registry-touching test.
- [x] 1.2 Fade-in on resume and stream start (R5): first real frame after
      any concealed frame, and the first frame ever delivered, get a 5 ms
      linear fade-in applied to their samples before delivery.

  **Done:** `PlaybackScheduler` tracks `fade_in_next_real_frame` (initially
  true), sets it after every concealed frame and after `rebuffer()`, and
  clears it once applied. `record_delivery` is called with the *unfaded*
  samples so a later concealment repeats the real waveform rather than a
  ramped copy. `ramp::apply_fade_in` added.
- [x] 1.3 Wide-hole skip policy (R6): when the next buffered sequence is
      more than `concealment_skip_threshold_packets` (config, default 10)
      ahead of the expected sequence, skip the hole via
      `JitterBuffer::skip_expected_sequence` (repeatedly, or add a
      `skip_to(sequence)` helper), reset the concealment run, and mark the
      next real frame for fade-in. No concealment frames for the skipped
      range.

  **Done:** added `JitterBuffer::skip_to_earliest_buffered()` (returns the
  count abandoned) plus a `skipped` statistic covering both skip paths, which
  R10's diagnostics will surface. `SchedulerConfig` gained
  `concealment_skip_threshold_packets` (default 10), validated to be smaller
  than `max_reorder_window` — the buffer rejects anything beyond that window,
  so a larger threshold could never engage and would silently disable the
  policy (new `InvalidConcealmentSkipThreshold` error kind). After skipping,
  `poll` recomputes the post-gap deadline and returns `Waiting` if that frame
  is not due yet, so the resumed audio keeps its own presentation time.
- [x] 1.4 Drain-with-fades (R7): add a scheduler `drain_remaining()` that
      returns all buffered frames in sequence order ignoring deadlines,
      with fade-out applied to the frame before each sequence hole,
      fade-in after each hole (including a hole against the last
      live-delivered sequence), and a final-tail fade on the last frame.

  **Done:** added `JitterBuffer::drain_all()` (sequence-ordered, advances
  past the last drained packet, counts them as emitted) and
  `PlaybackScheduler::drain_remaining()`. The first drained frame also fades
  in when the scheduler's pending fade-in flag is set — a preceding concealed
  frame faded to zero, so the resume seam needs the ramp even without a
  sequence hole.
- [x] 1.5 Unit tests, mirroring the Kotlin suite: repetition content and
      exact decay values (8000 → 4000 → 2000 → 1000 for constant input),
      entry-continuity first sample, tail-zero last sample, fade-in on
      resume/first frame, bounded outage bridge (exactly the policy bound,
      then `HardResyncRequired`/stop — decide and document which), wide-hole
      skip delivering the post-hole frame directly, drain hole-edge fades,
      stale-arrival rejection staying intact.

  **Done:** 108 core audio tests, including a new `ramp_tests.rs` covering the
  shaping helpers directly (saturation, degenerate shapes, over-long ramps).

  **Outage-bridge decision:** keep Rust's `AwaitingRebuffer` semantics rather
  than Kotlin's "stop synthesizing and go quiet". Reaching the bound pauses
  the scheduler until the caller explicitly calls `rebuffer()`, which re-arms
  the startup buffer — the correct response after a long outage, since the
  buffer is empty and the timeline has moved on, and it cannot silently
  resume mid-outage. **`DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS` raised
  from 5 to 25** (500ms at 20ms packets) to match the device-validated
  Kotlin bridge length: reaching the bound costs a full startup-buffer
  re-accumulation, so a bound short enough to trip on an ordinary brief
  outage would replace a ~100ms interruption with a much longer rebuffering
  silence. Decaying repetition is already inaudible within the first handful
  of packets, so the rest of the bridge is silence held open in case audio
  resumes. Revisit against device evidence in Phase 3.5.
- [x] 1.6 `bash scripts/check-rust.sh` green. (fmt, clippy `-D warnings`,
      and the full workspace suite — 221 core tests — all pass.)

## Phase 2 — Build the Rust listener playback runtime (scheduler → ring pump)

New component owning the data path from submitted packets to the render
ring. Suggested home: `rust/silent-disco-ffi/src/listener_playback.rs`
(worker thread is FFI-crate territory, matching the database worker
precedent), with any pure logic in `silent-disco-core`.

- [x] 2.1 `ListenerPlaybackRuntime`: owns one `PlaybackScheduler`, one
      `RenderRingProducer` (+ registered engine token), and one dedicated
      pump thread. Explicit lifecycle: `start(config)`, `stop()` (drain +
      fades per R7, then release), `Drop` fail-visible like the database
      worker. No UniFFI/JNI/logging/allocation in the *real-time C ABI
      read path* — the pump thread is NOT the real-time thread and may
      allocate/log sparingly.

  **Done, split across the two crates:** the pure, thread-free
  `audio::PlaybackPump` lives in `silent-disco-core` (owns the scheduler and
  ring producer, converts PCM16 → interleaved f32 with gain, `tick(now_ms)`,
  `finish()` which drains per R7 then stops). Frames the ring cannot accept
  are held in a pending FIFO and retried, never partly discarded — the same
  class of silent loss that was a real Kotlin bug. `silent-disco-ffi`'s
  `ListenerPlaybackRuntime` adds the registered token, the pump thread, and
  the lifecycle: a failed `start` leaves nothing registered, `stop` drains
  before joining and reports a panicking pump thread rather than swallowing
  it, and `Drop` still releases. Pacing (write-lead, depth cap, prefill) is
  2.2; this tick simply writes what is due.

  **Bug found while testing:** `finish()` called a queue helper whose own
  invariant required an empty pending buffer, so draining a tail into an
  already-full ring tripped it. Reworked into an append-then-flush FIFO,
  which is also what preserves playback order after a partial write.
- [x] 2.2 Pump loop (R8), all constants from the table above as config:
  - [x] Poll cadence ~10 ms; monotonic clock injected (testable).
  - [x] Release frames `write_lead_ms` ahead of mapped deadline.
  - [x] Never push past `target_fill_frames` queued depth (query the
        producer; do not rely on full-ring backpressure).
  - [x] At start: silence prefill of `(first frame's mapped local deadline
        - now)` clamped to `[0, max_prefill_ms]`, written before the first
        real frame, so the first frame plays at its deadline (the native
        callback consumes from stream open; play time = write time + queued
        depth).
  - [x] i16 → f32 conversion (and volume scaling) happens here, once —
        Kotlin stops touching PCM entirely, closing Block 18's disclosed
        deviation.

  **Done:** `PlaybackPumpConfig` carries `write_lead_ms` (400),
  `max_prefill_ms` (800), and `target_depth_frames` (400ms), with
  `target_depth_frames` validated against the ring's capacity (new
  `InvalidTargetDepth`). The clock is injected as `tick(local_now_ms)`; the
  FFI thread supplies it at a 10ms cadence, so every pacing test runs on a
  fake clock against a real ring.

  **Worth knowing:** the alignment prefill establishes the cushion on the
  very first frame, so the depth cap — not the lead — governs from the
  second frame onward. That also means the prefill can never exceed
  `write_lead_ms` in a normal config (a frame is released at most one lead
  early), making `max_prefill_ms` a pure safety bound; its test configures a
  lead wider than the ceiling to exercise it.
- [x] 2.3 Sync integration (R9): runtime accepts offset updates
      (`apply_offset_update` / `rebuffer` semantics already in the
      scheduler). Playback must not start before the first accepted
      update. Decide and document: Kotlin keeps forwarding sync-response
      timestamps to the Rust estimator (Block 6 exports exist) and pushes
      the resulting offset in, OR the runtime owns the estimator directly.
      Either way Kotlin performs no estimation math.

  **Decision: the runtime owns the estimator.** It holds its own
  `ClockSyncEstimator`; the platform only forwards raw four-timestamp
  exchanges via `begin_sync_probe` / `observe_sync_response`. This deletes
  the whole class of bug that produced a physically impossible skew
  (-5.26e10 ppm) and total silence — that came from platform-side
  regression math over placeholder offsets. Rejected samples now cannot
  reach either the timeline or the skew estimate, because the platform
  never sees an offset to push.

  **Timeline note:** local timestamps must come from `now_ms()`, which
  exposes the same monotonic clock the pump schedules against. Passing an
  unrelated platform clock would silently misalign sync and playback.

  Playback is gated on `sync_locked`: the pump returns `AwaitingSync` and
  queues nothing until a sample is genuinely accepted. The first accepted
  offset is adopted outright rather than compared against the placeholder
  (host and listener epochs are unrelated, so that comparison is
  meaningless); later ones correct softly or force a rebuffer. A scheduler
  that pauses on the concealment bound is re-armed automatically — the
  pause exists to force a fresh startup buffer, not to end playback.
- [x] 2.4 Diagnostics (R10): one snapshot struct — received, lost
      (arrival-continuity), late-dropped, concealed, skipped-hole count,
      current/peak ring depth frames, underrun + silence-filled counters
      (from ring telemetry), state (Buffering/Playing/AwaitingRebuffer/
      Stopped). Queryable via the control surface; runtime logs one
      summary line on stop.

  **Done:** `PlaybackDiagnostics` gathers all three layers — jitter-buffer
  accounting (accepted/emitted/skipped plus late, duplicate, and
  reorder-window rejections), concealment counters, buffered span, ring
  depth (current and peak), pending and prefill frames, and the ring's own
  underrun/silence-filled/full counters. `PlaybackPhase` exposes the
  scheduler state. Available live via `ListenerPlaybackRuntime::diagnostics()`.

  **Instead of a log line on stop,** `stop()` captures the final snapshot into
  `final_diagnostics()`. A log line would be written by the crate least able
  to route it, and the one moment worth reporting — the teardown — is exactly
  when the live accessor stops working. The platform layer formats and logs
  it, which is also where the existing summary log lives.
- [x] 2.5 Debug PCM tap (R11): optional config path; when set, runtime
      writes exactly the i16 frames it releases toward the ring (real,
      concealed, and drained — not the alignment prefill) to a WAV,
      finalized on stop. Same 44-byte-header format the Kotlin
      `DebugPcmRecorder` produced so the existing analysis tooling works.

  **Done:** `audio::DebugPcmRecorder` writes the canonical 44-byte header and
  patches its length fields on finish, so the existing offline analysis
  scripts work unchanged. Enabled per stream via
  `ListenerPlaybackRuntime::start_debug_capture(path)`; off otherwise. The
  capture records real, concealed, and stop-drained frames but not the
  alignment prefill, which is ring positioning rather than stream content.
  A capture failure disables further capture and is reported through
  `debug_capture_error()` rather than silently truncating the file — a
  recording that quietly stops is worse than none, since the analysis would
  read the truncation as a dropout.
- [x] 2.6 UniFFI control surface (`FfiListenerPlaybackHandle` or similar):
      `open(config) -> handle`, `submit_packet(...)` (per-packet forwarding
      from Kotlin's existing transport event loop is acceptable control-
      plane load: ~50 calls/s), `apply_sync_offset(...)` (or sync-sample
      forwarding per 2.3), `engine_token()` for the Oboe adapter,
      `diagnostics_snapshot()`, `stop()`. Errors explicit and typed; no
      silent fallbacks.

  **Done:** `FfiListenerPlaybackHandle` plus records
  `FfiListenerPlaybackConfig`, `FfiAudioPacket`, `FfiPlaybackDiagnostics`,
  `FfiSyncSampleOutcome`, and enum `FfiPlaybackPhase`. Per 2.3 the surface
  takes raw sync exchanges (`begin_sync_probe` / `observe_sync_response`),
  not offsets. `now_ms()` is exposed because every local sync timestamp must
  come from the same clock playback schedules against. Errors are a typed
  flat enum covering configuration, stopped, pump-thread, sync, and
  debug-capture failures.

  `ListenerPlaybackRuntime::stop` now takes `&self` (thread handle and final
  diagnostics moved behind their own locks), since UniFFI objects expose
  shared references. Verified by generating the Kotlin bindings and checking
  the emitted class and record shapes, not just by compiling the Rust.
- [x] 2.7 Rust tests: pump pacing with a fake clock and a real ring
      (deadline alignment of first frame incl. prefill clamping both
      directions, steady-state depth converging to the lead, depth cap
      under a flooded backlog, drain-on-stop content), diagnostics
      correctness, lifecycle/double-stop/drop behavior.

  **Done:** 22 pump tests and 12 runtime tests. The load-bearing one is
  `steady_state_ring_depth_converges_to_the_configured_cushion`: it runs 400
  simulated ticks with a host delivering ahead, the pump ticking every 10ms,
  and a consumer draining at 48kHz, then asserts the depth stays between
  ~12000 frames and the cap **and that the ring never underran once**. That
  is the direct statement of what the pacing exists to achieve, and it is
  the property the previous implementation failed.
- [x] 2.8 `bash scripts/check-rust.sh` green. (fmt, clippy `-D warnings`,
      full workspace suite — 243 core tests — and the UniFFI Kotlin bindings
      verified to generate.)

## Phase 3 — Rewire the Android manual-connect path

- [x] 3.1 `ManualListenerTransportController`: replace
      `ListenerPlaybackScheduler` + playback pump job + `OboePlaybackEngine
      .write/prefillSilence/queuedDepthFrames` usage with the runtime
      handle: forward `AudioReceived` events (`mapAudioReceivedToPacket`
      shape) via `submit_packet`, forward sync per 2.3, keep the
      `confidence != UNKNOWN` start gate, drive `OboeBridge.nativeOboeOpen`
      with the runtime's engine token, stop/drain via the runtime.
      Keep: `NetworkSessionLock` (Wi-Fi low-latency lock — Android platform
      adapter, stays Kotlin), UI state mapping, `received_gap` logging.
      Delete: `pendingPackets` buffering (the runtime buffers), Kotlin
      prefill/lead/depth constants, `DebugPcmRecorder` wiring (2.5
      replaces it; keep the Kotlin class only if the BLE path still needs
      it until 4.x).
- [x] 3.2 Surface runtime diagnostics in the existing summary log line and
      (minimally) in `ManualConnectUiState` where the UI already shows
      playback state — failures must stay visible per CLAUDE.md.
- [x] 3.3 Update/replace controller unit tests (`computeRingPrefillMs`
      tests move to Rust with 2.7; event-mapping and gating tests stay).
- [x] 3.4 Android gate green (`./gradlew test lintDebug`), plus
      `assembleDebug` for all ABIs.
- [x] 3.5 Device validation protocol (below) — must pass before 4.x.

  **PASSED on 2026-08-02 (SM-A546E, run 9).** First run of the fully
  Rust-owned pipeline, live-listened ("that sounded much better"):

  - WAV: **39.96s, zero sample-level discontinuities, zero silence gaps**
    (max sample jump 822 = ordinary waveform slope). Every prior run had
    5-12 gaps; the best previous was 158ms across 9 gaps.
  - Accounting exact: `received=1990 accepted=1990 emitted=1990`,
    `late=0 duplicate=0 reorderWindow=0 hardResyncs=0`. Eight packets were
    lost and all eight concealed by repetition — which is precisely why no
    gap appears in the recording.
  - `ringUnderruns=68` (6528 frames, ~136ms), down from 1257 in the last
    Kotlin run. Sync locked 144ms after stream start, which matches the
    silence-filled total almost exactly, so these are very likely the
    startup window before the pump could write. **Inference from the
    correlation, not proof** — a timestamped underrun counter would settle
    it, and is the obvious next diagnostic if the startup transient
    matters.
  - `ringPeakFrames=48000` is the stop-time drain, which deliberately
    queues the whole buffered tail at once and bypasses the depth cap
    (verified by reading `finish()`); it is also why the full final note
    survives. Not the cap failing mid-stream.
  - `prefillFrames=0`: correct here. The startup buffer had already filled
    by the time sync locked, so the first frame was due immediately and
    needed no alignment silence.

  **Caveat worth carrying forward:** the WAV records what the pump released
  toward the ring, so it cannot show ring underruns, which happen
  downstream. "Zero gaps in the WAV" means the pump produced continuous
  audio, not that nothing was audible.

## Phase 4 — Retire the legacy Kotlin pipeline everywhere

The legacy scheduler has three consumers; after Phase 3 only the manual
path is migrated. Kotlin must not remain a competing owner.

- [x] 4.1 Migrate `MainViewModelListenerPlayback` /
      `MainViewModelRustListener` (BLE / Wi-Fi Direct discovered-session
      listener path) onto the same runtime handle.
- [x] 4.2 Delete `ListenerPlaybackScheduler`, `AudioPacketBuffer`,
      the PCM16 helper functions, and their tests (behavior now covered by
      Rust tests). Delete `AudioTrackPlaybackEngine` (already marked
      legacy/unused) and shrink the `PlaybackEngine` interface to what
      actually remains — possibly nothing beyond the Oboe open/close
      bridge; if the interface dies entirely, that is the correct outcome.
  - [x] `OboePlaybackEngine` reduces to Oboe stream lifecycle around the
        runtime's engine token (no `write`, no PCM conversion, no
        prefill); rename if clarity improves.
- [x] 4.3 Delete Kotlin `DebugPcmRecorder` once no path uses it.
- [x] 4.4 Sweep for dead constants/imports; Android gate green.

  **Done. Two scope findings worth carrying forward:**

  1. **The host self-monitor path still uses `PlaybackEngine`/`PlaybackFrame`
     and `OboePlaybackEngine.write`.** This plan assumed those could shrink to
     nothing once the listener migrated, but `MainViewModelHostPlayback`
     renders locally decoded audio through the same interface. So
     `PlaybackEngine`, `PlaybackFrame`, and the engine's write path survive,
     while the listener-only pacing hooks (`prefillSilence`,
     `queuedDepthFrames`) are gone. Migrating host monitoring onto the Rust
     runtime is a genuine follow-up this plan did not account for — recorded
     in 5.x.
  2. **The debug-only demo session no longer fakes audio.** It previously
     synthesized packets and ran them through the real scheduler, which would
     have required an offset-injection API purely for demo purposes — and
     would make a fake session indistinguishable from a real one in every
     diagnostic that matters. It now walks the UI progress states and says so
     in its own message. Gated on `BuildConfig.DEBUG` and the demo id prefix
     as before.

  Deleted: `ListenerPlaybackScheduler`, `AudioPacketBuffer`,
  `BufferedAudioPacket`, the Kotlin PCM shaping helpers, `SilenceFiller`,
  `AudioTrackPlaybackEngine`, Kotlin `DebugPcmRecorder`,
  `recordIncomingPacket`, the pending-packet buffers, and the tests that
  existed only to exercise them.

## Phase 5 — Documentation and follow-ups

- [x] 5.1 Update `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` (Block 18
      deviation note now closed; Block 23 decision partially realized) and
      `memory.md` per its format.
- [x] 5.2 Record the desktop-listener implication: `Future C` (desktop as
      listener) should consume this same runtime; note it in
      `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` at the Future C item.
- [x] 5.3 Explicit follow-up (do NOT do inline): route audio/sync events
      to the runtime *inside* the Rust listener transport so they never
      cross to Kotlin at all (removes the per-packet UniFFI forwarding
      from 2.6). Keep as a separate block with its own device validation.

  **Implemented in the later playback hardening pass.** Both manual and
  discovered listener transports now attach the playback runtime inside Rust,
  so audio datagrams travel transport -> scheduler/pump without a per-packet
  Kotlin/UniFFI round trip. Kotlin receives control/lifecycle events only.
  Transport-to-playback failure is fail-visible rather than counted as a
  successful forward. Physical re-validation remains in the device gate below.
- [x] 5.4 Explicit follow-up: slew-limited mid-stream offset updates
      (scheduler `apply_offset_update` exists; the manual path currently
      freezes the mapping per stream — long streams will eventually need
      gentle correction bounded well under the 120 ms hard-resync
      threshold).

  **Implemented in the later playback hardening pass.** Accepted finite
  sub-threshold offset changes now slew by at most 5ms per observation and
  converge over repeated sync samples; only changes above the hard-resync
  threshold rebuffer. Regressions cover one-step bounding, convergence, exact
  threshold semantics, and non-finite rejection. Physical listening remains
  in the device gate below.

- [x] 5.5 **New follow-up, discovered in Phase 4:** migrate the *host*
      self-monitor path (`MainViewModelHostPlayback`) off Kotlin's
      `PlaybackEngine`/`PlaybackFrame` and onto the same runtime. This plan
      assumed the listener was the only consumer; it is not. Until then
      `PlaybackEngine`, `PlaybackFrame`, and `OboePlaybackEngine.write`
      survive for the host's own monitoring, which is a smaller but real
      remaining split of ownership.

  **Implemented in the non-device closure pass.** Android host self-monitoring
  now opens `FfiListenerPlaybackHandle`, locks its same-process host clock in
  Rust, submits the same host packets into the Rust scheduler/pump/ring,
  reanchors that runtime across host pauses, and updates gain through the Rust
  pump. Normal start/stop/drain runs off Android main. `MainViewModel` no longer
  references `PlaybackEngine` or `PlaybackFrame`; the legacy engine types remain
  only as isolated core/audio regression surfaces and are not a production
  playback owner. Rust regressions cover same-process clock lock and dynamic
  volume validation/scaling.

## Device validation protocol (Phase 3.5 gate)

The exact workflow that found and verified everything above; commands and
gotchas are documented in `memory.md` 2026-08-02 entries.

1. Quiet the desktop machine (`./gradlew --stop`, kill Kotlin daemons,
   load < 1.5) — desktop CPU contention and phone Wi-Fi power save both
   masquerade as app bugs. The Wi-Fi lock (R-adjacent, already shipped)
   should reduce the latter; note whether multi-second arrival outages
   still occur.
2. `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml
   manual_real_android_listener -- --ignored --nocapture`, paste the
   printed JSON into the app's Connect manually screen via
   `adb shell input text` (escape `{}",`), tap Connect.
3. Let song-a play its full 40 s (the desktop test still panics at the
   song-change — pre-existing, unrelated, tracked separately).
4. Pull the newest WAV from
   `/sdcard/Android/data/com.ekkus.silentdisco/files/` and run the
   analysis (channels/duration, max sample-to-sample delta, silence-gap
   list — script pattern in memory.md).
5. Pass criteria (all previously achieved by the Kotlin path on
   2026-08-02; the Rust path must not regress any):
   - zero sample-to-sample discontinuities > 12 000;
   - total silence < ~300 ms with gaps only at logged `received_gap`
     losses, each ≤ ~16 ms (ramped) — no 20 ms hard holes;
   - steady-state `oboeUnderruns` ≈ 0 outside genuine arrival outages;
   - stall/backpressure events ≈ 0 (depth cap working);
   - duration ≥ 39.5 s with a clean start (prefill) when sync locks fast;
   - diagnostics summary line present and consistent with the WAV.
6. A human listens live to at least one run and hears no pops or static
   (brief soft dips at genuine loss points are acceptable).

## Out of scope

- Host-side/desktop streaming (already Rust; send-ahead horizon done).
- The desktop `stop_playback` pump-thread panic (tracked in the desktop
  TODO, Block 28 notes).
- Time-stretch/resampling correction strategies (explicitly deferred by
  CLAUDE.md until simple strategies prove insufficient).
- Multi-listener scale, FEC/retransmission, codec work.
