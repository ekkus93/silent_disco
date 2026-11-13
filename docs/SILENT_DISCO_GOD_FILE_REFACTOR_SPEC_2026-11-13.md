# Silent Disco — God-file refactor specification

Date: 2026-11-13. Status: proposed; owner approval needed before implementation.
Companion TODO: [SILENT_DISCO_GOD_FILE_REFACTOR_TODO_2026-11-13.md](SILENT_DISCO_GOD_FILE_REFACTOR_TODO_2026-11-13.md).

## 1. Purpose

Split the codebase's God files by **responsibility**, so that each unit owns one concern and its own state. This is not about line counts. The 800-line limit stays a guideline (owner decision 2026-11-05) and remains enforced by `scripts/check-source-file-line-counts.sh`. A file is a God file when it mixes unrelated responsibilities or shares mutable state across them, whatever its length.

Success means:
- every responsibility named in §4 has exactly one owner;
- no Kotlin type holds mutable state for more than one of those responsibilities;
- domain legality stays in Rust, as CLAUDE.md requires;
- behaviour is unchanged. Every existing JVM, instrumented, Rust and desktop test passes, and the A54 device run (handoff §53.4) passes again at the end.

## 2. Findings (measured 2026-11-13 at `487b35aa`)

### 2.1 `MainViewModel` — the God object (Android)

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt` is 791 lines, with 22 more `MainViewModel*.kt` extension files: 23 files, about 5,600 lines in total.
- The class declares 103 properties, most of them `internal var` or mutable collections. Every extension file reads and writes them, so splitting the code into files never split the state.
- It exposes 55 functions in `MainViewModel.kt` alone, covering role selection, permissions, host setup, live-queue editing, host session lifecycle, approvals, playback control, discovery, joining, the manual endpoint, QR invitation keys, volume, tuning, leaving, manual resync and persistence retry.

The properties observed so far fall into these groups:

| Group | Examples of properties now on `MainViewModel` |
|---|---|
| Platform services | `domainStore`, `bleService`, `wifiDirectService`, `mdnsService`, `decoder`, `logger`, `diagnosticsStore`, `metrics` |
| Core controllers | `hostCoreFactory`, `listenerCoreFactory`, `listenerTransportController`, `hostTransportController`, `manualListenerController` |
| Host streaming | `hostPlaylistStreamer`, `hostPlaylistStreamStart`, `hostPlaylistFormat`, `hostStreamJob`, `hostPlaybackCommandJob`, `hostBoundaryJobs`, `hostPendingTrackBoundaries`, `stagedAudioFilesBySourceId`, `liveQueueSourceUris`, `latestDecodedAudio`, `latestPackets`, `hostTimingService` |
| Host monitor | `hostMonitorPlaybackRef`, `hostMonitorPlayback`, `closeHostMonitorOutput` |
| Listener playback and sync | `listenerPlaybackRef`, `listenerPlayback`, `listenerPlaybackSessionId`, `listenerSyncEstimator`, `listenerSyncHandover`, `listenerPlaybackControlExecutor`, `listenerStreamHostStartTimeMs`, `listenerStreamStartTrackName`, `pendingListenerTrackBoundaries`, `appliedListenerTrackBoundaries`, `listenerTrackBoundaryJobs`, `currentStreamId` |
| Joining and invitations | `invitedHostTlsKeys`, pending operation IDs, `persistedAudioReadGrants` |
| Diagnostics and capture | `listenerCallbackCapture`, `callbackCaptureStatus` |
| Background jobs | `playbackJob`, `resyncJob`, `scanJob` and others |
| UI state | `_uiState` / `uiState` (`AppUiState`) |

The complete inventory is the first TODO item. It must assign every one of the 103 properties to exactly one owner in §4.

The extension files, by size: `MainViewModelRustHost.kt` (787), `MainViewModelRustListener.kt` (654), `MainViewModelHostPlayback.kt` (454), `MainViewModelHostActions.kt` (400), `MainViewModelSynchronization.kt` (386), `MainViewModelRustListenerNetwork.kt` (335), `MainViewModelRustListenerPlayback.kt` (307), `MainViewModelHostStaging.kt` (249), `MainViewModelTransport.kt` (207), `MainViewModelRustHostRecovery.kt` (203), `MainViewModelManualEndpoint.kt` (145), `MainViewModelListenerPlayback.kt` (119), `MainViewModelHostMonitorPlayback.kt` (115), `MainViewModelRustHostEvents.kt` (105), `MainViewModelDiagnostics.kt` (71), `MainViewModelCallbackCapture.kt` (60), `MainViewModelRustHostSync.kt` (56), `MainViewModelDemo.kt` (56), `MainViewModelListenerActions.kt` (48), `MainViewModelRustHostResync.kt` (40), `MainViewModelPersistence.kt` (37), `MainViewModelSupport.kt` (7).

Architecture concerns, to be confirmed during the inventory:
- Kotlin queues and applies listener track boundaries (`pendingListenerTrackBoundaries`, `appliedListenerTrackBoundaries`, `listenerTrackBoundaryJobs`) and host boundaries (`hostPendingTrackBoundaries`, `hostBoundaryJobs`). CLAUDE.md says Kotlin must not be a competing owner of domain or timing state once Rust owns it. Each of these must be classified as either presentation, or domain/timing that belongs in Rust.
- `MainViewModelSynchronization.kt` runs a pre-runtime NTP-style sync exchange in Kotlin. Rust owns sync, so the same classification applies.
- `latestDecodedAudio` and `latestPackets` keep decoded audio on the view-model. Per CLAUDE.md, the platform decoding is temporary until shared Block 23.

### 2.2 `SilentDiscoApp` — the God composable (Android)

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt` is 769 lines. One composable, `SilentDiscoApp(...)`, runs from line 68 to the end of the file.
- It collects four view-models' state, holds 8 `rememberSaveable` dialog and flow flags (end and leave confirmations, invite dialog, verified-QR dialog, pending QR join, QR scan observed, camera rationale, camera denied), creates 5 activity-result launchers (including runtime permissions, the audio picker, the QR scanner and camera permission; the inventory names all five), runs 18 `LaunchedEffect`/`DisposableEffect` blocks, and declares all 16 navigation destinations with their callback wiring. The destinations are Startup, Home, HostMusicSetup, HostAccessSetup, HostReady, HostDashboard, HostQueueManagement, NearbySessions, ManualConnect, SessionJoin, ListenerPlayback, ConnectionHelp, AdvancedDiagnostics, Settings, TrustedDevices and TrustedHosts.

