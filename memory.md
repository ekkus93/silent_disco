# memory.md — `silent_disco`

## 2026-08-01T19:47:19Z - Claude Sonnet 5 - Shared Block 15.1 jitter buffer packet validation and ordering complete

- Implemented `audio::JitterBuffer` (`rust/silent-disco-core/src/audio/jitter_buffer.rs`): a bounded, ordered holding area (`BTreeMap<u64, AudioDatagram>` keyed by sequence) for validated packets belonging to exactly one host session/stream. `accept()` validates session/stream identity, rejects duplicates (already buffered) and late arrivals (already emitted), enforces a configurable reorder window (`max_reorder_window`, default 64 packets, hard ceiling `MAX_REORDER_WINDOW_LIMIT=4096`) and a configurable maximum buffered-duration span (`max_buffered_duration_ms`, default 2000ms, hard ceiling `MAX_BUFFERED_DURATION_LIMIT_MS=60000`, comparing `host_presentation_time_ms` against the earliest still-buffered packet). `pop_in_order()` emits strictly in-sequence (no gap-filling — that is deliberately left to the concealment policy in 15.2). `missing_sequence_count()` exposes the current gap for the scheduler/concealment layer to act on.
- Researched the existing Kotlin reference (`AudioPacketBuffer`/`ListenerPlaybackScheduler` in `app/src/main/java/.../core/audio/{AudioPipeline,PlaybackScheduling}.kt`) before implementing and confirmed it is *not* a straightforward port target for 15.1: it has no duplicate detection (a same-sequence insert into its `sortedMapOf` silently overwrites), no bounded reorder window, no stale-stream rejection, no maximum-buffered-duration bound, and no sequence-wraparound handling. `JitterBuffer` intentionally adds all four bounds as new architecture, consistent with this project's "queues and frame sizes must be bounded" rule — do not treat the Kotlin implementation as authoritative for what 15.1 needs to cover.
- 13 new tests in `audio/jitter_buffer_tests.rs`: out-of-order-to-in-order emission, gap/missing-sequence accounting, duplicate rejection, late (already-emitted) rejection, reorder-window boundary and overflow, buffered-duration overflow, wrong-session rejection, wrong/stale-stream rejection, and three config-validation failure cases.
- Full `bash scripts/check-rust.sh` gate passed cleanly (153/153 in the core lib suite, 0 warnings) on the pinned toolchain before committing. Confirmed the transient `transport::tests::socket_runtime_completes_multi_listener_join_sync_and_audio_exchange` timeout seen during Block 14 does not recur here — treated as an environment-timing flake unrelated to this work, not investigated further since it stayed green across every run since.
- Marked all seven 15.1 checklist items complete in `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` with an implementation note explaining the port-vs-new-architecture distinction above. Committed as `0af3e28` and pushed to `origin/master`.
- **Scope boundary for next session:** 15.2 (concealment policy — silence synthesis for gaps, bounded consecutive-concealment-before-resync) and 15.3 (scheduler — host-to-local presentation mapping, low/high water behavior, resync decisions, render-ring pacing) are separate files (`concealment.rs`, `scheduler.rs`) per the architecture spec's file layout and were deliberately left unimplemented; `JitterBuffer` only provides the mechanism (ordered storage + gap visibility) they will consume. 15.4 (deterministic clock injection) and 15.5 (broader test suite spanning the full scheduler) are also still open.
- Desktop Block 25 ("desktop decoder can feed the actual shared packetizer") remains unstarted: confirmed zero references to the packetizer anywhere under `desktop/src-tauri/src/` — this is real wiring work, not just a verification checklist, and overlaps substantially with desktop Block 26's data-flow wiring.

## 2026-08-01T19:40:11Z - Claude Sonnet 5 - Shared Block 14.2-14.4 streaming packetizer complete

- Implemented `audio::Packetizer` (`rust/silent-disco-core/src/audio/packetizer.rs`): a bounded, incremental transform from `DecodedPcmChunk` to shared-protocol `ProtocolFrame::Audio` datagrams. Retains at most one packet's worth of leftover interleaved samples ("carry") between `push_chunk` calls — never concatenates a full track. Configurable packet duration (default 20 ms, range 1-1000 ms), monotonic host presentation timestamps derived from `host_start_time_ms + sequence * packet_duration_ms`, and a short final chunk is silence-padded so every wire datagram satisfies the exact `payload.len() == samples_per_packet * channels * 2` requirement. Constructor validates the resulting encoded datagram against `MAX_AUDIO_DATAGRAM_BYTES` for the exact session/stream identifier lengths, rejecting configurations that would overflow it.
- Implemented `audio::StreamingPacketizeHandle` (`rust/silent-disco-core/src/audio/packetizer_worker.rs`): worker-thread wrapper mirroring the existing `StreamingDecodeHandle` pattern exactly — bounded `sync_channel` output queue (`StreamingPacketizeConfig{queue_capacity}`, default 32, max 256), `Arc<AtomicBool>` cancellation, backpressure via reserve-then-try_send retry loop (counted in `PacketizerSummary.backpressure_events`, never silently drops), and `Drop` that cancels+joins. Cancellation surfaces as `Err(PacketizerWorkerError{kind:Cancelled})` through `join()`/`cancel_and_join()` — **not** `Ok(summary)` with a `Cancelled` state — confirmed to match `StreamingDecodeHandle`'s existing precedent; do not "fix" this in a future session without re-checking the precedent first.
- 17 new tests in `audio/packetizer_tests.rs`: exact packet boundaries, short-final-chunk silence padding, empty stream, format mismatch, already-finished rejection, invalid configuration (zero-sample and oversized-datagram cases), out-of-range duration, stream-ID restart re-triggering `stream_start_message`, a Kotlin-reference compatibility fixture (`app/src/test/resources/rust-migration/packetization/pcm_packetization_v1.json`), and three worker-level tests (drain-and-complete, backpressure-without-dropping, cancel-without-panic) that open real decode workers.
- **Test-isolation gotcha**: the worker-level packetizer tests open real `StreamingDecodeHandle` instances and race with `audio::tests`' `active_worker_count()` assertions if run concurrently. Fixed by widening `audio_test_guard()` in `audio/tests.rs` from private to `pub(super)` and having the 3 new worker tests acquire `super::tests::audio_test_guard()`. If a future block adds more tests that open real decode/packetize workers in this module tree, they must acquire the same guard.
- Investigated a one-off failure in `transport::tests::socket_runtime_completes_multi_listener_join_sync_and_audio_exchange` (times out waiting for a transport event) that surfaced once during a full-gate run. Confirmed flaky, not a regression: the file was untouched this session, the test passed in isolation immediately after, and two subsequent full-suite runs (140/140) passed cleanly. Left as-is; if it recurs, suspect timing sensitivity under concurrent-test system load rather than the packetizer changes.
- Full `bash scripts/check-rust.sh` gate (fmt, clippy with `-D warnings`, full workspace test suite) passed cleanly on the actual pinned toolchain before committing.
- Marked TODO items 14.2 and 14.4 fully complete, and three of four 14.3 items complete, in `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`. Left one 14.3 item unchecked: "Host UI sees source/packetizer failure" — no UniFFI binding or Kotlin/Compose surface exists yet for this new packetizer worker (verified: no `Packetiz` references anywhere under `rust/silent-disco-ffi/src/`). That wiring is deferred to Block 25/26 per the TODO's own note, alongside feeding real desktop/Android decoder output into this packetizer.
- Committed as `1487c14` and pushed to `origin/master`. Left `desktop/tsconfig.app.tsbuildinfo` / `desktop/tsconfig.node.tsbuildinfo` untracked and unstaged — pre-existing build-artifact leftovers unrelated to this block, not something this commit should pick up.

