# Gapless Queue and Workflow Redesign TODO

**Spec:** `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_SPEC_2026-10-15.md`  
**Visual review:** `docs/mockups/SILENT_DISCO_WORKFLOW_MOCKUPS_2026-10-15.html`  
**Status:** Draft for product review; implementation has not started.  
**Target:** `master`

This checklist is dependency-ordered. Complete and qualify each block before moving to dependent work. Keep Rust authoritative for domain queue/protocol behavior; Android and desktop own presentation and platform audio-file boundaries. Do not mark a task complete from a mockup, state-only unit test, emulator-only run, or a CI run that did not test the relevant behavior.

## 0. Handoff and exact baseline

- [ ] Re-read this TODO, the companion spec, `AGENTS.md`, Rust core architecture spec, desktop host spec, playback review docs, and existing UI/UX progress ledger from the current exact master head.
- [ ] Inspect master, related branches/open PRs, and exact-head CI before implementation; reuse or reconcile existing work rather than duplicate it.
- [ ] Record the baseline master SHA, Android/desktop/Rust test status, and current desktop/Android single-track behavior in the implementation notes.
- [ ] Review the HTML workflow mockups with the product owner and capture accepted copy/layout changes before UI implementation.

## 1. Gapless feasibility and protocol design

- [ ] Trace host source staging, decode, packetization, stream IDs, sequence numbers, transport send-ahead, natural EOF, listener scheduler, render ring, Oboe lifecycle, desktop monitor/output, and late-join behavior.
- [ ] Write a concise implementation note describing how a track boundary can be represented on the current protocol without stopping/reopening listener output.
- [ ] Select the minimum compatible protocol/runtime change needed to keep one sample timeline continuous across track boundaries; document version negotiation and mixed-version behavior.
- [ ] Define the boundary as contiguous decoded sample indices after conversion to the negotiated PCM format; avoid relying on container duration rounding.
- [ ] Define measurable zero-gap/no-overlap thresholds for virtual tests and physical/device acceptance, including the exact diagnostic counters used.
- [ ] Define explicit queue entry and source staging limits based on current per-platform file limits, available storage, memory, and bounded decoder/prebuffer queues.

## 2. Rust authoritative playlist state

- [ ] Add bounded ordered queue entries with unique entry IDs, opaque source IDs, current entry identity, and queue revision to Rust domain state.
- [ ] Keep queue entry IDs distinct when a host intentionally adds the same source more than once; stage shared content only once when safe.
- [ ] Add typed commands for append, reorder upcoming entries, remove upcoming entries, clear/end session, retry failed preparation, and remove a failed item.
- [ ] Reject mutation of the current item while it is active; validate queue indices/IDs and command revisions atomically.
- [ ] Define queue snapshot fields for current/upcoming items, preparation/readiness/error state, and the next boundary; expose only bounded display metadata.
- [ ] Clear queue and release platform references on session end/profile close. Define recovery if a live session cannot restore its queue after process loss.
- [ ] Add Rust tests for ordering, duplicates, bounded capacity, mutation races, current-item locking, natural end, queue end, and teardown.
- [ ] Add FFI and Tauri DTO/API mappings from Rust types. Regenerate bindings with the repository tooling; do not hand-edit generated bindings.

## 3. Stable source staging and prebuffer lifecycle

- [ ] Implement transactional multi-file selection/staging on Android and desktop; a failed item does not corrupt or partially replace the existing queue.
- [ ] Keep native Android URIs and desktop paths inside their platform adapters. Send opaque IDs and bounded metadata to Rust.
- [ ] Retain Android document grants or stage copies so queued items remain available for the live session; verify cleanup behavior.
- [ ] Retain desktop staged sources for each queue entry and release them with the session/profile lifecycle.
- [ ] Validate supported container, length, metadata, and decoder viability before queue acceptance; report item-specific errors.
- [ ] Add a cancellable preparation worker for the next queued track. Keep source/decode/PCM queues bounded and off the network/audio real-time paths.
- [ ] Normalize adjacent tracks to a shared output format and continuous sample clock. Test sample-rate/channel changes and differing source formats.
- [ ] Make queue edits cancel/retarget the next-track prebuffer safely without stale packet leakage.
- [ ] Test source removal, revoked access, corrupt/truncated input, unsupported format, storage exhaustion, worker cancellation, and decoder failure.

## 4. Continuous gapless host/listener playback