### 2.3 Rust — borderline

- `rust/silent-disco-core/src/runtime/types.rs` (776 lines) is a grab bag of about 17 unrelated record types: `SnapshotRevision`, `AudioSourceDescriptor`, the host draft and its patches, `TuningPatch`, `NetworkEndpoint`, `CapabilitySnapshot`, `SessionAdvertisement`, `JoinRequestSummary`, `DeliveryReport`, `SynchronizationSummary`, `ListenerSummary`, `DiagnosticField`, `CoreDiagnostic` and `RuntimeRecordValidationError`. They're data, not mixed logic, so this is a navigability problem, not a God object.
- **Cohesive but at the limit**, so the next change to any of these forces a split: `audio/packetizer_worker.rs` (799), `ffi/src/audio_abi.rs` (796), `ffi/src/host_transport/handle.rs` (786), `audio/scheduler/engine.rs` (784), `ffi/src/host_control/handle.rs` (759), `ffi/src/audio_output.rs` (759), `runtime/actor_runtime/mod.rs` (758). The rule for these is to split by responsibility when they next grow, not now. See §6.

### 2.4 Out of scope

- Large test files close to 800 lines. Each covers one area.
- The desktop crate and React app. No God file was found there in this survey.
- `P2ViewModel.kt` (454) and `WorkflowViewModel.kt` (115). Their own responsibilities stay as they are, but §4 may absorb related state.

## 3. Constraints