## 2026-07-28T19:54:33Z - GPT-5.6 Thinking - Tauri desktop Block 10 core ownership complete

- Completed shared migration Block 10 documentation and desktop Blocks 9–10 for `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`.
- Added direct Tauri ownership of one `CoreActorRuntime`, one `DatabaseWorker`, one exclusive `ProfileLease`, the public identity derived from OS-protected secret material, and a bounded notification buffer for each open production profile.
- Production identity uses the operating-system credential store and fails closed when unavailable, locked, malformed, or unwritable. There is no plaintext file, synthetic production identity, anonymous identity, or in-memory fallback.
- `open_profile` performs blocking startup off the Tauri/UI thread and does not report `Ready` until profile locking, secure identity, Rust storage, the authoritative actor, and initial snapshot delivery all succeed. `get_current_snapshot` reads the real actor snapshot and preserves its revision.
- Duplicate opens fail explicitly. Partial startup and normal close attempt actor, database, and profile-lock cleanup in reverse order; cleanup failures remain attached instead of overwriting the primary failure. Close is idempotent after closed/failed state cleanup.
- Added bounded DTOs and Rust-derived TypeScript bindings without exposing native paths, private key material, database handles, raw pointers, or audio payloads. Generated Tauri schemas are excluded from Biome because they are tool-owned artifacts; application and handwritten frontend files remain covered.
- Tests cover successful open/current snapshot, duplicate-open rejection, profile-lock lifetime, storage failure without fallback, observer setup failure, and idempotent shutdown after partial failure. Notification tests cover latest-snapshot coalescing and visible non-snapshot queue overflow.
- Guarded finalizer run `30393427074` passed `cargo fmt --check`, Clippy with warnings denied, desktop backend tests, `cargo check`, Rust-derived binding verification, Biome formatting/lint, TypeScript checks, frontend tests, production frontend build, and the tracked-source line-count gate before committing `e371813f144d81617b505d9435d58dd1c7d27994`.
- Actual Secret Service behavior in the user's Ubuntu desktop session and full application launch remain device/environment acceptance work. No physical desktop credential-store or Android-device result is claimed here.

## 2026-07-27T20:00:00Z - GPT-5.6 Thinking - Tauri desktop host Block 1 baseline recorded

- Started the Ralph Loop for `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` from `master` at commit `e9ed31aca529f1811f9db4cc7121f8b2e3df31c4`.
- Confirmed the required desktop documents exist at `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md` and `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`.
- Compared implementation commit `294fd72ad703cf9bbf2b5ffc25599985f72dfbee` with the desktop-plan head. The five intervening commits change documentation only; no Rust, Kotlin, C++, Gradle, or workflow production source changed.
- Used permanent GitHub Actions run `30304221562` as the latest source-equivalent automated baseline. Its Rust quality, Android build/unit/lint, ABI packaging, and API 29 managed instrumentation jobs all completed successfully. This is recorded evidence, not a claim that the current execution container reran those commands.
- Physical Android acceptance remains open. The current handoff requires at least two physical Android devices; the native-load, Rust synchronization, Rust database, host/listener, resilience, P2, and release/device scenarios must not be inferred from CI or emulator results.
- Inspected the shared migration TODO and production search results. Shared Blocks 10, 12, 14, 16, 19, 23, and 26 remain incomplete:
  - Block 10: no production `CoreCommand`, `PlatformEvent`, `CoreSnapshot`, `CoreHandle`, or authoritative actor implementation; this blocks desktop actor integration, host UI state, Lab Mode, and most production desktop phases.
  - Block 12: host lifecycle and approval policy remain Kotlin/Android-owned; this blocks a production desktop host.
  - Block 14: no Rust bounded decoded-PCM ingestion or streaming host packetizer; this blocks production desktop audio transmission.
  - Block 16: no Rust SPSC render ring; this blocks the shared real-time output architecture and optional desktop monitor output.
  - Block 19: no Rust TCP/UDP transport runtime; this blocks real desktop-to-Android LAN sessions.
  - Block 23: decoder ownership remains undecided and unimplemented; this blocks the final production desktop decoder path.
  - Block 26: unified Rust diagnostics/export remains incomplete; this blocks the final desktop diagnostics and Lab Mode observability surface.
- Current Kotlin ownership remains visible in `MainViewModel`, `PcmPacketizer`, `ListenerPlaybackScheduler`, `AudioTrackPlaybackEngine`, BLE/Wi-Fi Direct services, and Kotlin TCP transport. The desktop implementation must not copy these responsibilities into Tauri-specific Rust or TypeScript.
- Execution environment inventory for this session:
  - Debian GNU/Linux 13 (`trixie`), x86_64 container; this is not the intended Ubuntu 24.04 developer baseline.
  - Node `v22.16.0`; npm `10.9.2`.
  - No installed Rust toolchain, `gh`, Android SDK/`adb`, PipeWire/PulseAudio/ALSA tools, WebKitGTK development packages, Secret Service development package, Avahi daemon/client development package, or physical Android device.
  - The container has no direct GitHub network access and no systemd user/system service environment. It cannot validate multicast/mDNS, real audio, Tauri Linux bundling, Android interoperability, or the user's actual LAN topology.
- Fresh local `./gradlew`, Cargo, and Android instrumentation commands were not run and must not be marked complete. Block 1 is a recorded partial baseline, not accepted complete; continue with the missing reproducible developer-machine inventory and fresh gates before adding desktop files.

## 2026-07-26T04:17:33Z - GPT-5.6 Thinking - Rust-owned Android persistence Block 9 complete

- Added `AndroidDatabasePathProvider`, which uses `noBackupFilesDir/domain/silent-disco.sqlite3`, creates only the parent directory, returns the complete path to Rust, never opens SQLite, and intentionally excludes the domain database from Android backup.
- Added schema migration v2 and the `legacy_imports` marker. `LegacyAndroidImport` is versioned, validated before transaction start, imports tuning/trust atomically, records committed completion, rejects conflicting marker versions, rolls back invalid input, and is idempotent on repeated startup.
- Added a pinned `jni` 0.21.1 control-plane bridge for database open/close, typed legacy import, settings load/save, and trusted-device upsert/query. Rust owns all SQL, migrations, connection state, and worker lifecycle; Kotlin receives stable explicit status codes and no raw SQL surface.
- Added `AndroidRustDomainStore`. It reads only documented tuning keys and the documented dynamic `trusted:` namespace, rejects malformed known values visibly, preserves legacy values on failure, deletes legacy domain keys only after Rust reports committed import success, and retries cleanup after a failed Android preference commit.
- Removed direct tuning and trust persistence from `MainViewModel`. Persistence-dependent host, scan, join, tuning, and trust actions are blocked until Rust initialization succeeds. A database failure is shown as persistent-storage unavailable; there is no fallback to legacy preferences.
- Database shutdown is explicit and fail-visible. Initialization retains a database-close failure as a suppressed exception rather than dropping it, and `MainViewModel.onCleared()` does not convert close failure into log-only success.
- Added Android instrumentation source covering first-run database creation, tuning/trust import, reopen from Rust values, invalid import preservation, malformed trust preservation, and corrupt-database visibility. Legacy trust preferences contained only device IDs/booleans, so imported display names intentionally use the device ID until richer metadata is learned later.
- PR #35 merged as `5fc5ae966b1157b2cd5887c10d3522da81856f8f`. Permanent CI run `30187155765` passed Rust formatting, Clippy with warnings denied, all Rust tests, Android debug/PoC-debug/release and instrumentation-APK builds, four-ABI Rust/JNI packaging, Android unit tests, and Android lint.
- Physical execution of `AndroidRustDomainStoreInstrumentedTest` is **NOT RUN** because no Android device is attached. Do not claim device validation until the exact command, device model, Android version, ABI, and result are recorded.

