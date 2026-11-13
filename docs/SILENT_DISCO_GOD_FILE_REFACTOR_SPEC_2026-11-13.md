# Silent Disco — God-file refactor specification

Date: 2026-11-13. Status: proposed; owner approval needed before implementation.
Companion TODO: [SILENT_DISCO_GOD_FILE_REFACTOR_TODO_2026-11-13.md](SILENT_DISCO_GOD_FILE_REFACTOR_TODO_2026-11-13.md).

## 1. Purpose

Split the codebase's God files by **responsibility**, so that each unit owns one concern and its own state. This is not about line counts. The 800-line limit is a guideline (owner decisions 2026-11-05 and 2026-11-13); since 2026-11-13 `scripts/check-source-file-line-counts.sh` only reports files over it for review and never fails. A file is a God file when it mixes unrelated responsibilities or shares mutable state across them, whatever its length.

Success means:
- every responsibility named in §4 has exactly one owner;
- no Kotlin type holds mutable state for more than one of those responsibilities;
- domain legality stays in Rust, as CLAUDE.md requires;
- behaviour is unchanged. Every existing JVM, instrumented, Rust and desktop test passes, and the A54 device run (handoff §53.4) passes again at the end.

## 2. Findings (measured 2026-11-13 at `487b35aa`)