1. **Behaviour preserving.** This is a refactor. No user-visible behaviour, protocol, timing or storage change. Any defect found along the way is fixed in its own commit with a test, as the Ralph loop requires.
2. **Rust stays authoritative.** Nothing moves from Rust into Kotlin. When a Kotlin responsibility turns out to be domain or timing logic that Rust already owns, it either moves to Rust (its own TODO item, with Rust tests) or is documented as presentation-only, with the reason.
3. **Real-time paths untouched.** The native callback, the render ring and the C ABI do not change. Moving Kotlin code must not add work, locking or allocation to audio or sync paths. Executors and dispatchers keep their current threading.
4. **Bounded and visible.** Every bounded queue, job and executor keeps its bound and its failure reporting, and diagnostics keep reporting the same counters.
5. **One owner per state.** After the split, a piece of mutable state is written by exactly one class. Others read it through that class's API or a `StateFlow`.
6. **Incremental.** Each step compiles, passes every gate, and is committed and pushed on its own. No long-lived branch.
7. **UI redesign.** The owner plans a full UI redesign (memory `project_ui_redesign_pending`). The `MainViewModel` split works below the screens and does not depend on it. The `SilentDiscoApp` split (§5) must be coordinated with the redesign: either do it as the redesign's first step, or do only the mechanical extraction in §5.1 now. The owner chooses.
8. **Lifecycle.** All controllers are created and closed by the owning `ViewModel`'s scope (`viewModelScope` / `onCleared`), so process death and configuration changes behave exactly as today.

## 4. Target architecture: `MainViewModel`

`MainViewModel` becomes a thin **coordinator**. It creates the controllers, forwards UI intents to them, and combines their state into `AppUiState`. It owns no domain or transport state itself.

| Owner (new) | Responsibility | Takes over |
|---|---|---|
| `HostSessionController` | Host core controller lifecycle, advertising and bind (`MainViewModelRustHost*`), approvals, recovery, resync, host events | host core and transport state, pending operation IDs, recovery dispatch |
| `HostPlaybackController` | Playlist staging, decoding and streaming, live-queue edits, host track boundaries, host timing | host streaming group, staging maps, boundary jobs, decoded audio |
| `HostMonitorController` | The host's local monitor playback | host monitor group |
| `DiscoveryController` | BLE, Wi-Fi Direct and mDNS discovery and scanning | discovery services, `scanJob`, discovered sessions |
| `ListenerSessionController` | Listener core controller, joining (discovery, manual endpoint, QR invitation), network establishment, leave and retry | listener core, manual endpoint, `invitedHostTlsKeys`, pending establish operations |
| `ListenerPlaybackController` | Listener playback runtime, sync estimator and handover, listener track boundaries, volume, manual resync | listener playback and sync group |
| `DiagnosticsController` | Diagnostics store, metrics, support report, developer callback capture | diagnostics and capture group |
| `PersistenceController` | Domain store access and persistence retry, persisted audio read grants | persistence group |
| `MainViewModel` (coordinator) | Role selection, permissions, `AppUiState` combination, demo sessions (debug only) | `_uiState`, role and permission state |

Rules:
- Controllers are plain Kotlin classes, not `ViewModel`s. Each gets its dependencies through its constructor, the same test seams the view-model uses today, so existing JVM tests can build them directly.
- Each controller exposes `StateFlow` snapshots of its own state plus intent methods. The coordinator maps snapshots into `AppUiState`. Controllers never write `AppUiState`.
- Controllers talk to each other through narrow interfaces, never by reaching into fields. For example, when a listener is approved, `ListenerSessionController` hands its stream start to `ListenerPlaybackController` through a method. Any dependency between controllers is one-way and recorded in the TODO.
- Listener and host never share a controller. A device is in one role at a time; the coordinator switches between them.
- The `MainViewModel*.kt` extension-file pattern is retired. Code moves into the owning controller's file or files, each split by responsibility.
- Where Rust already owns a decision, the controller calls Rust and does not recompute it (§3.2).

## 5. Target architecture: `SilentDiscoApp`

### 5.1 Mechanical extraction (safe without the redesign)

- `SilentDiscoApp` keeps the `NavHost`, the top-level state collection and the snackbar host.
- Each feature gets its own navigation-graph builder (`hostNavGraph`, `listenerNavGraph`, `settingsNavGraph`) holding that feature's `composable(...)` destinations and callback wiring.
- Activity-result launchers move into small `remember…` helpers next to their feature: the audio picker (host), the QR scanner and camera permission (listener / P2), and runtime permissions.
- Dialog and flow flags move into a saveable state holder per flow: `QrJoinFlowState` (pending QR join, scan observed, camera rationale, camera denied, verified-QR dialog) and `SessionConfirmationState` (end and leave confirmations, invite dialog).
- `LaunchedEffect` blocks move into the feature that owns them. Each effect keeps its exact keys, so it re-runs at the same moments.

