# Silent Disco — God-file refactor TODO

Spec: [SILENT_DISCO_GOD_FILE_REFACTOR_SPEC_2026-11-13.md](SILENT_DISCO_GOD_FILE_REFACTOR_SPEC_2026-11-13.md). Created 2026-11-13 at `487b35aa`.

Rules for every item, from spec §3 and §7:
- Behaviour is unchanged.
- Rust stays authoritative.
- One owner per piece of state.
- Each item is committed and pushed on its own, with all gates passing.
- Run the touched instrumented classes on the A54, always with `-Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true`.
- Use Conventional Commits, with no `Co-Authored-By` line.
- Record results in `memory.md` and the newest handoff.
- Mark only work that is done and verified.

Android gate: `./gradlew assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest test lintDebug`. When Rust changes, also run `bash scripts/check-rust.sh` and `cd desktop && npm run check`. Every item also runs `bash scripts/check-source-file-line-counts.sh`, which since 2026-11-13 only reports files over 800 lines; review any it lists for mixed responsibilities.

## 0. Owner decisions (spec §10)

- [x] Spec approved (owner, 2026-11-13, as written).
- [x] Order against the UI redesign: **(b)**, split `MainViewModel` now and leave all of `SilentDiscoApp` (section 6) to the UI redesign.
- [x] Kotlin timing state: **log for later**. Each confirmed item moves into its controller unchanged and is logged for a later Rust block. Section 5 is the "otherwise" branch only.
- [x] `runtime/types.rs`: **split only when it next grows**. The first item of section 7 is deferred; the watchlist item still applies.

## 1. Inventory (no code changes)

- [x] List all `MainViewModel` properties (68, not the 103 first estimated; spec Appendix A.1). For each one record its type, mutability, the threads and dispatchers that touch it, which of the 23 `MainViewModel*.kt` files read or write it, and its owner from spec §4. Commit the table as an appendix to the spec.
- [x] List the functions with their owners: 55 in `MainViewModel.kt` and 199 across the 23 files (spec A.2).
- [x] Record every cross-group read and write (spec A.3): state of one owner read or written by another group's code. Each one becomes a narrow interface method or an explicit one-way dependency.
- [x] Classify the Kotlin timing and domain candidates (spec A.4: T1 boundaries, T2 pre-runtime sync, T3 decoded audio; all logged for later per owner decision) as either presentation-only or domain/timing that belongs in Rust, citing the Rust owner where one exists: listener boundaries (`pendingListenerTrackBoundaries`, `appliedListenerTrackBoundaries`, `listenerTrackBoundaryJobs`), host boundaries (`hostPendingTrackBoundaries`, `hostBoundaryJobs`), `MainViewModelSynchronization.kt`, and `latestDecodedAudio` / `latestPackets`.
- [x] Map existing tests to owners (spec A.5; gaps: host monitor, persistence retry, the A.3 teardown sequences, `onCleared`): the 48 JVM test files in `app/src/test/.../app/`, the instrumented classes and the UI tests. List gaps.

## 2. Safety net