### 2.1 `MainViewModel` — the God object (Android)

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt` is 791 lines, with 22 more `MainViewModel*.kt` extension files: 23 files, about 5,600 lines in total.
- The class declares 68 properties (Appendix A.1), most of them `internal var` or mutable collections. Every extension file reads and writes them, so splitting the code into files never split the state.
- It has 55 functions in `MainViewModel.kt` alone (199 across the 23 files), covering role selection, permissions, host setup, live-queue editing, host session lifecycle, approvals, playback control, discovery, joining, the manual endpoint, QR invitation keys, volume, tuning, leaving, manual resync and persistence retry.

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

Appendix A assigns every one of the 68 properties to exactly one owner in §4.

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
- No further change to the 800-line policy. The owner relaxed it on 2026-11-13: the CI check now only reports files over 800 lines.

## 10. Decisions needed from the owner

1. Approve this spec.
2. **Order against the UI redesign:** either (a) the `MainViewModel` split now, with `SilentDiscoApp` §5.1 now and §5.2 in the redesign, (b) the `MainViewModel` split now and all of `SilentDiscoApp` in the redesign, or (c) everything after the redesign.
3. **Kotlin timing state:** for track boundaries and the pre-runtime sync exchange, if the inventory confirms they are domain or timing logic, should they move to Rust within this refactor, or be logged for a later block?
4. **`runtime/types.rs`:** split now, or only when it next grows.


## Appendix A — Inventory (TODO §1, measured at `6a787b93`)

Owners are the controllers in §4, with the `Controller` suffix dropped. **Service** means a platform service the coordinator creates once and injects into the controllers listed, owned by none of them. **T1–T3** are the timing and domain candidates classified in A.4.

### A.1 Properties

`MainViewModel` declares **68** properties (lines 71–225: constructor parameters and class members), not the 103 first estimated in §2.1. That estimate counted local variables too. "Users" lists the extension files (the `MainViewModel` prefix is dropped) that reference each property; `MainViewModel.kt` itself references most of them. Extension files cannot declare state, so all 68 live on the class.

| Line | Property | Kind | Type | Users | Owner | Note |
|---|---|---|---|---|---|---|
| 73 | `domainStore` | internal val | `RustDomainStore` | Persistence,RustHost | Persistence |  |
| 74 | `hostCoreFactory` | internal val | `(String) -> HostCoreController` | RustHost | HostSession |  |
| 77 | `listenerCoreFactory` | internal val | `(String) -> ListenerCoreController` | RustListener | ListenerSession |  |
| 90 | `bleService` | internal val | `BleTransport` | RustHost,RustHostRecovery,RustListenerNetwork,Transport | Service → Discovery, HostSession | BLE both scans (listener) and advertises (host) |
| 91 | `wifiDirectService` | internal val | `SessionTransport` | Demo,Diagnostics,RustHost,RustHostRecovery,RustListenerNetwork,RustListenerPlayback,Synchronization,Transport | Service → Discovery, HostSession, ListenerSession | used by 8 files across both roles |
| 92 | `mdnsService` | internal val | `MdnsTransport` | RustListenerNetwork,Transport | Discovery |  |
| 93 | `listenerTransportController` | internal val | `ListenerTransport` | Diagnostics,RustListener,RustListenerNetwork,RustListenerPlayback,Synchronization | ListenerSession | ListenerPlayback and sync use it through ListenerSession |
| 94 | `hostTransportController` | internal val | `HostTransportController` | Diagnostics,HostActions,HostPlayback,RustHost,RustHostEvents,RustHostRecovery,RustHostResync,RustHostSync | HostSession | HostPlayback broadcasts through a HostSession interface |
| 100 | `hostElapsedRealtimeMs` | internal val | `() -> Long` | HostPlayback | HostPlayback |  |
| 101 | `hostStreamDispatcher` | internal val | `CoroutineDispatcher` | HostPlayback | HostPlayback |  |
| 102 | `closeHostMonitorOutput` | internal val | `() -> Unit` | HostMonitorPlayback | HostMonitor |  |
| 104 | `logger` | internal val | `AppLogger(…)` | HostActions,HostPlayback,ListenerActions,ListenerPlayback,ManualEndpoint,Persistence,RustHost,RustHostRecovery,RustListener,RustListenerNetwork,RustListenerPlayback,Synchronization,Transport | Service (all) |  |
| 105 | `diagnosticsStore` | internal val | `DiagnosticsStore(…)` | Demo,Diagnostics,HostActions,HostPlayback,ListenerActions,ListenerPlayback,RustHost,RustHostRecovery,RustListener,RustListenerPlayback,Synchronization,Transport | Diagnostics | written by 12 files; each controller reports through a Diagnostics API |
| 106 | `metrics` | internal val | `DiagnosticsMetrics(…)` | Diagnostics,HostActions,HostPlayback,RustListener,Synchronization | Diagnostics |  |
| 107 | `decoder` | internal val | `AudioFileDecoder(…)` | HostActions,RustListener | HostPlayback | only a comment mentions it in the listener file |
| 108 | `hostTimingService` | internal val | `HostTimingService(…)` | Synchronization | HostSession | answers listener sync requests; timing candidate T2 |
| 109 | `manualListenerController` | internal val | `ManualListenerTransportController(…)` | CallbackCapture,ManualEndpoint | ListenerSession | developer capture reads it |
| 113 | `liveQueueSourceUris` | internal val | `ConcurrentHashMap<String, Uri>(…)` | HostStaging | HostPlayback |  |
| 114 | `persistedAudioReadGrants` | private val | `PersistedAudioReadGrants(…)` | — | Persistence |  |
| 123 | `invitedHostTlsKeys` | internal val | `mutableMapOf<String, ByteArray>()` | ManualEndpoint,RustListenerNetwork | ListenerSession |  |
| 125 | `_uiState` | internal val | `MutableStateFlow(…)` | Demo,Diagnostics,HostActions,HostPlayback,HostStaging,ListenerActions,ListenerPlayback,ManualEndpoint,Persistence,RustHost,RustHostRecovery,RustListener,RustListenerNetwork,RustListenerPlayback,Support,Synchronization,Transport | Coordinator | written by 19 files today; controllers will publish state instead |
| 134 | `uiState` | public val | `StateFlow<AppUiState>` | HostMonitorPlayback (+ SilentDiscoApp) | Coordinator |  |
| 136 | `currentSessionId` | @Volatile internal var | `SessionId?` | HostActions,RustHost,RustHostRecovery | HostSession |  |
| 137 | `currentStreamId` | internal var | `StreamId?` | HostActions,HostPlayback,RustHost,RustHostRecovery,RustListener | ListenerPlayback | **shared**: listener writes it; host code also reads and resets it, split into a host-owned and a listener-owned field |
| 138 | `listenerStreamHostStartTimeMs` | internal var | `Long?` | RustListener | ListenerPlayback | T1 |
| 139 | `pendingListenerTrackBoundaries` | internal val | `linkedMapOf<Long, PendingListenerTrackBo` | RustListener | ListenerPlayback | T1 |
| 146 | `appliedListenerTrackBoundaries` | internal val | `sortedMapOf<Long, PendingListenerTrackBo` | RustListener | ListenerPlayback | T1 |
| 147 | `listenerStreamStartTrackName` | internal var | `String?` | RustListener | ListenerPlayback |  |
| 148 | `listenerTrackBoundaryJobs` | internal val | `mutableMapOf<Long, Job>()` | RustListener | ListenerPlayback | T1 |
| 154 | `listenerSyncEstimator` | internal var | `RustSyncEstimator?` | Synchronization | ListenerPlayback | T2 |
| 155 | `listenerPlaybackControlExecutor` | internal val | `Executors.newSingleThreadExecutor { runn` | RustListenerPlayback | ListenerPlayback | keep its single thread |
| 158 | `listenerCallbackCapture` | internal var | `NativeCallbackWavCapture?` | CallbackCapture | Diagnostics |  |
| 159 | `_callbackCaptureStatus` | internal val | `MutableStateFlow(…)` | CallbackCapture | Diagnostics |  |
| 160 | `callbackCaptureStatus` | public val | `StateFlow<String>` | — (+ SilentDiscoApp) | Diagnostics |  |
| 161 | `listenerPlaybackRef` | private val | `AtomicReference<FfiListenerPlaybackHandle?>(…)` | — | ListenerPlayback | AtomicReference handover; move unchanged |
| 162 | `listenerPlayback` | internal var | `FfiListenerPlaybackHandle?` | CallbackCapture,RustListener,RustListenerPlayback,Synchronization | ListenerPlayback | capture and sync read it |
| 166 | `listenerPlaybackSessionId` | internal var | `SessionId?` | RustListenerPlayback | ListenerPlayback |  |
| 173 | `listenerSyncHandover` | internal var | `SessionSyncHandover?` | RustListenerPlayback | ListenerPlayback | T2 |
| 174 | `hostMonitorPlaybackRef` | private val | `AtomicReference<FfiListenerPlaybackHandle?>(…)` | — | HostMonitor | AtomicReference handover; move unchanged |
| 175 | `hostMonitorPlayback` | internal var | `FfiListenerPlaybackHandle?` | HostMonitorPlayback | HostMonitor |  |
| 180 | `latestDecodedAudio` | internal var | `AudioDecodeResult?` | HostActions | HostPlayback | T3 |
| 181 | `latestPackets` | internal var | `List<AudioPacket>` | HostActions,HostStaging,RustHost,RustHostRecovery | HostPlayback | T3 |
| 182 | `hostPlaylistStreamer` | internal var | `HostPlaylistStreamController?` | HostActions,HostPlayback,HostStaging | HostPlayback |  |
| 183 | `hostPlaylistStreamStart` | internal var | `com.ekkus.silentdisco.core.uniffi.FfiPlaylistStreamStart?` | HostActions,HostPlayback,HostStaging | HostPlayback |  |
| 184 | `hostPlaylistFormat` | internal var | `com.ekkus.silentdisco.core.audio.AudioFormatSpec?` | HostActions,HostStaging | HostPlayback |  |
| 185 | `stagedAudioFilesBySourceId` | internal val | `ConcurrentHashMap<String, java.io.File>(…)` | HostStaging,RustHost | HostPlayback | HostSession reads it at teardown |
| 187 | `hostBoundaryJobs` | internal val | `ConcurrentHashMap<Long, Job>(…)` | HostPlayback | HostPlayback | T1 |
| 188 | `hostPendingTrackBoundaries` | internal val | `ConcurrentHashMap<Long, PendingHostTrackBoundary>(…)` | HostActions,HostPlayback,HostStaging | HostPlayback | T1 |
| 189 | `hostStreamJob` | internal var | `Job?` | HostActions,HostPlayback,RustHost,RustHostRecovery | HostPlayback | host session teardown cancels it |
| 190 | `hostPlaybackCommandJob` | internal var | `Job?` | HostActions | HostPlayback |  |
| 191 | `playbackJob` | internal var | `Job?` | HostActions,ListenerPlayback,RustHost,RustHostRecovery,RustListenerPlayback | ListenerPlayback | **shared**: listener job, but host start cancels it; becomes a role-switch teardown |
| 192 | `resyncJob` | internal var | `Job?` | HostActions,RustHost,RustHostRecovery,RustListener,Synchronization,Transport | ListenerPlayback | **shared**: listener periodic resync, but host start and teardown cancel it; becomes a role-switch teardown |
| 193 | `scanJob` | internal var | `Job?` | ListenerActions,Support | Discovery |  |
| 194 | `pendingSyncCorrelationId` | internal var | `Long?` | Synchronization,Transport | ListenerPlayback | T2 |
| 196 | `nextSyncCorrelationId` | internal var | `Long` | Synchronization | ListenerPlayback | T2 |
| 197 | `hostCoreController` | @Volatile internal var | `HostCoreController?` | Diagnostics,RustHost | HostSession |  |
| 198 | `listenerCoreController` | internal var | `ListenerCoreController?` | Diagnostics,ListenerPlayback,RustListener,RustListenerNetwork,Synchronization,Transport | ListenerSession | sync and playback read it |
| 199 | `pendingEstablishNetworkOperationId` | internal var | `String?` | RustListenerNetwork | ListenerSession |  |
| 200 | `listenerTransportConnected` | internal var | `Boolean` | RustListener,RustListenerNetwork,Synchronization | ListenerSession |  |
| 208 | `listenerTransportRidesWifiDirect` | internal var | `Boolean` | RustListenerNetwork,Synchronization | ListenerSession |  |
| 209 | `listenerHasPendingWifiDirectJoin` | internal val | `Boolean` | — (+ SilentDiscoApp) | ListenerSession | read by SilentDiscoApp |
| 211 | `pendingStartAdvertisingOperationId` | @Volatile internal var | `String?` | RustHost,RustHostRecovery | HostSession |  |
| 213 | `bindingStartAdvertisingOperationId` | internal var | `String?` | RustHost | HostSession |  |
| 214 | `hostTransportEventLoopStarted` | internal var | `Boolean` | RustHost | HostSession |  |
| 215 | `listenerTransportEventLoopStarted` | internal var | `Boolean` | RustListener | ListenerSession |  |
| 216 | `localListenerDeviceId` | internal val | `String` | ManualEndpoint,RustListener,RustListenerNetwork | ListenerSession |  |
| 224 | `pendingListenerDatagramPorts` | internal val | `PendingHostJoinRoutes<FfiHostTransportHandle>(…)` | RustHost,RustHostEvents,RustHostRecovery | HostSession |  |
| 225 | `hostRecoveryDispatch` | internal val | `HostRecoveryDispatch(…)` | RustHostResync,RustHostSync | HostSession |  |

### A.2 Functions

There are 199 top-level functions across the 23 files, 55 of them in `MainViewModel.kt`. By file:

| File | Functions | Owner |
|---|---|---|
| `MainViewModel.kt` | 55: the UI intents plus init helpers | Each intent forwards to its controller. Role, permission and host-form intents stay with the Coordinator; audio-read-grant helpers go to Persistence; live-queue and setup-audio intents to HostPlayback; join and discovery intents to ListenerSession and Discovery; volume, tuning and manual resync to ListenerPlayback; `swapHostMonitorPlayback` to HostMonitor |
| `RustHost.kt` (21), `RustHostEvents.kt` (2), `RustHostRecovery.kt` (5), `RustHostResync.kt` (1), `RustHostSync.kt` (1) | host core, effects, events, recovery, resync, sync replies | HostSession |
| `HostActions.kt` (10) | start, resume, pause and stop playback, authoritative state | HostPlayback. `afterAuthoritativeHostPlayback*` reports to HostSession through an interface |
| `HostPlayback.kt` (8), `HostStaging.kt` (9) | streaming loop, boundaries, staging | HostPlayback |
| `HostMonitorPlayback.kt` (5) | the host's local monitor | HostMonitor |
| `RustListener.kt` (about 20) | listener core, snapshot and notifications, transport events, boundary scheduling | ListenerSession: core, notifications, effects, the event loop. ListenerPlayback: stream start/stop, boundaries (`*ListenerTrackBoundar*`, `listenerBoundary*`) |
| `RustListenerNetwork.kt` (7), `ManualEndpoint.kt` (7), `ListenerActions.kt` (2) | discovery start and stop, network establishment, manual and QR joins | ListenerSession. `startRustListenerDiscovery`, `stopRustListenerDiscovery` and `scanForSessionsImpl` go to Discovery |
| `RustListenerPlayback.kt` (8), `ListenerPlayback.kt` (4), `Synchronization.kt` (9) | playback runtime, sync probes, resync | ListenerPlayback |
| `Transport.kt` (13) | discovery observation plus host and listener transport failures | Split: `observe*`, `handleBle*`, `refreshDiscoveredSessions` and `handleTransportSnapshot` → Discovery; `reportHostBroadcastDelivery`, `hostControlDeliveryMessage` and `handleHostControlFailure` → HostSession; `stopListenerPlaybackForFailure`, `handleListenerConnectionFailure` and `handleListenerDisconnect` → ListenerSession |
| `Diagnostics.kt` (6), `CallbackCapture.kt` (2) | diagnostics refresh, support report, capture | Diagnostics, fed by a small provider interface from each session controller |
| `Persistence.kt` (2) | domain persistence init and retry | Persistence |
| `Demo.kt` (1), `Support.kt` (1) | debug demo, scan-state reset | Coordinator (demo, debug only) and Discovery (`clearScanState`) |

### A.3 Cross-group reads and writes

Each of these becomes a narrow interface method or a one-way dependency:

1. **Host → listener teardown.** Host stop (`stopHostPlaybackImpl`) and host teardown (`RustHost`, `RustHostRecovery`) cancel `playbackJob` and `resyncJob` and reset `currentStreamId`, which listener code owns. → The coordinator closes the listener controllers when the role becomes host. No field is shared.
2. **HostPlayback → HostSession transport.** `HostPlayback` and `HostActions` broadcast through `hostTransportController`. → HostSession exposes a `HostBroadcaster` interface. HostPlayback depends on HostSession, one way.
3. **HostSession → HostPlayback teardown.** `RustHost` and `RustHostRecovery` read `latestPackets`, `stagedAudioFilesBySourceId` and `hostStreamJob` during session end and rollback. → HostPlayback gets `stopAndRelease()`, which HostSession calls.
4. **ListenerSession ↔ ListenerPlayback.** `Synchronization` and `RustListenerPlayback` read `listenerCoreController`, `listenerTransportController`, `listenerTransportConnected` and `listenerTransportRidesWifiDirect`, and `RustListener` drives playback start and stop. → ListenerSession owns the transport and hands stream and sync events to ListenerPlayback through a `ListenerStreamSink` interface. ListenerPlayback sends sync probes through a `ListenerSyncChannel` that ListenerSession implements.
5. **Discovery → ListenerSession.** `RustListenerNetwork` reads `mdnsService.discoveredSessions` to pick the endpoint version. → Discovery exposes the discovered sessions as a `StateFlow`.
6. **Everyone → Diagnostics.** 12 files write `diagnosticsStore`, and 19 write `_uiState`. → Diagnostics exposes a reporting API. Each controller publishes its own state, and the coordinator maps it into `AppUiState`.
7. **Shared services.** `wifiDirectService`, `bleService` and `logger` are used by both roles. → They become services the coordinator creates, injected into the controllers that need them, with no controller owning them.
8. **Persistence.** `RustHost` reads `domainStore`. → It's injected into HostSession as a read-only dependency.

### A.4 Timing and domain candidates (owner decision: log for later)

Each moves into its controller unchanged and is logged for a later Rust block:

- **T1, track boundaries.** Listener: `pendingListenerTrackBoundaries`, `appliedListenerTrackBoundaries`, `listenerTrackBoundaryJobs` and `listenerStreamHostStartTimeMs`, plus the `listenerBoundary*` and `*TrackBoundar*` functions. Host: `hostPendingTrackBoundaries` and `hostBoundaryJobs`. Kotlin schedules when a boundary takes effect, which is playback timing. It is not yet confirmed whether this only drives what's shown (titles, track names) or competes with boundary timing in the Rust scheduler. That check comes before any move to Rust.
- **T2, the pre-runtime sync exchange.** `MainViewModelSynchronization.kt` probes and keeps `listenerSyncEstimator`, `listenerSyncHandover`, the correlation IDs and the `hostTimingService` replies. Rust owns sync (`RustSyncEstimator` wraps it), so Kotlin only orchestrates the probes. The candidate is to move the probe scheduling and handover into Rust.
- **T3, decoded audio on the host.** `latestDecodedAudio` and `latestPackets` keep the temporary Android platform decoding (CLAUDE.md: until shared Block 23). Removing them belongs to Block 23, not this refactor.

### A.5 Tests by owner

15 JVM test classes build a real `MainViewModel` through `TestMainViewModelSupport.kt` (`newTestMainViewModelHarness`):

| Owner | JVM tests (through the harness) | Other coverage |
|---|---|---|
| HostSession | `MainViewModelRustHostEffectTest`, `MainViewModelRustHostNotificationRecoveryTest`, `MainViewModelHostRebindRaceTest`, `MainViewModelStaleApprovalTest`, `HostJoinProjectionFailureTest`, `HostJoinTrustProjectionTest`, `HostResyncProjectionTest` | instrumented: host transport admission, rebind, session rebind, saturation, Keystore v4 |
| HostPlayback | `MainViewModelHostQueueTest`, `HostQueueBoundarySchedulerTest`, `HostStreamFailureStopTest` | `HostStagingTest`, `HostPacketPresentationTailTest`, `HostPlaybackAuthorityTest`, `HostPlaybackControlStateTest`, `HostPlaybackIdentityTest` (unit); instrumented late join |
| HostMonitor | none | **gap** |
| Discovery | `MainViewModelDiscoveryReconcileTest` | `ScanLifecycleTest` |
| ListenerSession | `MainViewModelRustListenerEffectTest`, `ListenerReconnectReplayTest` | `MainViewModelRustListenerTest` (unit helpers), `SessionSelectionGuardTest`; instrumented manual endpoint and QR screens |
| ListenerPlayback | `MainViewModelListenerResyncTest` | `ManualResyncStateTest`, `SyncEstimateMappingTest`; A54 device run |
| Diagnostics | `BridgeQueueDiagnosticsTest` | `SupportReportTest` |
| Persistence | none | `StorageInitializationPolicyTest`; **gap** for retry |
| Coordinator | none | `UiStateValidationTest`, `AppPresentationTest` |

**Gaps for TODO §2:**
- host monitor start, stop and swap;
- persistence retry;
- the cross-group sequences in A.3 items 1 and 3 (host start tears down listener jobs; session end releases staged audio);
- an `onCleared` test.