- [ ] Implement host lookahead so the next track is decoded and prebuffered before the current track reaches its exact sample boundary.
- [ ] Schedule next-track audio at the previous track's final sample boundary, with no inserted silence or crossfade.
- [ ] Preserve listener playback runtime, render ring, Oboe stream, and desktop audio output across a track boundary; do not route audio packets through Kotlin.
- [ ] Deliver track identity/boundary state to clients without resetting clock synchronization or tearing down the audio sink.
- [ ] Update host and client current-track presentation at the scheduled boundary.
- [ ] Keep late join and reconnect behavior truthful: a client joins at the current track/current timeline and receives the correct upcoming boundary.
- [ ] Preserve pause/resume sample timeline and Stop/replay semantics across queued tracks.
- [ ] At queue end, stop audio and leave the host session open. A newly added song requires explicit Play.
- [ ] On an exceptional late decode/read failure, keep playback running if possible; otherwise stop at a valid boundary, retain queue state, and expose Retry/Remove failed item. Never silently skip.
- [ ] Add deterministic virtual tests that assert contiguous sample indices, no overlap, bounded prebuffering, consistent host/client boundary time, pause/resume continuity, edit cancellation, and failure behavior.

## 5. Android host experience

- [ ] Implement the 3-step Host wizard: Music → Access → Ready, using the reviewed mockup direction.
- [ ] Support multi-select, ordered playlist preview, remove/reorder before session start, and concise per-file validation.
- [ ] Keep primary wizard navigation/actions visible at minimum supported screen size; split overflowing tasks into steps/screens instead of page scrolling.
- [ ] Implement live host screen with always-visible current song/progress/transport, Add songs, Manage queue, listener count, and invite route.
- [ ] Implement dedicated queue management with current item locked, upcoming reorder/remove, pinned actions, and visible queue count.
- [ ] Add Compose/UI-state tests for setup, live append, reorder/remove, empty queue, transport state, error recovery, navigation, compact layout, large text, accessibility, and process/configuration changes.

## 6. Desktop host experience

- [ ] Implement the same Host wizard stages and queue behavior in React/Tauri.
- [ ] Support multi-select and show first/current/upcoming songs clearly during setup.
- [ ] Implement a fixed-viewport live dashboard with current track, progress, Play/Pause/Stop, Add songs, Manage queue, listener count, and invite action.
- [ ] Implement queue management with pinned actions and safe reorder/remove of upcoming entries.
- [ ] Ensure minimum supported desktop window size presents primary controls without page scrolling or hidden actions.
- [ ] Add frontend and Tauri tests for source selection, transactional staging, queue commands, live edits, track transitions, failures, and session cleanup.

## 7. Client join/listening experiences

- [ ] Implement the 3-stage Client wizard: Find → Connect/Wait → Listening, preserving existing QR, discovery, approval, and trust rules.
- [ ] Keep discovery, QR scan, selected session, invite code, waiting approval, cancel, and recovery actions discoverable in fixed workflow screens.
- [ ] Navigate to Listening only after approval/admission, accepted clock synchronization, and playable readiness.
- [ ] Present current song and synchronized track changes without leaving the Listening screen.
- [ ] Keep client controls limited to local volume, leave, and client-side recovery; clients cannot control the host queue.
- [ ] Add Compose/React navigation, accessibility, small viewport/window, approval, timeout, late-join, reconnect, and track-boundary tests.

## 8. Shared design system and accessibility

- [ ] Translate Material 3 Expressive color, type, shape, elevation, spacing, component, focus, and motion tokens into Android and desktop equivalents.
- [ ] Use concise action labels and one-line empty states. Remove engineering copy from normal host/client paths; retain technical details in diagnostics.
- [ ] Keep task navigation and primary actions fixed. Any long collection is placed in a dedicated screen with pinned actions and visible position/count.
- [ ] Verify keyboard/focus order on desktop, screen-reader semantics, contrast, touch target sizes, large text, reduced motion, compact windows, landscape, and Android system bars/insets.
- [ ] Add screenshot/golden coverage for each reviewed workflow state on Android and desktop; update mockups only after product review.

## 9. Qualification and closeout

- [ ] Run focused Rust tests, formatting, strict Clippy, and full workspace tests.
- [ ] Run Android JVM/Compose tests, lint, debug/release assembly, and managed instrumentation; record physical-device results separately.
- [ ] Run desktop generated-binding check, formatting, lint/type-check, frontend tests/build, strict Tauri Clippy, and Rust tests.
- [ ] Test a packaged desktop-host to Android-listener session with different source formats and multiple tracks; verify measured gapless boundary and no output-device reopen.
- [ ] Test Android-host to Android-listener track transitions and desktop-listener acceptance if supported.
- [ ] Test add/reorder/remove during playback, pause/resume at boundary, Stop/Play, queue end/add/Play, late join, reconnect, and failure recovery on real or qualified device/audio paths.
- [ ] Reconcile every checklist item against exact-head CI, test logs, device evidence, and the specification. Do not report gapless playback complete from UI tests alone.
- [ ] Update the existing UI/UX progress ledger with the new scope and preserve its prior completion evidence.