### 5.2 With the redesign

Feature screens get their own state holders driven by the controllers in §4. That's left to the redesign spec.

## 6. Rust items

- **`runtime/types.rs`:** split into `runtime/types/` modules by area: `revision`, `host_draft` (with its patches), `tuning`, `network` (`NetworkEndpoint`, `SessionAdvertisement`, `CapabilitySnapshot`), `listeners` (`JoinRequestSummary`, `ListenerSummary`, `SynchronizationSummary`, `DeliveryReport`) and `diagnostics` (`DiagnosticField`, `CoreDiagnostic`). The validation error stays with the records it validates. Public paths are kept through `pub use`, so nothing outside `runtime` changes, and neither do the UniFFI or desktop bindings (`bindings:check` must stay clean).
- **Near-limit files:** no change now. When one of them must grow, split it by responsibility in that same change. For example, `packetizer_worker.rs` splits into the worker loop and its statistics/failure reporting, and `ffi/src/host_transport/handle.rs` into constructors and sends. `host_transport/protocol_policy.rs` already shows the pattern.

## 7. Testing strategy

- **Before moving anything:** for each controller, list the existing tests that cover its behaviour (JVM: `MainViewModel*Test`, the 48 files in `app/src/test/.../app/`; instrumented: the host-transport, late-join, rebind, saturation and Keystore classes; UI: the screen tests). Fill any gap with a characterization test written against the current `MainViewModel` and passing on it. Gaps include anything that changes timing, a boundary or a recovery path.
- **While moving:** tests move with their behaviour and target the controller directly. Coordinator tests check only how controller state maps into `AppUiState`.
- **Gates after every step:**
  - `./gradlew assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest test lintDebug`;
  - the instrumented classes touched by that step on the A54, always with `-Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true` (memory `feedback-connected-tests-uninstall`);
  - `bash scripts/check-rust.sh` and `npm run check` whenever Rust changes;
  - `scripts/check-source-file-line-counts.sh`.
- **Mutation check:** for each move that carries a timing, ordering or recovery rule, disable the rule in the new controller once and confirm a test fails, with a trap-based restore.
- **End:** the full instrumented suite on the A54. The four known layout failures (handoff §53.3) must not grow. Then the A54 late-join and reconnect device run.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Lost ordering between state that used to be updated together under one object | Keep updates that must be atomic in one controller. Write a characterization test for each cross-group sequence before splitting it. |
| Threading changes (executors, dispatchers, `AtomicReference` handover) | Move each executor and reference with its owner unchanged. No new dispatcher. |
| Job leaks on role switch or `onCleared` | Each controller closes its jobs in `close()`. A test checks that every controller is closed after `onCleared`. |
| Hidden coupling in extension files | The inventory (TODO 1) records every cross-group read and write before any move. |
| Clash with the UI redesign | §5 is split into a mechanical part and a redesign part. The owner chooses the order. |
| Domain logic found in Kotlin | Classify it (§3.2). Moving it to Rust is a separate, separately tested item. |

## 9. Non-goals

- No UI or UX change: no visual, copy or navigation change, unless the redesign covers it.
- No protocol, storage, schema or transport change.
- No change to the desktop app.
- No new dependency injection framework.
- No change to the 800-line policy or its CI gate.

## 10. Decisions needed from the owner

1. Approve this spec.
2. **Order against the UI redesign:** either (a) the `MainViewModel` split now, with `SilentDiscoApp` §5.1 now and §5.2 in the redesign, (b) the `MainViewModel` split now and all of `SilentDiscoApp` in the redesign, or (c) everything after the redesign.
3. **Kotlin timing state:** for track boundaries and the pre-runtime sync exchange, if the inventory confirms they are domain or timing logic, should they move to Rust within this refactor, or be logged for a later block?
4. **`runtime/types.rs`:** split now, or only when it next grows.