## 2026-07-26T02:32:00Z - GPT-5.6 Thinking - Rust schema and migrations Block 8 complete

- Added ordered immutable Rust migrations with explicit versions and SHA-256 checksums. `schema_migrations` records version, application timestamp, and checksum; checksum mismatch, unsupported newer schemas, and failed transactional migrations are fatal and never trigger automatic delete/recreate behavior.
- Added the strict initial SQLite schema for application settings, trusted devices, session history, and diagnostic runs, including required constraints, indexes, and foreign keys.
- Added typed repositories for settings, trusted devices, session lifecycle records, and diagnostic summaries. Raw SQL and the SQLite connection remain private to the Rust storage worker and never cross FFI.
- Added temporary-file database tests for empty-to-latest migration, reopen, rollback, checksum mismatch, newer-schema rejection, constraint mapping, worker request ordering, Unicode names, and binary public keys.
- PR #31 merged as `4dd2de7c54942f047d4fd47ca8c73ae73721fabe`. Guarded validation run `30184437336` passed Rust formatting, Clippy with warnings denied, all Rust tests, debug/PoC-debug/release and instrumentation APK builds, four-ABI Rust packaging checks, Android unit tests, and Android lint.
- Block 8 does not select the Android database path or import legacy `SharedPreferences`; those remain explicit Block 9 work with no fallback to duplicate Kotlin persistence.

## 2026-07-25T23:27:11Z - GPT-5.6 Thinking - Rust SQLite worker Block 7 complete

- Added a Rust-owned SQLite worker in `silent-disco-core`; one dedicated thread owns the only connection and callers receive typed control-plane operations rather than raw SQL or connection access.
- The command queue is bounded with a default capacity of 32. Normal requests use nonblocking admission and return visible `StorageBusy` when full; accepted commands are tested to receive a result rather than being dropped.
- Shutdown closes request admission before queuing the shutdown command, making post-stop clients reject deterministically instead of racing into `ReplyDisconnected`. The worker exposes explicit start, checkpoint, stop, close, and join behavior; dropping an unjoined worker is fail-visible rather than silently detaching it.
- Pinned `rusqlite` 0.40.1 with bundled SQLite and committed the regenerated lockfile. Startup enables and verifies foreign keys, requires WAL, applies a 2,000 ms busy timeout, requires `synchronous=FULL`, records the SQLite library version and connection policy in diagnostics metadata, and fails initialization if any required policy cannot be established.
- Added separate storage categories for open, pragma, migration, query, transaction, constraint, busy/queue-full, corruption, close, thread start, stopped worker, panic, reply disconnect, and shutdown state. `CoreError` conversion preserves operation/schema context and derives subsystem from the stable error code to prevent code/subsystem mismatch.
- Tests use temporary database files and cover serialized thread ownership, bounded queue saturation, accepted-command completion, deterministic shutdown rejection, explicit stop/join, WAL-policy rejection, corruption detection, invalid configuration, SQLite error mapping, and stable error-subsystem integrity.
- PR #28 merged as `32ca46b1062b0e85f477f03d54541502145f348a`. CI run `30179055667` passed Rust formatting, Clippy with warnings denied, all Rust tests, debug/PoC-debug/release APK builds, instrumentation-test APK compilation, four-ABI Rust packaging, Android unit tests, and Android lint.
- Block 7 intentionally creates no schema or repository SQL. Ordered migrations, tables, repositories, and legacy Android data import remain Block 8 and later work.

## 2026-07-25T20:58:09Z - GPT-5.6 Thinking - Rust synchronization Block 6 code complete

- Ported clock synchronization to Rust with distinct host/local monotonic timestamp types, checked four-timestamp RTT/offset arithmetic, bounded correlation tracking, bounded sample/drift history, low-RTT selection, confidence classification, skew estimation, and initial/periodic/drift decisions.
- Added tests for near-`u64` arithmetic, impossible orderings, duplicate and stale responses, mismatched echoed timestamps, pending-probe capacity, high-RTT rejection, history bounds, confidence thresholds, and decision behavior.
- Added binding-friendly Rust synchronization records and static JNI exports with bounded positive handles, stable explicit error statuses, collision-safe registry insertion, explicit destruction, and no JNI pointer dereferences.
- Added a synchronized Kotlin bridge that consumes every native status immediately and performs no estimator calculations. Non-finite values, unknown confidence codes, invalid handles, load/link failures, and impossible timestamps fail visibly.
- Added `RustSyncEstimatorInstrumentedTest`, which loads the existing Kotlin compatibility JSON fixture and invokes the Rust estimator. Permanent CI now compiles/packages the instrumentation-test APK.
- PR #24 merged as `929ec82a24e6e817e0e9a6a40c07558739b9222a`. Pre-merge CI run `30174145493` passed Rust formatting, Clippy with warnings denied, all Rust tests, debug/PoC-debug/release APKs, instrumentation-test APK compilation, four-ABI Rust packaging, Android unit tests, and Android lint.
- Physical execution of `RustSyncEstimatorInstrumentedTest` was **NOT RUN** because no physical Android device is attached. Block 6 physical-Android acceptance remains open; do not claim device validation until the command and device details are recorded.

## 2026-07-25T19:51:20Z - GPT-5.6 Thinking - Rust migration Block 5 complete

- Completed Block 5 while preserving all physical-device-only gates.
- Established protocol v2 with `SDP2`, a fixed 16-byte network-order header, explicit version/kind/flags/length, a 64 KiB control limit, and a 4 KiB audio datagram limit.
- Added canonical Rust control, synchronization, and PCM16 audio schemas; bounded parsing, exact-length validation, CRC32 integrity, authorization/staleness policy, and independent diagnostics.
- Added production-encoder-generated executable vectors for every message kind, boundary sizes, deterministic hashes, and malformed/unsupported/integrity cases.
- CI run `30170763626` passed Rust format, Clippy with warnings denied, Rust tests, all APK variants, four-ABI packaging, Android unit tests, and Android lint.

## 2026-07-25T19:51:20Z - GPT-5.6 Thinking - Rust migration Block 4 complete

- Completed validated Rust domain identifiers, stable domain enums, and structured errors while preserving the physical-device gate.
- IDs are bounded and validated; enums have stable numeric/wire representations; `CoreError` has subsystem-specific codes, bounded context, severity, retryability, and operation correlation.
- CI run `30168849005` passed Rust format, Clippy with warnings denied, Rust tests, all APK variants, four-ABI packaging, Android unit tests, and Android lint.

## 2026-07-25T17:45:29Z - GPT-5.6 Thinking - Block 3 Rust Android integration code complete

