# Silent Disco — Gapless Queue and Workflow Redesign Handoff

**As of:** 2026-10-22 (America/Los_Angeles; 2026-10-23 UTC)
**Repository:** https://github.com/ekkus93/silent_disco
**Branch:** `master` (direct-to-master development; no new branch or PR)
**Master head inspected at handoff commit:** `799e048f15f63f03c3ec26b45d70e335d41c8ff3`
**Authoritative worklist:** `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md`
**Specification:** `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_SPEC_2026-10-15.md`
**Producer race design:** `docs/SILENT_DISCO_PRODUCER_BOUNDARY_COMMIT_PROTOCOL_2026-10-16.md`
**Developer guidance:** `AGENTS.md` and `README.md`

> Handoff, not acceptance signoff. Source and hosted software checks cover much of the implementation, but uninterrupted real-device audio and the remaining UI/packaging acceptance gates have NOT been demonstrated. Reload the TODO and current `master` before resuming; the branch is being updated by another loop.

## 1. Executive status

The gapless multi-song host queue and workflow redesign is implemented across most of the Rust, Android, and desktop paths. The authoritative TODO currently has **53 checked / 15 unchecked top-level tasks** (68 total). Sections 0, 1, and 2 are checked, as are the core Android/desktop workflow implementations and the principal client join UI. **This redesign is not done.**

The host supports selecting multiple local audio files, queue append/reorder/remove, a locked playing entry, Play/Pause/Stop, natural queue end without ending the session, retry/remove error handling, and an authoritative Rust queue/stream state. Android uses Music → Access → Ready and Find → Connect/Wait → Listening flows; desktop has the corresponding host wizard/dashboard and queue controls.

Protocol v3 adds sample-indexed `TrackBoundary` and continuous PCM/sequence clocks. The producer is intended to reserve the successor with the authoritative actor before successor PCM is published; queue edits retarget prefetch. Listeners retain their playback runtime/output across a normal boundary instead of receiving per-track Stop/Start. Rust owns domain/protocol decisions, while Kotlin and Tauri/React own platform source/audio boundaries and presentation. Protocol v2 is not silently interoperable with v3.

### Most recent progress beyond the last fully green software baseline

- `f48614ddd6062c5f008596292486dc7de77ee50c` and formatting fix `326adef0650a53be554f7fd10b365514f4e9a9df`: a production-path UniFFI actor/edit-first backpressure regression removes a prefetched successor through a typed actor mutation with a 32-frame producer buffer, retargets from the accepted snapshot, checks the removed track never leaks boundary/PCM, and verifies the replacement boundary at sample 9,600 with 42 contiguous PCM packets.
- The subsequent `ce268955cd24cab79deeead3296e3776a40d00ea` extends that regression to Stop and replay: an old producer with a retired stream ID must fail closed when attempting successor reservation and must not change the new active stream's queue/boundary. The test allows existing current-song PCM before the rejected reservation, not stale successor audio.
- `799e048f15f63f03c3ec26b45d70e335d41c8ff3` applies pinned rustfmt to the new Stop/replay test. **Do not describe the latest regression as qualified until its exact-code-head Rust/Android workflow finishes green.**
- Revisit the current latest commit after this handoff: a different automated worker may advance `master` while CI is running.

## 2. Verification evidence and its limits

