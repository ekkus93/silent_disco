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

- [ ] 1.1 Replace silence synthesis in `PlaybackScheduler`/`ConcealmentPolicy`
      with decaying-repetition concealment (R4):
  - [ ] Track the last real packet's samples and the consecutive-concealment
        generation inside the scheduler.
  - [ ] Synthesize: source samples attenuated by `>> (generation + 1)`
        (cap 8), entry ramp over 5 ms blending from the previously played
        frame's final sample values, tail fade over the final 5 ms to zero.
        Integer PCM16 math; port from Kotlin
        `pcm16LeConcealmentPayload` / `pcm16LeLastFrame`
        (`PlaybackScheduling.kt`) — but operate on `i16` samples directly,
        no byte packing.
  - [ ] `ConcealmentOutcome::HardResyncRequired` semantics unchanged.
- [ ] 1.2 Fade-in on resume and stream start (R5): first real frame after
      any concealed frame, and the first frame ever delivered, get a 5 ms
      linear fade-in applied to their samples before delivery.
- [ ] 1.3 Wide-hole skip policy (R6): when the next buffered sequence is
      more than `concealment_skip_threshold_packets` (config, default 10)
      ahead of the expected sequence, skip the hole via
      `JitterBuffer::skip_expected_sequence` (repeatedly, or add a
      `skip_to(sequence)` helper), reset the concealment run, and mark the
      next real frame for fade-in. No concealment frames for the skipped
      range.
- [ ] 1.4 Drain-with-fades (R7): add a scheduler `drain_remaining()` that
      returns all buffered frames in sequence order ignoring deadlines,
      with fade-out applied to the frame before each sequence hole,
      fade-in after each hole (including a hole against the last
      live-delivered sequence), and a final-tail fade on the last frame.
- [ ] 1.5 Unit tests, mirroring the Kotlin suite: repetition content and
      exact decay values (8000 → 4000 → 2000 → 1000 for constant input),
      entry-continuity first sample, tail-zero last sample, fade-in on
      resume/first frame, bounded outage bridge (exactly the policy bound,
      then `HardResyncRequired`/stop — decide and document which), wide-hole
      skip delivering the post-hole frame directly, drain hole-edge fades,
      stale-arrival rejection staying intact.
- [ ] 1.6 `bash scripts/check-rust.sh` green.

## Phase 2 — Build the Rust listener playback runtime (scheduler → ring pump)

New component owning the data path from submitted packets to the render
ring. Suggested home: `rust/silent-disco-ffi/src/listener_playback.rs`
(worker thread is FFI-crate territory, matching the database worker
precedent), with any pure logic in `silent-disco-core`.

- [ ] 2.1 `ListenerPlaybackRuntime`: owns one `PlaybackScheduler`, one
      `RenderRingProducer` (+ registered engine token), and one dedicated
      pump thread. Explicit lifecycle: `start(config)`, `stop()` (drain +
      fades per R7, then release), `Drop` fail-visible like the database
      worker. No UniFFI/JNI/logging/allocation in the *real-time C ABI
      read path* — the pump thread is NOT the real-time thread and may
      allocate/log sparingly.
- [ ] 2.2 Pump loop (R8), all constants from the table above as config:
  - [ ] Poll cadence ~10 ms; monotonic clock injected (testable).
  - [ ] Release frames `write_lead_ms` ahead of mapped deadline.
  - [ ] Never push past `target_fill_frames` queued depth (query the
        producer; do not rely on full-ring backpressure).
  - [ ] At start: silence prefill of `(first frame's mapped local deadline
        - now)` clamped to `[0, max_prefill_ms]`, written before the first
        real frame, so the first frame plays at its deadline (the native
        callback consumes from stream open; play time = write time + queued
        depth).
  - [ ] i16 → f32 conversion (and volume scaling) happens here, once —
        Kotlin stops touching PCM entirely, closing Block 18's disclosed
        deviation.
- [ ] 2.3 Sync integration (R9): runtime accepts offset updates
      (`apply_offset_update` / `rebuffer` semantics already in the
      scheduler). Playback must not start before the first accepted
      update. Decide and document: Kotlin keeps forwarding sync-response
      timestamps to the Rust estimator (Block 6 exports exist) and pushes
      the resulting offset in, OR the runtime owns the estimator directly.
      Either way Kotlin performs no estimation math.