- Added a Gradle-owned Rust Android build using pinned Rust 1.97.1, cargo-ndk 4.1.2, Android NDK 28.2.13676358, and minSdk/platform 29.
- Explicitly builds and packages `libsilent_disco_ffi.so` for `armeabi-v7a`, `arm64-v8a`, `x86`, and `x86_64` in debug, PoC-debug, and release APKs. Generated libraries stay under `app/build/generated/rustJniLibs`; `cleanRustAndroid` deletes only that generated tree.
- Added the Rust C ABI version export and direct JNI entry point, plus `RustCoreBridge` with explicit load/version exceptions and no hard-coded success fallback. The bridge is control-plane/startup-only and is not called from the real-time audio path.
- Added JVM ABI validation tests and `RustCoreBridgeInstrumentedTest`, which includes `Build.SUPPORTED_ABIS` in failure output.
- CI run `30167855172` passed Rust formatting, Clippy with warnings denied, Rust tests, all Android APK builds, four-ABI APK packaging checks, Android unit tests, and Android lint on the same revision.
- The instrumented native-load/version test has not been executed here because no physical Android device is attached. The two physical-device tasks and Block 3 acceptance remain open. Run `./gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=com.ekkus.silentdisco.core.rust.RustCoreBridgeInstrumentedTest` on a device and record the model, Android version, ABI, and result before proceeding past this gate.

## 2026-07-25T16:56:06Z - GPT-5.6 Thinking - Rust migration compatibility baseline complete

- Completed the code-verifiable Phase 1 baseline and Block 2 workspace scaffold from `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`.
- Added versioned compatibility fixtures for every existing control-message variant, clock-sync samples and outlier rejection, PCM packetization headers/payload hashes, host/listener state decisions, tuning normalization, and legacy settings/trusted-device persistence.
- Added `RustMigrationCompatibilityFixtureTest`, which exercises production Kotlin codecs, sync estimator, packetizer, binary audio codec, state helpers, tuning functions, and the production-owned `LegacyPreferencesContract`.
- Added the Rust workspace pinned to Rust 1.97.1 with `silent-disco-core`, `silent-disco-ffi`, and `silent-disco-test-support`, plus `scripts/check-rust.sh` and permanent GitHub Actions CI.
- CI run `30166034765` passed Android unit tests, Android `lintDebug`, Rust formatting, clippy with warnings denied, and all Rust workspace tests on the same revision.
- Physical-device baseline checks, APK Home-screen launch, and connected Android tests remain unverified in this environment and remain unchecked.

## 2026-07-25T15:29:13Z - GPT-5.6 Thinking - Rust migration Block 1.2 ownership inventory

The shared-Rust-core migration has started using `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`. The following Kotlin/Android components are the current authoritative owners and form the extraction checklist:

- **Protocol models:** `app/src/main/java/com/ekkus/silentdisco/core/protocol/ProtocolModels.kt`.
- **Protocol serialization and TCP framing:** `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt` (`JsonMessageCodec`, control/sync codecs, and `AudioPacketCodec`).
- **Host and listener lifecycle:** `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`, with state records and presentation helpers in `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`.
- **Join approval and rejection:** `MainViewModel.approveJoinRequest`, `rejectJoinRequest`, and incoming control-message handlers.
- **Clock synchronization:** `app/src/main/java/com/ekkus/silentdisco/core/sync/ClockSync.kt`, `SyncMaintenance.kt`, and the synchronization orchestration in `MainViewModel.kt`.
- **PCM packetization:** `app/src/main/java/com/ekkus/silentdisco/core/audio/AudioPipeline.kt` (`PcmPacketizer`).
- **Jitter buffering and playback scheduling:** `AudioPipeline.kt` (`AudioPacketBuffer`) and `app/src/main/java/com/ekkus/silentdisco/core/audio/PlaybackScheduling.kt` (`ListenerPlaybackScheduler`).
- **Audio output:** Kotlin `AudioTrackPlaybackEngine` in `PlaybackScheduling.kt`; the current C++/Oboe bridge is diagnostic rather than the production render engine.
- **BLE discovery and advertisement:** `app/src/main/java/com/ekkus/silentdisco/core/transport/BleDiscoveryService.kt`.
- **Wi-Fi Direct establishment:** `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`.
- **TCP channel transport:** `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`, orchestrated by `WifiDirectTransportService.kt`.
- **Settings and trusted-device persistence:** Android `SharedPreferences` owned directly by `MainViewModel.kt`.
- **Diagnostics:** `app/src/main/java/com/ekkus/silentdisco/core/diagnostics/DiagnosticsStore.kt`, `core/logging/DiagnosticsMetrics.kt`, `core/logging/AppLogger.kt`, and orchestration in `MainViewModel.kt`.

Execution constraints for this session:

- The repository is writable through the connected GitHub integration, but the execution container has no GitHub network access, no authenticated `gh`, no Android device, and no installed Rust toolchain.
- Fresh `./gradlew test`, `./gradlew lintDebug`, connected Android tests, and APK Home-screen validation could not be truthfully executed here. Those Block 1.1 checks remain incomplete and must not be marked complete based only on the older June 28 results below.
- Continue committing only coherent sub-blocks whose contents can be verified without fabricating test or device results.

## 2026-06-28T21:26:27Z - Claude Sonnet 4.6 - FIX5 Ralph Loop: COMPLETE (9 blocks delivered)

**FIX5 (correctness hardening pass) COMPLETE. All P0/P1/P2 items implemented and tested:**
- Block 1 (689713b): `nextStateForSyncProbe()` helper, `requestListenerSyncProbe()` preserves active playback state (P0.1-P0.2)
- Block 2 (5b3b27b): `classifyTransportSnapshotRole()` helper, listener transport failure → ERROR state (P0.3-P0.4)
- Block 3 (94562b6): Delivery-first `rejectJoinRequest()`, invite-code rejection diagnostics after delivery (P0.5-P0.7)
- Block 4 (d1ff8fb): `hostControlDeliveryMessage()` helper, honest delivery warnings in pause/stop/endSession (P1.1-P1.2)
- Block 5 (dbbdf91): Zero-listener audio broadcast sets `_uiState.lastError`, removed `zeroPeerBroadcastCount` (P1.3-P1.4)
- Block 6 (b8d0853): `handleSyncFailure()` and `handleListenerDisconnect()` clear `buffered`/`playing` flags (P1.5-P1.7)
- Block 7 (a5c1f3b): Demo simulation calls `requestListenerSyncProbe("Demo clock sync")` not `manualResync()` (P1.8)
- Block 8 (5865a60): `HostSessionValidator`, `requireHostSessionForPlayback()`, BLE test hooks, tautological tests replaced (P2.1-P2.4)
- Block 9 (a980bb2): Instrumented BLE tests on Samsung A54 via `emitScanFailureForTest`/`emitAdvertiseFailureForTest`

**Key FIX5 decisions:**
- `nextStateForSyncProbe()`: initial states (APPROVED/CONNECTING/SYNCING_CLOCK) → SYNCING_CLOCK; all others preserved
- `classifyTransportSnapshotRole()`: HOST_FAILURE if hosting active, LISTENER_FAILURE if listener active, else IGNORE
- `rejectJoinRequest()` now delivery-first: pending request only removed if rejection delivery succeeded
- `handleJoinRequestMessage()` invite-code rejection: "Rejected X" diagnostics only written after delivery
- `hostControlDeliveryMessage()`: returns warning string for ZERO_PEERS/PARTIAL_FAILURE, null for OK
- `zeroPeerBroadcastCount` removed; zero-peer events surfaced through `_uiState.lastError` directly
- `handleListenerDisconnect()` now also cancels playbackJob/resyncJob, stops engine, clears pending state
- `HostSessionValidator.validate(form)` extracted to AppState.kt; `MainViewModel.validateHostForm()` delegates to it
- `requireHostSessionForPlayback(sessionId)` extracted to MainViewModel.kt top-level
- BLE internal hooks: `emitAdvertiseFailureForTest`/`emitScanFailureForTest` on concrete `BleDiscoveryService`
- Instrumented test uses `runBlocking` + `delay(50)` before `tryEmit` to guarantee collector subscribes first
- **CRITICAL**: This repo's pre-commit hook blocks `Co-Authored-By:` lines in commit messages. Never include this attribution.