- [x] Write characterization tests for every gap from §1 (`MainViewModelRefactorSafetyNetTest`: host monitor stop order and idempotence, storage retry recoverable vs fatal, host stop cancels the stream and listener-role jobs, session end deletes staged audio; two mutations killed). Host *stop*, not start, cancels the listener-role jobs (spec A.3 item 1 corrected). Original wording: against the current `MainViewModel`. Each must pass before any move. Cover especially the cross-group sequences: approve, then authorize, then stream start; stop, rebind, then the stale join discarded; leave, then playback torn down; role switch, then jobs cancelled.
- [x] Add a test that `onCleared` cancels every job (`clearingTheViewModelStopsEveryServiceAndClosesEveryController`; needed a new seam: `MainViewModel` takes `manualListenerController`, and `ManualListenerTransportController` takes `closeNativeOutput`, both defaulting to today's behaviour). Original wording: and closes every executor and native handle `MainViewModel` holds today. The same test will later check each controller's `close()`.
- [x] Baseline run of the full instrumented suite on the A54 (2026-11-13 at `608313bf`: 180 tests, 4 failed, 1 skipped; the 4 failures are exactly the known layout ones below, and every ported v4 transport test passed). Original wording: Baseline run of the full instrumented suite on the A54. Record the result; the known failures (handoff §53.3) are 3 in `AdaptiveLayoutRemainingCoverageTest` and 1 in `SessionJoinPinnedStatusTest`.

## 3. Coordinator scaffolding

- [x] Add a small internal controller contract (`app/controller/ViewModelController.kt`: `ViewModelController` with a child supervisor scope and `close()` = cancel then `release()`; `ViewModelControllers` closes in reverse order and continues past failures; `MainViewModel.onCleared` closes it first; 4 tests, supervisor mutation killed). Original wording: a `close()`, an owned scope derived from `viewModelScope`, and `StateFlow` state. No behaviour moves yet.
- [x] Make `MainViewModel` combine controller state flows into `AppUiState`. **Done with `DiagnosticsController`**: the coordinator copies the controller's snapshot into `AppUiState` at refresh time, keeping today's synchronous timing. Previously deferred: with no controller state there is nothing to combine, and a mapping layer without a user would be speculative. Original wording: so far with no controllers. This proves the mapping pattern without changing behaviour.

## 4. Extract controllers, one per item, lowest coupling first

For each item:
- move its state and code out of `MainViewModel` and its extension files;
- delete the emptied extension files;
- point its tests at the controller;
- run the mutation check from spec §7 on any timing, ordering or recovery rule it carries.

- [x] `DiagnosticsController` (`app/controller/DiagnosticsController.kt`; 6 unit tests; full Android gate passed). It owns the store, metrics, bridge pressure, refresh and capture; the ~46 store writes go through `diagnostics.updateHost/updateListener`. `MainViewModelDiagnostics.kt` and `MainViewModelCallbackCapture.kt` are now thin wrappers. A refresh still copies into `AppUiState` synchronously (the deferred §3 combine item: the coordinator maps the controller's snapshot). `onCleared` finishes capture at its original point and closes the registry last. Original wording: diagnostics store, metrics, support report, developer callback capture (`MainViewModelDiagnostics.kt`, `MainViewModelCallbackCapture.kt`).
- [x] `PersistenceController` (`app/controller/PersistenceController.kt`; 2 unit tests; close-once mutation killed; full Android gate passed). It owns the domain store, which is no longer a `MainViewModel` property, and the audio read grants. The coordinator still maps storage status into `AppUiState`, and `onCleared` closes the store at its original point. Original wording: domain store access, persistence retry, persisted audio read grants (`MainViewModelPersistence.kt`).
- [x] `DiscoveryController` (`app/controller/DiscoveryController.kt`; 4 unit tests; the reconcile and scan-lifecycle tests pass unchanged; full Android gate passed). It owns radio start and stop, the scan window (`scanJob` removed) and reconciliation into the listener core. Failure routing and the Wi-Fi Direct snapshot stay with the coordinator because the radios are shared with the host (A.3 item 7). The listener core's platform-operation replies move with `ListenerSessionController`. Original wording: BLE, Wi-Fi Direct, mDNS, scan job, discovered sessions (part of `MainViewModelTransport.kt`, discovery reconcile). Tests include `MainViewModelDiscoveryReconcileTest`.
- [x] `HostMonitorController` (`app/controller/HostMonitorController.kt`; 4 unit tests; failed-start cleanup mutation killed; full Android gate passed). It owns the monitor runtime (`hostMonitorPlaybackRef` removed), start, submit, re-anchor, volume and stop, with the Oboe and runtime seams injectable for JVM tests. `MainViewModelHostMonitorPlayback.kt` is now one-line wrappers. Original wording: the host's local monitor (`MainViewModelHostMonitorPlayback.kt`).
- [x] `ListenerPlaybackController` (`app/controller/ListenerPlaybackController.kt` and `ListenerTrackBoundaries.kt`; 7 + 5 unit tests; handover-reuse and boundary pause-gate mutations killed; full Android gate passed for each part). Part 1 moved the runtime, its single control thread, the same-session clock-lock handover, the pre-runtime estimator and probe ids. Part 2 moved the status-poll and periodic-resync jobs (`playbackJob`, `resyncJob` removed; host stop and recovery cancel them through the controller) and the track boundaries (T1, still Kotlin per decision 0.3). The coordinator still writes all UI state. `currentStreamId` stays on `MainViewModel` because host code shares it; the host extraction splits it. Original wording: playback runtime, sync estimator and handover, listener boundaries, volume, manual resync (`MainViewModelListenerPlayback.kt`, `MainViewModelRustListenerPlayback.kt`, the listener part of `MainViewModelSynchronization.kt`). Tests include `MainViewModelListenerResyncTest`.
- [x] `ListenerSessionController` (`app/controller/ListenerSessionController.kt`; 5 unit tests; ready-before-join (Block 52.2) mutation killed, also by `MainViewModelRustListenerEffectTest`; full Android gate passed). It owns the listener core and its platform effects, network establishment (endpoint or Wi-Fi Direct group) and release, the join request, the transport event loop, the device id and the invitation TLS keys; `MainViewModel` lost 8 fields. The coordinator implements `ListenerSessionCallbacks` (snapshot mapping, transport-event routing, selected session, invite code). The manual-endpoint form, scan/join/leave/retry intents and snapshot mapping only write UI state and call the controllers, so they leave with the coordinator clean-up; the manual connection itself was already `ManualListenerTransportController`. Original wording: listener core, discovery and manual and QR joins, network establishment, leave and retry (`MainViewModelRustListener.kt`, `MainViewModelRustListenerNetwork.kt`, `MainViewModelManualEndpoint.kt`, `MainViewModelListenerActions.kt`). Tests include `MainViewModelRustListenerTest` and `MainViewModelRustListenerEffectTest`.
- [ ] `HostPlaybackController`. **Part 1 done** (`app/controller/HostPlaybackController.kt`, `HostSourceStaging.kt`; 5 unit tests; failed-staging cleanup mutation killed; full Android gate passed): source staging (staged files, live-queue URIs, bounded copy, Rust validation, discard, orphan cleanup) and the streamer, stream start, format and latest packets. The dead `latestDecodedAudio` field was removed (only ever set to null). **Part 2 done** (4 unit tests; pending-command guard mutation killed; full Android gate passed): the stream and playback-command jobs (`hostStreamJob`, `hostPlaybackCommandJob` removed), the boundary timers and pending boundaries. **Part 3 done** (`app/controller/HostStreamLoop.kt`; unheard-boundary cancellation mutation killed by `HostStreamFailureStopTest`; full Android gate passed): the stream loop moved unchanged into `HostStreamLoop`, which reaches the UI, host core and failure paths only through `HostStreamEnvironment`. **Part 4 done** (full Android gate passed): `currentStreamId` is split into `hostPlayback.streamId` and `listenerPlaybackController.streamId`; host session teardown clears both (spec A.3 item 1), and `MainViewModelRustHostNotificationRecoveryTest` now sets the listener id first, so dropping that reset fails it. **Left:** move stream preparation (open playlist, first packet) into the controller; play/pause/resume/stop intents are coordinator orchestration (host-core authority, UI, transport, monitor, listener controllers) and are placed in the coordinator clean-up. Original wording: staging, decoding, streaming, live-queue edits, host boundaries, host timing (`MainViewModelHostStaging.kt`, `MainViewModelHostPlayback.kt`, the playback part of `MainViewModelHostActions.kt`). Tests include `MainViewModelHostQueueTest`.
- [ ] `HostSessionController`: host core lifecycle, advertising and bind, approvals, recovery, resync, events (`MainViewModelRustHost*.kt`, the session part of `MainViewModelHostActions.kt`). Tests include `MainViewModelRustHostEffectTest`, `MainViewModelHostRebindRaceTest`, `MainViewModelRustHostNotificationRecoveryTest` and `MainViewModelStaleApprovalTest`.
- [ ] Coordinator clean-up. `MainViewModel` keeps only role, permissions, demo sessions (debug only), controller creation and `AppUiState` mapping. Every `MainViewModel*.kt` extension file is gone. No property of `MainViewModel` is a mutable domain, transport or playback field.

## 5. Kotlin timing state (only if the owner chose to move it now, decision 0.3)

- [ ] For each item classified in §1 as domain or timing logic: implement it in Rust with Rust tests, expose it through UniFFI, make the controller call it, and delete the Kotlin version. Run a device check on the A54 for anything on the playback or sync path.
- [ ] Otherwise, log each one with its classification in the newest handoff for a later block.

## 6. `SilentDiscoApp` (order per decision 0.2)

- [ ] Characterization: confirm the existing UI tests cover each of the 16 destinations' main callbacks, and add any that are missing.
- [ ] Feature navigation-graph builders `hostNavGraph`, `listenerNavGraph` and `settingsNavGraph`. `SilentDiscoApp` keeps only the `NavHost`, top-level state and the snackbar.
- [ ] Launcher helpers per feature (audio picker, QR scanner, camera permission, runtime permissions).
- [ ] Flow state holders `QrJoinFlowState` and `SessionConfirmationState`, keeping `rememberSaveable` behaviour across rotation and process death, with a `StateRestorationTester` test for each.
- [ ] Move each `LaunchedEffect` / `DisposableEffect` to its feature, keeping its keys unchanged.

## 7. Rust (per decision 0.4)

- [ ] Split `runtime/types.rs` into `runtime/types/{revision,host_draft,tuning,network,listeners,diagnostics}.rs`, keeping `pub use` re-exports. `cargo test --workspace --all-features`, UniFFI generation and desktop `bindings:check` must show no API change.
- [ ] Record the near-limit watchlist in the handoff: `packetizer_worker.rs`, `ffi/src/audio_abi.rs`, `ffi/src/host_transport/handle.rs`, `audio/scheduler/engine.rs`, `ffi/src/host_control/handle.rs`, `ffi/src/audio_output.rs` and `runtime/actor_runtime/mod.rs`, with the rule that the next change to any of them splits it by responsibility.

## 8. Close-out

- [ ] Full instrumented suite on the A54 with no new failures against the §2 baseline.
- [ ] A54 device run: `manual_real_android_listener_late_joins_and_reconnects_over_secure_transport`, which needs one QR scan from the owner.
- [ ] Final survey: no type holds mutable state for more than one responsibility, and no file mixes responsibilities. Update the CLAUDE.md "Authoritative migration documents" list if the owner wants this spec referenced there.
- [ ] Handoff section and `memory.md` entry with the gates actually run.
