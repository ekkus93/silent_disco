# Gapless Queue Redesign Handoff

**Date:** 2026-10-16  
**Branch at handoff:** `master`  
**Starting HEAD:** `1753dac` (`docs: record desktop Tauri workflow coverage and fixes`)  
**Project checklist:** [Gapless Queue and Workflow Redesign TODO](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md)  
**Implementation record:** [Implementation notes](SILENT_DISCO_GAPLESS_QUEUE_IMPLEMENTATION_NOTES_2026-10-15.md)

## Current state

The queue and workflow redesign is substantially implemented across Rust core/FFI, Android, and desktop. The current worktree contains the in-progress implementation and documentation updates for the checklist above. These changes were present but uncommitted at handoff; the handoff document itself is committed separately.

Rust owns bounded queue state and protocol semantics. Protocol v3 carries sample-indexed track boundaries over one continuous stream. Android and desktop stage files in their platform layers and use the shared Rust playlist packetizer. Queue edits are reconciled before a boundary enters the output queue: stale targets are rejected and retargeted, and later boundary commits wait for the previous boundary to apply. Regression tests cover stale packet prevention, retargeting, serial transitions, and replay behavior. Desktop's end-to-end host workflow crosses a multi-track queue, and a stop/replay regression verifies that playback resumes from the same current track.

The implementation is **not yet qualified as gapless on physical audio devices**. Virtual tests, emulator instrumentation, and application builds establish substantial coverage, but the project checklist correctly keeps device playback acceptance open.

## Validation recorded for the current implementation

The following local checks completed successfully:

- `RUST_TEST_THREADS=1 bash scripts/check-rust.sh` — Rust formatting, strict Clippy, workspace tests, integration tests, and doc tests passed (369 core unit tests and 66 FFI unit tests, plus the rest of the workspace suites).
- Android `./gradlew testDebugUnitTest lintDebug assembleDebug connectedDebugAndroidTest --stacktrace --console=plain` — passed on API 36; 120 instrumentation tests passed and one was skipped.
- Android `./gradlew assembleRelease --stacktrace --console=plain` — passed.
- Focused API 36 `HostPlaylistNativeStreamTest` — 2 tests passed.
- Desktop Tauri Rust `CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 cargo test --locked --all-features` — 348 passed, 5 ignored.
- Desktop Tauri Rust strict Clippy and formatting checks — passed.
- Desktop `npm run check` — frontend checks, 187 tests, bindings check, and production build passed.
- `bash scripts/check-source-file-line-counts.sh` — 656 tracked files checked; all under the configured limit.
- `git diff --check` — clean.

These are local results, not a claim that hosted CI ran successfully against the final exact commit. The full commands and evolving test counts are recorded in the implementation notes and TODO.

## Remaining work

The authoritative task-by-task list is in the TODO. Open items are:

1. **Cross-platform scheduler race coverage:** add one combined Android/desktop virtual scheduler scenario and exercise remaining pause/resume/edit race permutations. Existing production scheduler tests are separate, with deadline calculations compared across platforms.
2. **Design system and motion:** finish component-level token coverage and broader Android motion/reduced-motion behavior.
3. **Accessibility and layout acceptance:** verify platform insets/system bars, non-primary touch targets, broad workflow traversal, system font scaling across flows, landscape, and hands-on screen-reader/focus behavior. Some desktop focus/reduced-motion/contrast and primary Android touch-target checks already have automated coverage; see TODO §8 for detail.
4. **Visual regression coverage:** add Android screenshot/golden coverage for reviewed workflow states. Desktop has deterministic accessibility-outline snapshots; pixel screenshots still require a suitable WebView runner.
5. **Playback qualification on devices:**
   - Run packaged desktop-host to Android-listener playback with multiple tracks and different source formats; capture boundary diagnostics and confirm no output-device reopen.
   - Run Android-host to Android-listener transitions and desktop-listener acceptance if supported.
   - Exercise add/reorder/remove during playback, pause/resume at a boundary, Stop/Play, queue end/add/Play, late join, reconnect, and recovery after failure on real or otherwise qualified audio paths.
   - Record sequence/sample-index continuity, concealment, boundary scheduled/applied counters, host discontinuities, and output open/close counters as specified in the TODO.
6. **Exact-head closeout:** reconcile every checklist item with evidence and run hosted CI for the final pushed source when GitHub Actions is available.

The TODO records two known external constraints: physical test devices were unavailable, and recent GitHub Actions runs failed before any job steps or were blocked by the repository Actions budget. Treat both as pending prerequisites, not passing checks. Gapless playback must not be called complete until the device acceptance criteria and exact-head closeout are satisfied.

## Suggested next steps

1. Continue with TODO §4's combined virtual scheduler and remaining race permutations, updating the checklist and implementation notes as evidence lands.
2. Complete the remaining automated UI/accessibility and Android golden work in §§8 and 5–7.
3. When qualified devices/audio paths and Actions capacity are available, execute §9's acceptance matrix, retain diagnostic logs, and close the checklist only against that evidence.

## Working tree note

At handoff, the implementation changes and TODO/implementation-note edits were still in the shared worktree and were not included in this documentation-only handoff commit. Preserve and review those changes before creating an implementation commit. The untracked `.serena/` directory was also present and was not included.