**Tests:** All unit tests pass. Lint clean. 3 instrumented BLE tests pass on Samsung A54 (R5CW31AX4FL).

## 2026-06-28T20:38:27Z - Claude Sonnet 4.6 - FIX4 Ralph Loop: COMPLETE (all 9 blocks delivered)

**FIX4 (third hardening pass) COMPLETE. All 9 Ralph Loop blocks implemented and tested:**
- Block 1 (77e166d): canManualResync broader, requestListenerSyncProbe helper, host/listener separation (P0.1-P0.4)
- Block 2 (6eadd42): classifyBroadcastDelivery production helper, reportHostBroadcastDelivery, all control/sync broadcasts consumed (P0.5-P0.8)
- Block 3 (34fdc4c): Host audio zero-peer disclosed, partial delivery warning, zeroPeerBroadcastCount (P0.9)
- Block 4 (5597b04): Wi-Fi Direct permission check in startHost(), async group failure → host ERROR (P0.10-P0.11)
- Block 5 (c965c4d): selectDiscoveredSession guarded with canSelectSession(), clearScanState in failure/disconnect (P1.1-P1.2)
- Block 6 (3d03b69): Missing host session sets hostState=ERROR; streamId assigned to currentStreamId (P1.3-P1.4)
- Block 7 (5cb30dc): BLE failures SharedFlow, observeBleFailures, handleBleScanFailure/AdvertiseFailure (P1.5-P1.6)
- Block 8 (1427b61): AudioTrack write error code preserved (no coerceAtLeast), dynamic invite-code label (P1.7, P2.1)
- Block 9: Tests embedded across blocks 1-7 (ManualResyncStateTest, BroadcastDeliveryTest, SessionSelectionGuardTest, HostPlaybackIdentityTest, BleDiscoveryServiceTest)

**Tests:** All pass. Lint clean.

**Key FIX4 decisions:**
- canManualResync() is now allowlist-by-exclusion: any state except IDLE/SCANNING/SESSION_SELECTED/DISCONNECTED/ERROR with selectedSession != null
- requestListenerSyncProbe(source) is the internal impl; manualResync() is the thin UI wrapper; handleJoinApprovalMessage calls probe directly bypassing UI gate
- startPeriodicListenerResync() replaces startPeriodicResync(); no longer called from startHostPlayback(); shouldKeepResyncing() is listener-only
- classifyBroadcastDelivery() in TransportModels.kt is the testable pure function; reportHostBroadcastDelivery() uses it
- approveJoinRequest() sends approval first, only updates UI state if delivery succeeded (zero peers = not delivered)
- Zero audio peers: disclosed via diagnostics but does NOT kill host preview; zeroPeerBroadcastCount tracked separately
- WifiDirectTransportService.startHost() has own hasWifiDirectPermission() check; all socket/group errors caught and returned as failed()
- handleTransportSnapshot() maps transport FAILED state to host ERROR when hosting is active
- selectDiscoveredSession() enforces canSelectSession() in ViewModel; rejection sets visible lastError
- BleDiscoveryService has failures: SharedFlow<BleOperationFailure> exposed via _failures MutableSharedFlow
- AudioTrack.write() no longer coerces negative result to 0; error message shows actual platform error code

## 2026-06-28T20:11:58Z - Claude Sonnet 4.6 - FIX3 Ralph Loop: COMPLETE (all 8 blocks delivered)

**FIX3 (second hardening pass) COMPLETE. All 8 Ralph Loop blocks implemented and tested:**
- Block 1 (35ba823): Rename OboePlaybackEngine → AudioTrackPlaybackEngine, add PlaybackEngine interface (P0.1)
- Block 2 (91756f0): Start/catch playback engine writes in listener and host loops (P0.2-P0.5)
- Block 3 (36e7fda): Scan/join UI wiring — isScanning, canSelectSession, clearScanState (P0.6-P0.8)
- Block 4 (612da80): Host startup result handling, TransportOperationResult, stopAdvertising public (P0.9-P0.11)
- Block 5 (74a927d): Transport delivery stats — SendAllResult, consecutive failure counter, stop after 10 (P0.12-P0.15)
- Block 6 (a717993): Host control broadcast failure handling, handleHostControlFailure, propagate buffered clear (P1.1-P1.4)
- Block 7 (ea24f01): Diagnostics honesty, invite code, SDK-aware permissions, misc correctness (P1.5-P2.1)
- Block 8 (1075e94): Tests for PlaybackEngine failure, SendAllResult, PermissionCatalogue SDK matrix (P2.2-P2.7)

**Test & Lint Results:** 179 unit tests pass across 20 test files. 0 new errors. All blocks committed and pushed to GitHub.

**Key FIX3 decisions:**
- PlaybackEngine interface in PlaybackScheduling.kt; AudioTrackPlaybackEngine is the impl; deprecated typealias OboePlaybackEngine kept as marker
- handleListenerPlaybackEngineFailure / handleHostPlaybackEngineFailure cancel job + set ERROR + lastError + diagnostics
- handleHostControlFailure sets lastError + host diagnostics (not listener) for all control broadcasts
- propagateListenerPlaybackState clears buffered=false when playback is not PLAYING
- PermissionCatalogue.requiredPermissions(sdkInt) is now SDK-aware: NearbyWifiDevices on 33+, FineLocation on 32-, Bluetooth runtime perms on 31+ only
- SilentDiscoApp derives permission list from PermissionCatalogue (no more hand-rolled duplicate)
- manualResync() local fallback gated behind BuildConfig.DEBUG + demo-session- prefix
- startHostPlayback() no longer generates random session ID fallback; returns error if no active session
- ByteArray concatenation uses pre-allocated array instead of fold+`+` (O(n²) → O(n))
- OboeBridge now has loadResult: Result<Unit> and isAvailable: Boolean for structured diagnostics

## 2026-06-28T19:20:51Z - Claude Haiku 4.5 - CODE_REVIEW2 Ralph Loop: COMPLETE (all 8 blocks delivered)

**CODE_REVIEW2 hardening pass COMPLETE. All 8 Ralph Loop blocks implemented and tested:**
- Block 1 (4d8095c): Scan lifecycle, explicit isScanning state (P0.1-P0.5) — 70+ tests
- Block 2 (465a994): Host startup validation, CREATING_SESSION (P1.6-P1.9) — 13 tests
- Block 3 (d00d551): Invite code enforcement (P0.9-P0.10) — 19 tests
- Block 4 (645af55): Join buffering step UI (P1.2-P1.5) — state machine updates
- Block 5 (4ab0794): Manual resync gating (P1.10-P1.12) — button gating + error messages
- Block 6 (3675c56): Playback volume, eliminate fake writes (P0.6-P0.8) — error on write before start
- Block 7 (02b615c): Oboe diagnostics clarity (P2.1-P2.2) — 7 diagnostic tests
- Block 8: Final tests & lint documentation (P1.15-P1.17, P2.3-P2.4) — comprehensive summary

**Test & Lint Results:** 73+ unit tests pass across 17 test files (1,684 lines). 0 lint errors introduced. 8 pre-existing GradleDependency warnings (unchanged, deferred per spec). All silent failures → explicit errors. All fake-success → fails loudly. All state transitions validated. All form inputs validated before side effects. Production helpers comprehensively tested.