- [ ] 2.4 Diagnostics (R10): one snapshot struct — received, lost
      (arrival-continuity), late-dropped, concealed, skipped-hole count,
      current/peak ring depth frames, underrun + silence-filled counters
      (from ring telemetry), state (Buffering/Playing/AwaitingRebuffer/
      Stopped). Queryable via the control surface; runtime logs one
      summary line on stop.
- [ ] 2.5 Debug PCM tap (R11): optional config path; when set, runtime
      writes exactly the i16 frames it releases toward the ring (real,
      concealed, and drained — not the alignment prefill) to a WAV,
      finalized on stop. Same 44-byte-header format the Kotlin
      `DebugPcmRecorder` produced so the existing analysis tooling works.
- [ ] 2.6 UniFFI control surface (`FfiListenerPlaybackHandle` or similar):
      `open(config) -> handle`, `submit_packet(...)` (per-packet forwarding
      from Kotlin's existing transport event loop is acceptable control-
      plane load: ~50 calls/s), `apply_sync_offset(...)` (or sync-sample
      forwarding per 2.3), `engine_token()` for the Oboe adapter,
      `diagnostics_snapshot()`, `stop()`. Errors explicit and typed; no
      silent fallbacks.
- [ ] 2.7 Rust tests: pump pacing with a fake clock and a real ring
      (deadline alignment of first frame incl. prefill clamping both
      directions, steady-state depth converging to the lead, depth cap
      under a flooded backlog, drain-on-stop content), diagnostics
      correctness, lifecycle/double-stop/drop behavior.
- [ ] 2.8 `bash scripts/check-rust.sh` green.

## Phase 3 — Rewire the Android manual-connect path

- [ ] 3.1 `ManualListenerTransportController`: replace
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
- [ ] 3.2 Surface runtime diagnostics in the existing summary log line and
      (minimally) in `ManualConnectUiState` where the UI already shows
      playback state — failures must stay visible per CLAUDE.md.
- [ ] 3.3 Update/replace controller unit tests (`computeRingPrefillMs`
      tests move to Rust with 2.7; event-mapping and gating tests stay).
- [ ] 3.4 Android gate green (`./gradlew test lintDebug`), plus
      `assembleDebug` for all ABIs.
- [ ] 3.5 Device validation protocol (below) — must pass before 4.x.

## Phase 4 — Retire the legacy Kotlin pipeline everywhere

The legacy scheduler has three consumers; after Phase 3 only the manual
path is migrated. Kotlin must not remain a competing owner.

- [ ] 4.1 Migrate `MainViewModelListenerPlayback` /
      `MainViewModelRustListener` (BLE / Wi-Fi Direct discovered-session
      listener path) onto the same runtime handle.
- [ ] 4.2 Delete `ListenerPlaybackScheduler`, `AudioPacketBuffer`,
      the PCM16 helper functions, and their tests (behavior now covered by
      Rust tests). Delete `AudioTrackPlaybackEngine` (already marked
      legacy/unused) and shrink the `PlaybackEngine` interface to what
      actually remains — possibly nothing beyond the Oboe open/close
      bridge; if the interface dies entirely, that is the correct outcome.
  - [ ] `OboePlaybackEngine` reduces to Oboe stream lifecycle around the
        runtime's engine token (no `write`, no PCM conversion, no
        prefill); rename if clarity improves.
- [ ] 4.3 Delete Kotlin `DebugPcmRecorder` once no path uses it.
- [ ] 4.4 Sweep for dead constants/imports; Android gate green.

## Phase 5 — Documentation and follow-ups

- [ ] 5.1 Update `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` (Block 18
      deviation note now closed; Block 23 decision partially realized) and
      `memory.md` per its format.
- [ ] 5.2 Record the desktop-listener implication: `Future C` (desktop as
      listener) should consume this same runtime; note it in
      `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` at the Future C item.
- [ ] 5.3 Explicit follow-up (do NOT do inline): route audio/sync events
      to the runtime *inside* the Rust listener transport so they never
      cross to Kotlin at all (removes the per-packet UniFFI forwarding
      from 2.6). Keep as a separate block with its own device validation.
- [ ] 5.4 Explicit follow-up: slew-limited mid-stream offset updates
      (scheduler `apply_offset_update` exists; the manual path currently
      freezes the mapping per stream — long streams will eventually need
      gentle correction bounded well under the 120 ms hard-resync
      threshold).

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