**Last confirmed complete applicable software CI baseline before the newest Stop/replay test:**
- Exact code head `326adef0650a53be554f7fd10b365514f4e9a9df`: [Rust/Android CI 39899297244](https://github.com/ekkus93/silent_disco/actions/runs/39899297244) succeeded (pinned Rust fmt, strict Clippy/workspace/FFI tests; Android JVM/build/lint; independent API 29 emulator instrumentation; Android candidate publication).
- [Desktop CI 39899297217](https://github.com/ekkus93/silent_disco/actions/runs/39899297217) succeeded (frontend/Tauri and Linux bundle smoke).
- [Source-line and runner-policy CI 39899297324](https://github.com/ekkus93/silent_disco/actions/runs/39899297324) succeeded.
- Enumerated unexpired **at that checkpoint**, not guaranteed permanent: Android release-candidate artifact `12478004158` (35,160,342 bytes); instrumentation diagnostics `12478671624`; desktop Linux bundle `12478626414` (214,641,826 bytes). Recheck artifacts and rebuild from one identical SHA before physical tests.

**Newer CI observed during preparation of this handoff:**
- `ce268955`: source-line run `39902820137` passed; Rust/Android run `39902820144` was in progress; desktop run `39902820100` subsequently showed cancelled during another master update. Do not count it as a fully green head.
- `799e048f`: source-line run `39903060305` passed; Rust/Android run `39903060337` was pending; desktop run `39903060283` was in progress at inspection; later status may differ. Refresh exact-head CI before making an acceptance claim.
- A successful `Publish CI Status` workflow is not equivalent to all code build/test jobs succeeding. Do not reuse status from an older SHA to qualify a newer SHA.

Previous checkpoints passed significant core/FFI integration, desktop/frontend, Android JVM and API 36 instrumentation coverage (including 119 passing, one skipped in an earlier 120-test instrumented suite). Exact-head builds and emulator tests are useful **software evidence**, not physical playback measurements. Historical Oct 17 CI jobs failing before job steps and older runner/tool profile errors documented in the TODO are not necessarily current blockers; verify current workflow outcomes instead.

## 3. Outstanding work — the 15 unchecked TODO items

The TODO's own checkboxes are authoritative. The entries below paraphrase the unfinished acceptance, including partially implemented items; do not toggle a checkbox because a mockup, source-level test, or emulator-only run passed.

| TODO | Incomplete acceptance / practical next step |
| --- | --- |
| **`3` producer/prebuffer edit race** | Close all producer/actor reservation commit-point race permutations: edit-before-reserve, edit-after-reserve, backpressured buffered frames, stale successor under concurrent Stop/replay, and cancellation/error classification. Recent FFI tests reduce risk but do not fully qualify every physical or concurrent path. The actor must never approve a removed successor or leak its audio. |
| **`4` late join/reconnect** | Verify current track, correct current timeline, cached upcoming boundaries, pause state, clock re-anchor, and title reconciliation across production Android/desktop and **physical** disconnect/reconnect. Existing socket cache and virtual tests are substantial but insufficient. |
| **`4` pause/resume and Stop/replay** | Run multi-track production/output tests at and near boundaries: no timeline drift, no replay of retired boundaries/PCM, Stop preserving queue, Play restarting current song, and no needless listener output reopen. |
| **`4` late decode/read failure** | Qualify device-path failure after partial audio, exact audible-tail Stop timestamp, preservation of failed item and Retry/Remove, and correct post-recovery playback; no silent skip. |
| **`4` deterministic production integration** | Finish combined Android/desktop production scheduler tests plus remaining edit/pause/Stop/backpressure interleavings; check sample continuity, boundary time, bounded prebuffer, and stale-frame suppression in the actual host path. |
| **`5` Android workflow/UI tests** | Broaden Compose and state tests for process/configuration changes, live edits, empty queue, recovery/navigation, accessibility/large text and compact screens. |
| **`6` desktop workflow tests** | Complete frontend/Tauri tests and acceptance for transactional staging, live transitions, failure recovery, and session cleanup under production packaging/lifecycle. Many existing integration tests already pass. |
| **`7` client join/listening tests** | Broaden Android reconnect/approval/timeout/late-join presentation and cross-platform host/listener timing, viewport, and accessibility checks. Desktop hosts; no imaginary desktop listener acceptance if unsupported. |
| **`8` shared tokens** | Finish Material 3 Expressive component token adoption and Android motion/reduced-motion behaviors beyond already migrated workflow surfaces. |
| **`8` accessibility/layout** | Audit physical screen reader, focus, contrast, every touch target, large text, orientation, system bars/insets, desktop keyboard and compact window, and reduced motion. Existing 48 dp and color-token tests do not cover every component. |
| **`8` visual regressions** | Add Android screenshot/golden coverage for reviewed states and real pixel screenshots for desktop if an appropriate webview runner is available; current desktop outline snapshots are not equivalent to pixel goldens. |
| **`9` desktop-host → Android-listener** | Run **packaged** Linux desktop host against physical Android listener(s) with heterogeneous audio files, multiple boundaries, measured zero inserted/duplicated frames, and no output close/reopen. |
| **`9` Android-host → Android-listener** | Run physical Android-to-Android heterogeneous multi-track gapless path, collect output counters; only include desktop-listener acceptance if that product path is genuinely supported. |
| **`9` device race/transport matrix** | Exercise live add/reorder/remove, pause/resume right at boundary, Stop/Play, queue end/add/Play, late join/reconnect, and read/decode failure/recovery with real or otherwise qualified devices/audio paths. |
| **`9` reconciliation/closeout** | For each open checkbox attach exact code SHA, commands/logs, CI, real-device evidence, spec acceptance, and any residual exclusions; do not promote software CI to physical signoff. |

## 4. Highest-priority continuation plan

**Priority A — finish and pin source-level correctness**
1. Resolve current exact `master` with Ralph Bridge, re-read the authoritative TODO, and inspect all CI triggered by the newest Stop/replay regression. Fix failing formatting/tests directly on `master` without new PRs/branches; rerun or wait for matching exact-head CI. Avoid making the previous head's green checks stand in for current-head verification.
2. Extend the **same production** actor ↔ UniFFI producer ↔ host transport integration path across the outstanding interleavings. In particular, assert actor and producer agree on the boundary reservation point under deep output backpressure, after accepted retargeting, after schedule commit, and across Stop/new stream generations. Tests must assert actual PCM values/indices/sequence, not merely emitted state snapshots.
3. Run and capture pinned Rust formatting, Clippy, workspace and FFI tests; Android JVM/lint/emulator; desktop frontend + Tauri/bundle; source-line and runner-policy checks. Fix real failures rather than marking a TODO item complete from source inspection.

**Priority B — real device audio acceptance (owner hardware required)**
1. Obtain a packaged Ubuntu 22.04/24.04 amd64 desktop host and Android APK from the **same** green commit, with at least two physical Android devices for the `README.md` interoperability matrix. Connect phones via ADB, verify LAN discovery/routing and audio output.
2. Use the existing `README.md` “Physical desktop-to-Android interoperability” workflow and `desktop/scripts/validate-block47-android-interoperability.sh` for broader join/transport/session evidence. Block 46.3 fresh-machine package validation is separate: `desktop/scripts/validate-block46-fresh-machine.sh`. These checks do not themselves prove the new gapless boundary requirement; explicitly extend/log a playlist-boundary measurement run.
3. For both desktop-host/Android-listener and Android-host/Android-listener, play at least three heterogeneous supported files (different sample rates/channels/containers where supported); capture sample-index and sequence monotonicity, host boundary scheduled/applied counters, dropped/concealed/duplicate frames, output stream opens/closes/rebinds, wall-clock/presentation-time alignment, and audible output. Test pause, edits, Stop/replay and reconnect with the output still active.
4. **Acceptance criterion:** next track begins exactly after prior decoded sample's end (no inserted or overlapping frames); no output stream close/reopen attributable to a track boundary; no boundary discontinuity attributable to transition; all peers agree on correct current/upcoming item and no retired successor plays. Record host/client logs and diagnostics with exact SHA, device details, OS, file formats, and reproduction steps. Failure or absence of measurement leaves `9` open.

**Priority C — polish, qualify and close**
1. Finish `5–8` Android/desktop UI navigation, accessibility, layout, golden and resilience gaps; exercise physical TalkBack/keyboard/rotation and reduced motion.
2. Perform packaged fresh-install/restart/cleanup and other lifecycle validation. Confirm Lab Mode remains absent from normal production bundles.
3. Reconcile `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md` and supporting acceptance records **only after evidence exists**. Close each checkbox individually, then issue an end-to-end release readiness summary.

## 5. Key architecture invariants and known risks

- Rust actor is the sole authoritative queue, revision, current entry, playback state, stream identity, and boundary reservation owner. Both host adapters send typed mutations and apply the accepted snapshot; stale revisions are errors, not best-effort reorders.
- Source identities are opaque at the Rust layer. Platform URI/path grants/staging and cleanup remain in the platform adapter. Maximum queue is **100 entries**, **16 GiB** of unique queued source bytes, at most **8 GiB** per source, and bounded decoder/prebuffer capacity.
- Producer and actor must linearize successor reservation **before** successor packet emission. A queued boundary can be buffered ahead of the sender by ~32 frames; a consumer-side-only “scheduled” event is too late to protect the edit race. Keep a single stream generation authoritative so a delayed old producer cannot mutate or send successor audio into a new playback run.
- Transport timeline is continuous decoded PCM frame/sample indices and monotonic packet sequences; a `TrackBoundary` is metadata at the first sample of the new entry, not a Stop/Start or a container duration estimate.
- Pause re-anchors the presentation clock; Stop cancels boundary jobs and preserves the active queue; natural end leaves a ready session but requires explicit Play after new additions. Failed audio must be surfaced with recovery rather than silently skipped.
- Listener native output/ring/runtime should survive a normal boundary. Emulator Oboe rebind checks do not establish physical zero-gap output on arbitrary Android hardware.
- Preserve authorization/approval, late-join replay order, pause/reconnect correctness, and cancellation safety; avoid send-ahead or monitor-only frames after Stop or a failed network handoff.

## 6. How to resume / handoff constraints

- **Repository access:** use **Ralph Bridge only** for all GitHub and CI reads/writes, and commit directly to `master` with exact-head compare-and-swap; do not use the default GitHub connector, create one branch per task, or open per-task PRs.
- Read `AGENTS.md`, the redesign spec/TODO, producer-reservation protocol note, existing playback review, and `README.md`. Check the live head and CI again; this snapshot goes stale immediately on another commit.
- For code changes, add real regression coverage, keep UniFFI bindings generated, and qualify with the repository-pinned toolchains. For documentation, keep the TODO honest about open gates.
- Hardware/device tests require user access to real devices, packaged artifacts, and audio diagnostics. If not available, classify physical acceptance as **blocked pending hardware**, not passed or skipped. The primary user assistance needed is device connection and execution/collection of those physical measurements; additional development can continue independently.
- Do not equate a CI status-publication success, an emulator playback test, or the mockups with product acceptance. A passing candidate build is available for **testing**, not production signoff.

## References

- [Authoritative redesign TODO](./SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md)
- [Redesign specification](./SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_SPEC_2026-10-15.md)
- [Producer boundary reservation protocol](./SILENT_DISCO_PRODUCER_BOUNDARY_COMMIT_PROTOCOL_2026-10-16.md)
- [Project setup / physical acceptance commands](../README.md)
- [Architecture and CI conventions](../AGENTS.md)
- [Previous fully green code-head CI](https://github.com/ekkus93/silent_disco/actions/runs/39899297244)