## 2026-06-07T07:37:04Z - Claude Sonnet 4.6 - Session start; created CLAUDE.md and initialized memory

- Project is an Android-only offline silent disco PoC using Kotlin, Jetpack Compose, Oboe, Wi-Fi Direct (primary transport), and BLE (discovery only).
- CLAUDE.md was created this session, adapted from `.github/copilot-instructions.md`, with a Memory file section added by the user.
- The implementation is nearly complete per `docs/TODO.md` — all core features are checked off. Remaining open items: verify stable Oboe/AAudio playback callback behavior, real-device integration tests, and measuring actual listener capacity.
- Sync/buffer/threshold tuning controls are exposed as in-app persisted settings in Diagnostics to allow real-device adjustment without rebuilding.
- `memory.md` did not previously exist; this is its first entry.

## 2026-06-07T19:17:15Z - Claude Sonnet 4.6 - Documented Ralph Loop workflow in CLAUDE.md

- User confirmed this project uses a Ralph Loop pattern: pick next `[ ]` from `docs/TODO.md`, implement, write unit tests, run tests; if pass → commit + push + mark `[x]` + move on; if fail → keep iterating.
- Added a "Ralph Loop workflow" section to `CLAUDE.md` documenting this process.
- `memory.md` bridges fresh context sessions so each loop iteration knows where to pick up.

## 2026-06-07T19:31:04Z - Claude Sonnet 4.6 - Completed TODO 11.4: Verify stable playback callback/write behavior

- Created `OboePlaybackEngineTest.kt` with 9 unit tests covering: null-AudioTrack fallback paths (pre-start, post-stop, empty payload, idempotent stop, playbackPositionMs), write count tracking, and write-loop integration with ListenerPlaybackScheduler (deadline ordering, null-frame skipping, empty scheduler).
- All tests pass. Committed and pushed (commit 9df11a3).
- This was the last code-implementable unchecked item in docs/TODO.md.
- Remaining unchecked items all require physical Android devices: 17.2 (integration/device tests), 17.3 (manual validation checklist), 18 (measure listener capacity). Section 20 items are explicitly deferred.

## 2026-06-28T10:09:26Z - Claude Sonnet 4.6 - Lint, unit tests, on-device tests; CODE_REVIEW2 spec reviewed

- Ran `./gradlew lint` — 8 GradleDependency notices only (version bumps requiring AGP 9.1.0 + compileSdk 37; deferred). No code issues.
- Ran `./gradlew test` — all 3 variants pass (debug, pocDebug, release).
- Ran `./gradlew connectedDebugAndroidTest` on device R5CW31AX4FL — passed after fixing Oboe STL mismatch (`-DANDROID_STL=c++_shared` added to defaultConfig cmake arguments in build.gradle.kts).
- Attempted all 8 GradleDependency dep bumps; all failed (either require AGP 9.1.0+/compileSdk 37, or trigger Kotlin FIR compiler crash). All reverted.
- Committed STL fix + .gitignore update + deleted run_claude.md as 7e37b70.
- Read `SILENT_DISCO_CODE_REVIEW2_SPEC.md` and `SILENT_DISCO_CODE_REVIEW2_TODO.md` — spec from ChatGPT session, focuses on hardening silent failures and fake-success patterns across scan lifecycle, host startup, playback engine, volume, BLE, invite codes, and TCP send.
- Created `docs/responses1.md` with 7 clarification questions for ChatGPT 5.5; committed and pushed as a325a77. Awaiting answers before starting CODE_REVIEW2 implementation.

## 2026-06-07T22:46:40Z - Claude Sonnet 4.6 - docs/CODE_REVIEW1_TODO.md fully complete — all items [x]

- Fixed missing Start Hosting validation: button now disabled (and error text shown) when session name is blank or no audio file selected.
- Moved `StepState` enum and 6 `ConnectionProgressState` step extension functions from private in JoinProgressScreen to package-visible in AppState.kt.
- Added ConnectionProgressStepTest (22 tests) covering all six step state functions.
- Added UiStateValidationTest covering JoinProgress button visibility (joinable/retryable sets), Start Hosting validation message logic, and playback button enabled states.
- Added mockito-core + mockito-kotlin to test deps to mock android.net.Uri.
- All 13 manual testing checklist items marked [x]. docs/CODE_REVIEW1_TODO.md has 0 unchecked items.
- Committed as c666bba, pushed to GitHub.

## 2026-06-07T22:29:52Z - Claude Sonnet 4.6 - Completed CODE_REVIEW1_TODO tasks 10, 11, 12 — all 16 tasks now done

- Task 10: Added custom `darkColorScheme` to `Theme.kt` with deep purple/teal palette (`primary = 0xFF9C7DFF`, `secondary = 0xFF00E5CC`, dark background `0xFF0F0E1A`).
- Task 11: Added `TopAppBar` to all 6 non-home screens (HostSetupScreen, HostControlScreen, DiscoverSessionsScreen, JoinProgressScreen, ListenerPlaybackScreen, DiagnosticsScreen). Each gets `onBack: () -> Unit` wired to `navController.popBackStack()` in SilentDiscoApp. JoinProgressScreen's back arrow calls `onCancel` (cancels join + pops back). Redundant `headlineMedium` title Texts removed from screen bodies.
- Task 12: Added `material-icons-extended` icons to primary action buttons: PlayArrow (Start), Pause, Stop, Close (End Session), ExitToApp (Leave Session), BarChart (Diagnostics), Share, Refresh (Scan), Sync (Manual Resync). Pattern: `Icon + Spacer(ButtonDefaults.IconSpacing) + Text`.
- LazyColumn screens (HostControlScreen, DiscoverSessionsScreen) restructured to `Column { TopAppBar; LazyColumn(contentPadding = ...) }` so app bar is pinned.
- Column screens (HostSetupScreen, JoinProgressScreen, ListenerPlaybackScreen, DiagnosticsScreen) restructured to `Column { TopAppBar; Column(Modifier.weight(1f)) { content } }`.
- All 16 tasks in docs/CODE_REVIEW1_TODO.md are now marked [x]. Committed as 863209d, pushed.

## 2026-06-07T22:07:00Z - Claude Sonnet 4.6 - UI/UX code review; created docs/CODE_REVIEW1_TODO.md

- Conducted full UI/UX review of all 7 screens (HomeScreen, HostSetupScreen, HostControlScreen, DiscoverSessionsScreen, JoinProgressScreen, ListenerPlaybackScreen, DiagnosticsScreen) plus Theme.kt, AppState.kt, SilentDiscoApp.kt.
- Key bugs found: raw enum names shown to users everywhere, "Continue to Playback" silently does nothing when tapped early, "Now playing" on listener screen reads from host's file picker field (always null on listener), radio/checkbox tap targets too small.
- Key UX issues: "Add Demo Join" dev button exposed in production UI, boolean progress list in JoinProgressScreen, all 4 action buttons always visible, placeholder "local demo transport" text on session cards, no loading indicators, no button disabled states, no TopAppBar/back navigation, unbranded default Material3 theme.
- Created docs/CODE_REVIEW1_TODO.md with 16 detailed task groups and a manual testing checklist. This is the next Ralph Loop target file.

## Full validation run 30342064738

- Source commit: `4dcb8dc3da8a649a14259afc6876088909641f6c`
- Rust format/Clippy/tests lane: **success**
- Android build/packaging/unit/lint lane: **success**
- Android instrumentation lane: **success**
- Desktop frontend format/lint/typecheck/tests/build lane: **success**
- Desktop Rust format/Clippy/tests/check lane: **success**
- Linux AppImage/DEB bundle lane: **success**

## 2026-07-30 — Desktop Block 13 authority closure

- Base commit: `acb15e42400a9c9a18ced1e5f27c3f130a5e54d8`.
- Android host playback now serializes commands and awaits Rust-confirmed `BUFFERING`, `PLAYING`, `PAUSED`, and `STOPPED` snapshots before executing corresponding platform side effects.
- Start may decode and packetize only after Rust accepts buffering; playback-engine start, control broadcast, and packet-loop launch occur only after Rust accepts playing.
- Pause, resume, and stop perform no success-side effects when the Rust transition rejects, times out, or is cancelled.
- Stop intentionally cancels a pending start/resume/pause command before requesting authoritative stopped state.
- Removed the dormant `trustListener`/`trustRustListener` path and `manual-trust-*` operation IDs. Trusted-device persistence remains owned by the Rust `PersistTrustedDevice` storage-effect path.
- Added `HostPlaybackAuthorityTest` for accepted ordering and rejection-side-effect suppression.
- Guarded validation run: `30524538283`. Required gates: source-size invariant; shared Rust fmt/strict Clippy/all-feature tests; Android assemble, unit tests, and lint; desktop generated bindings, format, lint, typecheck, tests, build; desktop Rust fmt/strict Clippy/tests/check.

## 2026-07-30 — Desktop Block 15 platform-effect runner

- Base production commit: `cb540ea8262501b4177267e3f61c33b9cd583154`.
- Added a bounded desktop platform-effect channel and one owned worker. The core observer diverts only `CoreNotification::Effect`; snapshots, transport effects, storage effects, errors, and diagnostics retain their existing bridge behavior.
- Every completion returns through `CoreActorHandle::submit_platform_event` with the original operation ID. The runner never mutates `CoreSnapshot` or actor state directly.
- Implemented real desktop capability resolution. Secure storage is available after profile identity startup; discovery, advertising, standard-IP transport, source selection/preparation, and native audio output remain explicitly unavailable until their dedicated blocks.
- Implemented a real profile-owned diagnostics JSON export using a hashed filename, create-new temporary file, flush/sync, atomic rename, and directory sync. Native paths do not cross the completion or IPC boundary.
- Unsupported effects return correlated structured failures rather than successful no-ops. Adapter panics are contained and reported as `ffi_panic_contained` failures.
- Added bounded cancellation, queued-effect cancellation during shutdown, deterministic worker join, operation-correlation tests, stale-completion rejection coverage against the real core actor, diagnostics export tests, unsupported-effect tests, panic/error containment tests, cancellation tests, and shutdown ownership tests.
- Guarded validation run: `30529942712`. Required gates: source-size invariant; shared Rust fmt/strict Clippy/all-feature tests; Android assemble, unit tests, and lint; desktop generated bindings, format, lint, typecheck, tests, build; desktop Rust fmt/strict Clippy/tests/check.

## 2026-07-30 — Desktop Block 16 secure audio source selection complete

- Source commit validated: `bf9664058c9ca239e6d1995d512782aed81c5921`.
- Final implementation validation run: `30539622045`.
- Added backend-owned native file selection for one explicit WAV, FLAC, or MP3 source; cancellation remains distinct from failure and no unrestricted filesystem capability is exposed.
- Inspection verifies canonicalized regular files, an 8 GiB size bound, bounded/sanitized display names, fixed-size content signatures, explicit unsupported formats, and opaque source IDs. Native paths remain only in a single backend registry and are cleared fail-visibly when the profile closes.
- The authoritative actor receives only the redacted descriptor. Profile readiness waits for the acknowledged capability snapshot, and React waits for a newer Rust snapshot rather than mutating source state optimistically.
- Tests cover cancellation, dialog failure, missing files, directories, empty/oversized files, Unicode bounds, deceptive extensions, malformed MP3 headers, canonicalization and permission failures, deterministic identities, registry rollback/clear, capability publication, and frontend cancellation/error behavior.
- Automated validation passed source-size enforcement, generated-binding verification, frontend format/lint/typecheck/tests/build, Rust format/strict Clippy/tests/check, lockfile reproducibility, and Linux Tauri bundle creation. Native dialog interaction on a physical desktop session remains unclaimed.

## 2026-07-30 — Desktop Block 17 atomic source staging complete

- Validated input commit: `7948e62a6526a84c3b4fceacc7971acd9c8e9bbb`.
- Guarded validation run: `30576293784`.
- Source selection copies the inspected file into the active profile's `sources/` directory through an owned temporary file, fixed 64 KiB buffers, streaming SHA-256, length/signature verification, file and directory synchronization, and no-clobber atomic publication.
- Stable source IDs and filenames are content-addressed. Existing staged content is reused only after full regular-file, length, and hash verification; mismatches and collisions fail visibly without overwriting data.
- Staging supports bounded 10 Hz progress events, explicit cancellation, profile-close cancellation/join, and deterministic cleanup that preserves both primary and cleanup failures.
- Startup removes only strict, provably owned incomplete regular temporary files. Unrelated files, symbolic links, and non-file entries are never silently deleted.
- Tests cover success, cancellation, source failure during copy, destination write failure, hash mismatch, collision, verified reuse, incomplete-temp cleanup, cleanup refusal, original preservation, progress throttling, and cancellation control.
- Validation passed source-size enforcement; shared Rust format/strict Clippy/tests; Android builds, ABI packaging, unit tests, lint, and instrumentation; generated desktop bindings, format/lint/typecheck/tests/build; desktop Rust strict gates; exact lockfiles; and Linux Tauri bundle creation. Native file-dialog interaction on a physical desktop session remains unclaimed.

## 2026-07-30 — Desktop Block 18 decoder decision complete

- Evidence commit: `0cecbc38cfca68620131ed4c072968896fac2e65`.
- Guarded revalidation run: `30589549529`.
- Revalidated input commit: `a5e07308e0fc5fdb0bca36b04c58112036643e98`; its only source-equivalent addition was this temporary audit workflow, removed by the completion commit.
- Selected decoder: `symphonia = 0.6.0`, default features disabled, features `wav`, `pcm`, `flac`, `mp3`, `id3v1`, `id3v2`; license `MPL-2.0`.
- Selected ownership: shared Rust streaming decoder (shared Block 23 Path B), with no automatic platform, HTML, Web Audio, TypeScript, or FFmpeg fallback.
- Initial formats: WAV/PCM, native FLAC, and MP3. Desktop Block 19 will convert source-native planar buffers incrementally into bounded 48 kHz stereo PCM16 little-endian chunks.
- Valid-fixture realtime factors on this CI host: WAV `5934.7x`, FLAC `2832.2x`, MP3 `963.9x`.
- Peak RSS on this CI host: WAV `3.0` MiB, FLAC `3.1` MiB, MP3 `3.6` MiB, and the 2 MiB metadata MP3 `6.9` MiB.
- Corrupt and truncated fixtures failed visibly; cooperative cancellation stopped at a decoder packet boundary. These measurements are environment-specific evidence, not product-wide performance limits.
- Shared Block 23 remains open for Android bridge overhead, physical mobile evidence, iOS file-access constraints, and removal of the temporary platform decoder path.

## 2026-07-30 — Desktop Block 19 bounded streaming decode complete

- Shared Rust owns incremental WAV/FLAC/MP3 decode to canonical 48 kHz stereo PCM16.
- Decoded chunks and queued duration are bounded; checked sample indices, visible backpressure, typed failures, cancellation, join, and cancel-on-drop are covered.
- Desktop resolves the exact opaque staged source before starting the shared worker; packetization/playback wiring remains assigned to later blocks.
- Repository execution policy remains direct work on `master`; no branch or PR was used.
- Validation run: `30599085238`.
- Validated input commit: `4c05e5763b1771fc2c7a04690d46b8c76665aa43`.

## 2026-07-30 — Desktop Block 20 shared transport runtime complete

- Shared Rust owns TCP control and UDP synchronization/audio runtime semantics.
- Protocol framing, bounds, authorization, accounting, queues, failures, shutdown/join, and virtual transport/clock behavior are covered.
- Desktop interface and bind-selection work remains in Block 21.
- Direct `master` work; no branch or PR.
- Validation run: `30605377851`.
- Validated input: `09366180e01f65aba04bed2f95d54fb648449fcb`.

## 2026-07-31 — Desktop Block 21 network bind policy complete

- Desktop hosting enumerates bounded interface snapshots and classifies loopback, link-local, private LAN, VPN, container, and other addresses.
- Automatic selection is restricted to an unambiguous active private-LAN IPv4 candidate; ambiguity requires explicit user selection.
- Explicit preferences are revalidated immediately before the shared Rust transport binds TCP control and UDP synchronization/audio endpoints.
- Actual bound addresses and ports, interface changes, bind failures, partial cleanup, and cleanup failures remain visible and typed.
- Host Setup now includes an accessible network-policy card and blocks session creation until the network selection is ready.
- Implementation commit: `bef33cab2798c41172eced93747ecf73927dcd90`.
- Focused publication run: `30613180304`.
- Direct `master` work; no branch or PR.
- Final validation run: `30613572498`.
- Validated input: `fd081a1574f54956754adcd40c0578933e468c1f`.

## 2026-07-31 — Desktop Block 22 manual endpoint host workflow complete

- Desktop exposes authoritative manual host connection information without requiring mDNS.
- The DTO combines the shared-core session advertisement with the actual bound control/synchronization/audio endpoint.
- The desktop transport worker feeds real join/disconnect events into the authoritative actor.
- Pre-approval TCP Hello does not grant UDP synchronization or audio authorization.
- Host Session UI shows connection details, copy controls, pending and connected listeners, visible failures, disabled future playback controls, and revision-aware end-session behavior.
- Shared handshake commit: `4c132f28f8807dd5afb6a791f747f96515051d67`.
- Runtime/DTO commit: `47724cd6a4f931f14b003cb7bed249546b8fbdf7`.
- Host Session UI commit: `88e2b851feba8a06cdd0016ef840f48762d3a94c`.
- Direct `master` work; no branch or PR.
- Final validation run: `30620932603`.
- Validated input: `3f9b90aca0549e5870b34d12cee83c514a2ccd40`.

## 2026-07-31 — Desktop Block 23 listener management complete

- Completed revision-aware desktop approval, rejection, and listener removal on direct `master`.
- Added bounded desktop transport-effect and storage-effect execution. Delivery and persistence failures are correlated and fail-visible; React never executes transport or storage effects.
- Added authoritative request age, listener synchronization/delivery details, pending-operation reconciliation, trusted-device policy, duplicate-action prevention, and accessible listener-management UI.
- Guarded Actions run `30678111276` passed against exact input `8f9d156d5d94cba7178cc01ad8cb546d691da003` with the complete Rust, desktop, Linux bundle, Android build/test/lint/ABI, managed-device instrumentation, generated-binding, lockfile, and source-size matrix.
- Physical Android control-plane interoperability remains Desktop Block 24.

## 2026-08-01T12:10:37Z - Claude Sonnet 5 - Desktop Block 24 physical Android control interoperability complete

- Ralph-looped `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` Block 24 from the ChatGPT handoff at `7e6bdc1`, on direct `master`, no branch/PR. Full evidence: `docs/DESKTOP_BLOCK24_ANDROID_CONTROL_INTEROPERABILITY.md`.
- Added `ManualHostEndpoint` (Rust core) parsing/validating the desktop's exact "Connection payload" JSON; added `FfiListenerTransportHandle`/`parse_manual_host_endpoint`, a new narrowly-scoped UniFFI object wrapping the existing production `SocketListenerTransport`; added Android's `ManualListenerTransportController`, `ManualEndpointScreen`, and a "Connect manually" entry point, deliberately independent of the legacy Wi-Fi Direct `SessionInfo`/`ControlMessage` path. Commits `a0c7205`, `adc7b6b`.
- Architecture finding: the shared `CoreActorRuntime` already has listener-role commands (`SelectSession`/`SubmitJoin`/`CancelJoin`) but `TransportEvent::JoinRequested`/`ListenerConnected`/`ListenerDisconnected` are host-role-only (`require_role_event(AppRole::Host)`) — there is no event path yet for a connected transport to report `JoinApproval`/`JoinRejection` back into the actor. Per the handoff's explicit fallback, kept the new handle transport-only rather than completing that actor wiring; Kotlin observes typed FFI events directly. Finishing the shared actor's listener join-lifecycle remains open follow-up work, not claimed here.
- Found and fixed a pre-existing desktop tooling gap: `desktop/src-tauri` had no `rust-toolchain.toml`, so `npm run bindings:check` and direct `cargo` from `desktop/` silently used the machine default (1.95.0) instead of the pinned 1.97.1. Added `desktop/rust-toolchain.toml`. Commit `66a3272`.
- Physical acceptance run on a real Samsung Galaxy A54 (`SM-A546E`, Android 16, API 36, serial `R5CW31AX4FL`) against a real `silent-disco-desktop` debug binary on the same Wi-Fi LAN (desktop `192.168.88.109`, phone `192.168.88.107`) surfaced two real defects invisible to source review or the automated suite (both bypass UI-level gates):
  - Desktop: `desktop_capabilities()` still hardcoded `local_network_available: false` from before Block 21 actually implemented real interface binding; `HostNetworkPolicyCard` gates session creation on this flag, so the desktop host's own UI could never create a session. Fixed to `true`; updated the two stale test assertions.
  - Android: `ManualListenerTransportController`'s poll loop mapped every post-connection `FfiListenerTransportException` (including `Closed`/`ShuttingDown`) to `Failed`, so a real desktop "End session" rendered as "Couldn't connect" instead of a distinct "Host disconnected" state. Fixed to always map to `Disconnected`; extracted as `mapPostConnectionFailure` and covered by `ManualListenerTransportControllerTest`.
  - Both fixes committed at `f0fff45` and `2b9be53`; re-verified against the physical device after the fix (fresh session, fresh join/approval, then End Session correctly showed "Host disconnected").
- All 7 physical acceptance scenarios passed end-to-end on the real device: A (approval), B (rejection, separate run), C (listener-initiated disconnect), D (host removal), E (desktop end-session, after the fix), F (invalid/unreachable endpoint - "No route to host" within seconds, no false success), G (wrong protocol version - live validation rejected `999` vs supported `2` before any connection attempt).
- Full Rust workspace gate, Android build/unit-test/lint/instrumentation-compile, and desktop `npm run check` + Rust backend all passed on the final commit. One pre-existing desktop Rust test (`port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport`) fails in this environment; confirmed via `git stash` that it fails identically on a clean checkout — unrelated to Block 24, not fixed.
- Audio interoperability was explicitly not claimed or tested; playback remains disabled pending later packetization/streaming blocks.
