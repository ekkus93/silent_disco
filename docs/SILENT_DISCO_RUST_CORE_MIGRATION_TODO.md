# Silent Disco Shared Rust Core Migration TODO

**Status:** Ready for implementation  
**Date:** 2026-07-25  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Specification:** `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`

---

## 0. How to execute this TODO

This is a staged architectural migration, not a rewrite-in-place.

Use the Ralph Loop:

1. Select the next unchecked task or the next coherent sub-block explicitly allowed by the task.
2. Read the specification and all referenced production files before editing.
3. Implement the smallest complete change.
4. Add production-facing tests. Do not test copied local logic.
5. Run the validation commands listed for the block.
6. Fix every failure before proceeding.
7. Mark only completed tasks `[x]`.
8. Commit and push the completed block.
9. Record material architectural decisions, failures, and device results in `memory.md`.

### Non-negotiable rules

- [ ] Do not leave `master` unable to build at the end of a committed block.
- [ ] Do not introduce a broad `try/catch`, `runCatching`, Rust `unwrap`, or Rust `expect` that converts a real failure into log-only behavior.
- [ ] Do not claim an operation succeeded until the responsible transport, platform adapter, audio engine, or database operation reports success.
- [ ] Do not silently fall back to an in-memory database, fake transport, fake playback engine, anonymous identity, or demo implementation.
- [ ] Do not allow Kotlin and Rust to remain competing authoritative state owners after the migration block that transfers a responsibility.
- [ ] Do not call UniFFI, JNI, SQLite, networking, logging, allocation-heavy code, or blocking synchronization from the audio render callback.
- [ ] Do not delete or recreate a user database automatically after migration or integrity failure.
- [ ] Do not create or reference additional assistant-generated design documents unless they are also committed at the exact path referenced.
- [ ] Do not add `Co-Authored-By:` lines to commit messages; this repository rejects them.

### Baseline validation commands

Run as applicable after every block:

```bash
./gradlew test
./gradlew lintDebug

cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Instrumented tests are required when a block changes Android native loading, Oboe, file paths, permissions, networking, lifecycle behavior, or bindings.

---

# Phase 1 — Preserve current behavior and establish the Rust workspace

## Block 1 — Record the migration baseline

### 1.1 Capture current build and test results

- [x] Run `./gradlew test`.
- [x] Run `./gradlew lintDebug`.
- [ ] Run the current connected Android test suite on an available physical device.
- [ ] Record the exact commands, device model, Android version, pass/fail count, and relevant warnings in `memory.md`.
- [ ] Confirm the current Android APK starts and reaches the Home screen.

### 1.2 Inventory current ownership

Create a section in `memory.md` listing the current Kotlin owner for each responsibility:

- [x] protocol models/serialization;
- [x] host and listener lifecycle;
- [x] join approval/rejection;
- [x] clock synchronization;
- [x] packetization;
- [x] jitter buffer/playback scheduling;
- [x] audio output;
- [x] BLE discovery;
- [x] Wi-Fi Direct establishment;
- [x] TCP channel transport;
- [x] settings/trusted-device persistence;
- [x] diagnostics.

This inventory is a migration checklist, not a new design document.

### 1.3 Add compatibility fixtures

Add versioned test fixtures under:

```text
app/src/test/resources/rust-migration/
├── protocol/
├── sync/
├── packetization/
├── state/
├── tuning/
└── persistence/
```

- [x] Add representative control-message fixtures for every existing message variant.
- [x] Add sync request/response samples with expected offset, RTT, and confidence outputs.
- [x] Add PCM packetization input and expected packet headers/payload hashes.
- [x] Add state-transition tables for host and listener workflows.
- [x] Add tuning normalization edge cases.
- [x] Add current settings/trust persistence examples.
- [x] Ensure fixtures contain no device secrets or private user information.

### 1.4 Add JVM tests that verify the fixtures against current Kotlin behavior

- [x] Tests deserialize or construct production Kotlin models and compare them to the fixtures.
- [x] Tests use production sync and packetization functions.
- [x] Tests cover error/failure transitions, not only happy paths.
- [x] Tests fail clearly when a fixture changes.

**Acceptance:** Existing Android behavior has an executable compatibility baseline before Rust code replaces it.

---

## Block 2 — Create the Rust workspace

### 2.1 Create workspace files

Create:

```text
rust/Cargo.toml
rust/Cargo.lock
rust/rust-toolchain.toml
rust/silent-disco-core/Cargo.toml
rust/silent-disco-core/src/lib.rs
rust/silent-disco-ffi/Cargo.toml
rust/silent-disco-ffi/src/lib.rs
rust/silent-disco-test-support/Cargo.toml
rust/silent-disco-test-support/src/lib.rs
```

Use a workspace shape similar to:

```toml
# rust/Cargo.toml
[workspace]
resolver = "2"
members = [
    "silent-disco-core",
    "silent-disco-ffi",
    "silent-disco-test-support",
]

[workspace.package]
edition = "2024"
license = "MIT"
rust-version = "1.XX"

[workspace.lints.rust]
unsafe_code = "deny"

[workspace.lints.clippy]
all = "deny"
pedantic = "warn"
```

- [x] Pin an available stable Rust toolchain in `rust-toolchain.toml`.
- [x] Commit `Cargo.lock` because the workspace produces application libraries.
- [x] Set `unsafe_code = "deny"` for `silent-disco-core`.
- [x] Permit narrowly scoped unsafe code only in `silent-disco-ffi` with documented safety invariants.

### 2.2 Add initial crate APIs

- [x] `silent-disco-core` exports a version record and one deterministic smoke function.
- [x] `silent-disco-ffi` depends on the core but contains no domain logic.
- [x] `silent-disco-test-support` provides fixture-loading helpers only.

### 2.3 Add quality scripts

Create repository scripts or Gradle tasks for:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

- [x] The commands work from a clean checkout.
- [x] No script relies on an absolute path.
- [x] Failure returns nonzero status.

### 2.4 Update ignore/build metadata

- [x] Ignore Rust `target/` output.
- [x] Do not ignore generated artifacts that the selected binding policy requires to be checked in.
- [x] Document the Rust commands in the existing developer guidance (`CLAUDE.md` if appropriate).

**Acceptance:** Rust format, clippy, and tests pass; Android build/tests remain unchanged and passing.

---

## Block 3 — Add Android Rust build and load smoke test

### 3.1 Select reproducible Android build integration

- [x] Build Rust for each Android ABI currently supported by the application.
- [x] Integrate the build through Gradle; no manual `.so` copying.
- [x] Reuse the repository's pinned Android NDK version.
- [x] Ensure debug and release variants select intentional Rust profiles.
- [x] Add a clean task that removes generated native output safely.

### 3.2 Add minimal exported native version API

Before UniFFI is added, expose a tiny C function:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn silent_disco_core_abi_version() -> u32 {
    1
}
```

The final annotation must match the pinned Rust edition/toolchain.

- [x] Add a Kotlin/JNI or small C++ bridge call that reads the version.
- [x] Do not call this from the real-time path.
- [x] Fail visibly if the library cannot load or returns an unsupported version.

### 3.3 Add instrumented smoke test

- [ ] Test loads the Rust library on a physical Android device.
- [ ] Test verifies the ABI version.
- [x] Test reports the device ABI in failure output.
- [x] Remove any fallback that returns a hard-coded successful version when native loading fails.

**Acceptance:** Android debug APK links and loads Rust on at least one physical device; existing tests still pass.

---

# Phase 2 — Portable domain types, errors, protocol, and synchronization

## Block 4 — Implement Rust domain IDs, enums, and structured errors

### 4.1 Add strongly typed identifiers

Create Rust newtypes for:

- [x] `SessionId`;
- [x] `StreamId`;
- [x] `DeviceId`;
- [x] `RequestId`;
- [x] `OperationId`;
- [x] `DiagnosticRunId`;
- [x] monotonic milliseconds;
- [x] packet sequence;
- [x] sample index.

Requirements:

- [x] bounded string lengths;
- [x] validation on construction;
- [x] no implicit acceptance of blank IDs;
- [x] deterministic serialization;
- [x] useful `Display` without leaking secrets.

### 4.2 Port domain enums

Port and test semantic equivalents of:

- [x] application role;
- [x] approval mode;
- [x] host lifecycle;
- [x] listener lifecycle;
- [x] playback state;
- [x] transport state;
- [x] sync confidence;
- [x] trust state;
- [x] delivery severity.

Do not expose Kotlin/Compose labels from Rust.

### 4.3 Implement `CoreError`

Use a stable shape:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub code: CoreErrorCode,
    pub message: String,
    pub subsystem: CoreSubsystem,
    pub severity: ErrorSeverity,
    pub retryable: bool,
    pub operation_id: Option<OperationId>,
    pub context: Vec<ErrorContextEntry>,
}
```

- [x] Add stable error codes for validation, protocol, transport, synchronization, audio, storage, platform, FFI, queue overflow, and shutdown.
- [x] Bound context entry count and key/value lengths.
- [x] Do not use one generic `Unknown` error for known failure paths.
- [x] Add tests for conversion without losing the original operation/subsystem.

**Acceptance:** Core domain types compile without Android dependencies and are comprehensively unit tested.

---

## Block 5 — Implement Rust protocol framing and golden vectors

### 5.1 Define protocol version 2

The Rust protocol is authoritative. Define:

- [x] fixed magic;
- [x] protocol version field;
- [x] message-kind field;
- [x] flags;
- [x] payload length;
- [x] maximum control-frame size;
- [x] maximum audio datagram size;
- [x] maximum identifier/string sizes;
- [x] network byte order.

Do not silently reinterpret current Kotlin serialization as the permanent cross-platform format.

### 5.2 Implement control models

Port semantic equivalents for:

- [x] hello/session announcement;
- [x] join request;
- [x] join approval;
- [x] join rejection;
- [x] heartbeat;
- [x] stream start;
- [x] pause;
- [x] stop;
- [x] disconnect;
- [x] resync notice.

### 5.3 Implement sync and audio headers

- [x] Sync request and response use fixed-width bounded fields.
- [x] Audio header includes session/stream identity, sequence, sample rate, channels, samples per packet, sample index, host presentation timestamp, payload length, and integrity field.
- [x] Payload length is checked before allocation/copy.
- [x] Unsupported codec/sample format fails explicitly.

### 5.4 Add codec implementation

- [x] Select and pin the bounded control payload format.
- [x] Keep framing independent from the payload codec.
- [x] Parser reads the fixed header before allocating the payload.
- [x] Oversized frames fail before allocation.
- [x] Unknown versions and kinds fail with stable error codes.
- [x] Trailing bytes and truncated input fail.

### 5.5 Add golden vectors

Create Rust-owned fixtures under:

```text
rust/silent-disco-core/testdata/protocol/v2/
```

- [x] One vector per message kind.
- [x] Boundary-size vectors.
- [x] Malformed/truncated/oversized vectors.
- [x] Deterministic payload hashes.
- [x] Tests decode and re-encode byte-identically where canonical encoding is required.

### 5.6 Add parser hardening

- [x] Property tests or fuzz targets for frame parsing.
- [x] No panic for arbitrary input.
- [x] Bounded memory usage.
- [x] Diagnostic counters distinguish malformed, unsupported, unauthorized, stale, and oversized frames.

**Acceptance:** Rust protocol tests pass and the wire format is fully specified by executable vectors.

---

## Block 6 — Port clock synchronization to Rust

### 6.1 Implement monotonic time types

- [x] Separate host and local monotonic timestamp types where helpful.
- [x] Reject impossible timestamp orderings.
- [x] Avoid wall-clock time in scheduling calculations.

### 6.2 Port estimator behavior

Implement and test:

- [x] four-timestamp offset calculation;
- [x] RTT calculation;
- [x] correlation-ID matching;
- [x] sample history window;
- [x] outlier rejection;
- [x] low-RTT preference;
- [x] confidence classification;
- [x] drift threshold;
- [x] initial-sync and periodic-sync decisions.

### 6.3 Verify Kotlin compatibility fixtures

- [x] Run Kotlin baseline fixtures through Rust.
- [x] Differences require either a Rust fix or an explicitly documented intentional behavior change in `memory.md`.
- [x] Add edge cases for overflow, duplicate response, stale correlation ID, high RTT, and negative/invalid ordering.

### 6.4 Expose pure FFI sync smoke API

- [x] Add temporary or permanent UniFFI-friendly sync records.
- [x] Kotlin test invokes Rust estimator with a fixture.
- [x] Do not maintain a new Kotlin estimator wrapper that duplicates calculations.

**Acceptance:** Rust produces approved sync results and all estimator tests pass on host and Android.

**Physical-device status:** Host tests, JVM tests, Android builds, four-ABI packaging, and compilation of the Android instrumentation-test APK pass. Execution of `RustSyncEstimatorInstrumentedTest` on a physical Android device is **NOT RUN**; Block 6 physical-Android acceptance remains open until that test is executed and recorded.

---

# Phase 3 — Rust-owned SQLite and data migration

## Block 7 — Implement the Rust SQLite worker

### 7.1 Add storage module and database thread

Use one dedicated worker that owns the SQLite connection.

Conceptual message shape:

```rust
enum DatabaseCommand {
    LoadSettings { reply: ReplyPort<Result<TuningSettings, CoreError>> },
    SaveSettings { value: TuningSettings, reply: ReplyPort<Result<(), CoreError>> },
    ListTrustedDevices { reply: ReplyPort<Result<Vec<TrustedDevice>, CoreError>> },
    UpsertTrustedDevice { value: TrustedDevice, reply: ReplyPort<Result<(), CoreError>> },
    DeleteTrustedDevice { device_id: DeviceId, reply: ReplyPort<Result<(), CoreError>> },
    BeginSession { value: SessionRecord, reply: ReplyPort<Result<(), CoreError>> },
    EndSession { value: SessionOutcome, reply: ReplyPort<Result<(), CoreError>> },
    StoreDiagnosticRun { value: DiagnosticRun, reply: ReplyPort<Result<(), CoreError>> },
    CheckpointAndClose { reply: ReplyPort<Result<(), CoreError>> },
}
```

The exact channel type may differ.

- [x] Queue is bounded.
- [x] Full queue returns visible `StorageBusy`; no command is dropped.
- [x] Worker has explicit start, stop, and join lifecycle.
- [x] Database connection never crosses into the audio callback.

### 7.2 Configure SQLite explicitly

- [x] Enable and verify foreign keys.
- [x] Request and verify WAL mode where supported.
- [x] Set bounded busy timeout.
- [x] Select and document synchronous policy.
- [x] Record SQLite library version in diagnostics.
- [x] Fail initialization if required durability settings cannot be established.

### 7.3 Add structured storage errors

- [x] Map open, pragma, migration, query, transaction, constraint, busy, corruption, and close errors separately.
- [x] Preserve operation and schema version context.
- [x] No `unwrap` or `expect` on production database results.

**Acceptance:** Database worker tests prove serialized ownership, bounded queue behavior, close/join, and explicit failure mapping.

**Implementation status:** Complete. Block 7 provides worker infrastructure and connection policy only. Schema migrations, tables, repositories, and Android data import remain deferred to Block 8 and later blocks.

---

## Block 8 — Implement schema and migrations

### 8.1 Add migration framework

- [x] Ordered immutable migrations compiled into Rust.
- [x] Each migration has version and checksum.
- [x] Migration table records version, timestamp, and checksum.
- [x] Entire migration runs transactionally.
- [x] Checksum mismatch is fatal.
- [x] Newer unsupported schema is fatal.
- [x] Failed migration rolls back.
- [x] No automatic delete/recreate.

### 8.2 Create initial schema

Implement tables required by the spec:

- [x] `schema_migrations`;
- [x] `app_settings`;
- [x] `trusted_devices`;
- [x] `session_history`;
- [x] `diagnostic_runs`;
- [x] required indexes and foreign keys.

### 8.3 Implement repositories

- [x] Settings load/save with Rust validation.
- [x] Trusted-device list/get/upsert/delete.
- [x] Session begin/end/update.
- [x] Diagnostic summary insert/query/export.
- [x] No raw SQL crosses FFI.

### 8.4 Add migration tests

- [x] Empty database to latest.
- [x] Reopen latest database.
- [x] Failed migration rollback.
- [x] Checksum mismatch.
- [x] Newer schema rejection.
- [x] Constraint violation mapping.
- [x] Concurrent request ordering through worker.
- [x] Integrity of Unicode names and binary public keys.

**Acceptance:** All database tests pass using temporary files, not only `:memory:` databases.

**Implementation status:** Complete. PR #31 merged as `4dd2de7c54942f047d4fd47ca8c73ae73721fabe`. Guarded validation run `30184437336` passed Rust formatting, strict Clippy, all Rust tests, Android debug/PoC-debug/release/instrumentation builds, four-ABI JNI packaging, Android unit tests, and Android lint. Android database path selection and legacy `SharedPreferences` import remain Block 9.

---

## Block 9 — Integrate Rust database with Android and migrate current persisted data

### 9.1 Add Android path provider

Create `AndroidDatabasePathProvider` that:

- [x] selects an application-private database path;
- [x] creates only the parent directory;
- [x] returns the complete path to Rust;
- [x] never opens SQLite from Kotlin;
- [x] applies Android backup policy intentionally.

### 9.2 Add one-time legacy import

Current tuning/trust data in `SharedPreferences` must be handled explicitly.

- [x] Define a versioned `LegacyAndroidImport` record.
- [x] Kotlin reads only the known legacy keys.
- [x] Kotlin passes typed legacy values to Rust once.
- [x] Rust validates and imports them transactionally.
- [x] Rust records import completion in SQLite.
- [x] Kotlin deletes legacy domain keys only after Rust reports committed success.
- [x] Failure leaves legacy data intact and surfaces an error.
- [x] Repeated startup is idempotent.

### 9.3 Remove direct domain persistence

- [x] `MainViewModel` no longer writes tuning/trusted-device domain state to `SharedPreferences` after successful migration.
- [x] Platform-only preferences may remain if documented.
- [x] No silent fallback to old preferences if Rust database fails.

### 9.4 Add Android instrumentation tests

- [x] First run creates database.
- [x] Legacy settings import.
- [x] Legacy trust import.
- [x] Import failure preserves legacy values.
- [x] Reopen loads Rust values.
- [x] Database migration failure displays fatal/recoverable state.

**Acceptance:** All SQLite domain access is Rust-owned; Android has no production SQL and no duplicate domain persistence.

**Implementation status:** Complete. PR #35 merged as `5fc5ae966b1157b2cd5887c10d3522da81856f8f`. Permanent CI run `30187155765` passed Rust formatting, strict Clippy, all Rust tests, Android debug/PoC-debug/release and instrumentation-APK builds, four-ABI JNI packaging, Android unit tests, and Android lint. `AndroidRustDomainStoreInstrumentedTest` is compiled and packaged but physical-device execution is **NOT RUN**; device acceptance remains open until its command and device details are recorded.

---

# Phase 4 — Authoritative Rust actor and UniFFI control plane

## Block 10 — Implement commands, events, effects, snapshots, and actor runtime

### 10.1 Define core records

Implement:

- [x] `CoreCommand`;
- [x] `PlatformEvent`;
- [x] `TransportEvent`;
- [x] `AudioEvent`;
- [x] `StorageEvent`;
- [x] `PlatformEffect`;
- [x] `CoreNotification`;
- [x] `CoreSnapshot`;
- [x] `CommandReceipt`;
- [x] operation IDs and snapshot revisions.

### 10.2 Implement serialized actor

- [x] One actor owns mutable domain state.
- [x] Commands/events are processed in FIFO order within a documented source-order policy.
- [x] Queue is bounded.
- [x] Queue overflow is visible and does not drop silently.
- [x] Long work is delegated to workers/effects.
- [x] Results return as events with operation IDs.
- [x] Stale or duplicate completions are rejected.

### 10.3 Implement notification dispatcher

- [x] Notifications are emitted off the actor lock.
- [x] Effects and errors cannot be silently dropped.
- [x] Snapshot coalescing, if implemented, guarantees latest revision delivery.
- [x] Observer failure becomes a bridge error.
- [x] Notification worker has explicit shutdown/join.

### 10.4 Add actor tests

- [x] deterministic command sequence;
- [x] invalid command rejection without mutation;
- [x] effect and completion correlation;
- [x] stale completion rejection;
- [x] queue overflow;
- [x] observer failure;
- [x] monotonic snapshot revisions;
- [x] shutdown while operations pending;
- [x] no notification under state lock.

**Acceptance:** A host-independent Rust test can drive a complete simulated host/listener state flow through commands/events and snapshots.

**Implementation status:** Complete. PR #38 merged the authoritative actor runtime and its strict repair pass. Permanent Desktop CI run `30339287568` and repository CI run `30339287556` passed Rust formatting, Clippy with warnings denied, Rust tests, Android builds/tests/lint, generated-artifact checks, and Linux desktop bundle smoke validation. The actor remains platform-independent; UniFFI and Android `CoreFacade` work remain Block 11.

---

## Block 11 — Add UniFFI API and generated Kotlin bindings

### 11.1 Configure UniFFI

- [ ] Add pinned UniFFI dependencies and generation configuration.
- [ ] Choose checked-in generated bindings or deterministic build generation.
- [ ] Add a verification command that fails when generated bindings are stale.
- [ ] Expose core, binding, database-schema, and wire-protocol versions.

### 11.2 Expose `CoreHandle`

UniFFI surface must support:

- [ ] `open(config, observer)`;
- [ ] `submit_command(command)`;
- [ ] `submit_platform_event(event)`;
- [ ] `current_snapshot()`;
- [ ] audio engine token acquisition;
- [ ] `shutdown()`.

- [ ] All fallible methods return typed errors.
- [ ] No raw pointer is exposed through UniFFI as an unrestricted integer.
- [ ] Opening twice on the same mutable database path is prevented or explicitly coordinated.

### 11.3 Add Android `CoreFacade`

Create a thin Kotlin facade that:

- [ ] opens Rust on a background dispatcher;
- [ ] adapts observer notifications to `StateFlow`/event flow;
- [ ] marshals presentation state to the main thread;
- [ ] translates UI actions into `CoreCommand`;
- [ ] forwards platform completion facts as `PlatformEvent`;
- [ ] exposes startup failure explicitly;
- [ ] shuts down deterministically.

Do not put domain rules in `CoreFacade`.

### 11.4 Add binding tests

- [ ] Rust open and version check from JVM/instrumented Android.
- [ ] Submit command and receive revisioned snapshot.
- [ ] Receive platform effect.
- [ ] Report completion event.
- [ ] Structured error survives Rust→Kotlin conversion.
- [ ] Observer exception is contained.
- [ ] Shutdown is idempotent and visible.

**Acceptance:** Android can run a simulated Rust-domain flow while the existing Kotlin domain remains production-active behind a temporary feature flag.

---

# Phase 5 — Transfer authoritative domain state to Rust

## Block 12 — Port host draft validation and host lifecycle

### 12.1 Port host draft/settings validation

- [ ] Session-name validation.
- [ ] Audio-source-required validation.
- [ ] Invite-code rules.
- [ ] Tuning normalization and cross-field constraints.
- [ ] Tests use Rust production functions.

### 12.2 Port host lifecycle

Implement legal transitions for:

- [ ] idle;
- [ ] creating session;
- [ ] advertising/waiting;
- [ ] ready;
- [ ] streaming;
- [ ] paused;
- [ ] ending;
- [ ] error/retry.

### 12.3 Port join-request approval logic

- [ ] pending request insertion/deduplication;
- [ ] invite-code validation;
- [ ] approval delivery-first behavior;
- [ ] rejection delivery-first behavior;
- [ ] trusted-device policy;
- [ ] partial/zero-recipient delivery reporting;
- [ ] stale request rejection.

### 12.4 Route Android host UI through Rust

- [ ] Host screen actions submit Rust commands.
- [ ] Platform BLE/network actions execute Rust effects.
- [ ] Completion facts return as platform events.
- [ ] Compose renders Rust snapshot.
- [ ] Kotlin no longer mutates authoritative host lifecycle state.

### 12.5 Remove transferred Kotlin logic

- [ ] Remove or isolate old host validation/state functions.
- [ ] No hidden fallback to Kotlin on Rust error.
- [ ] Delete obsolete tests after equivalent Rust/integration coverage exists.

**Acceptance:** Host creation, approval, rejection, pause/stop/end intent state is Rust-authoritative on Android.

---

## Block 13 — Port listener lifecycle and recovery

### 13.1 Implement listener state machine

Cover:

- [ ] idle;
- [ ] scanning;
- [ ] session selected;
- [ ] join requested;
- [ ] awaiting approval;
- [ ] approved;
- [ ] connecting;
- [ ] initial synchronization;
- [ ] buffering;
- [ ] playing;
- [ ] reconnecting;
- [ ] desynchronized;
- [ ] disconnected;
- [ ] error.

### 13.2 Port progress and recovery rules

- [ ] selection guard during active join;
- [ ] initial versus periodic sync state preservation;
- [ ] transport failure to listener error;
- [ ] stale buffered/playing flags cleared;
- [ ] disconnect cleanup;
- [ ] retry eligibility;
- [ ] session disappearance;
- [ ] host rejection visibility.

### 13.3 Route Android listener UI through Rust

- [ ] Discovery facts enter as platform events.
- [ ] Join/cancel/retry/resync actions are Rust commands.
- [ ] UI progress renders Rust snapshot.
- [ ] Kotlin does not maintain duplicate listener lifecycle state.

### 13.4 Add parity/integration tests

- [ ] Reproduce FIX3/FIX4/FIX5 listener hardening expectations in Rust tests.
- [ ] Android tests verify UI actions map to Rust commands.
- [ ] Failure messages remain visible and are not overwritten by later success text.

**Acceptance:** Host and listener lifecycle state are exclusively Rust-authoritative.

---

# Phase 6 — Streaming packetization, jitter buffer, and scheduler

## Block 14 — Implement bounded streaming host packetization

### 14.1 Define audio source ingestion API

For the initial decoder boundary, implement a bounded bulk API such as:

```rust
pub struct DecodedPcmChunk {
    pub format: AudioFormat,
    pub first_sample_index: u64,
    pub frames: Vec<i16>,
    pub end_of_stream: bool,
}
```

- [x] Do not send one FFI call per frame.
- [x] Bound chunk frames and total queued source data.
- [x] Reject format changes mid-stream unless explicit reconfiguration exists.
- [x] Preserve source errors.

**Desktop Block 19 evidence:** The bounded ingestion API passed Actions run `30599085238` against direct-master input `4c05e5763b1771fc2c7a04690d46b8c76665aa43`. Packetization remains open in 14.2.

### 14.2 Implement streaming packetizer

- [x] 20 ms packet windows or explicit configurable packet duration.
- [x] Session/stream identity.
- [x] sequence and sample index.
- [x] future host presentation timestamp.
- [x] bounded payload size.
- [x] end-of-stream handling.
- [x] no full-track concatenation.

### 14.3 Add backpressure

- [x] Source queue is bounded.
- [x] Full queue returns backpressure/failure; no overwrite.
- [x] Packetizer worker has stop/join semantics.
- [ ] Host UI sees source/packetizer failure.

### 14.4 Add tests

- [x] exact packet boundaries;
- [x] partial final packet policy;
- [x] empty stream;
- [x] format mismatch;
- [x] queue full;
- [x] stream restart and stream-ID change;
- [x] compatibility fixture comparison.

**Acceptance:** Rust creates bounded packets incrementally and Kotlin no longer concatenates the complete decoded audio track for the new path.

**Implementation note (2026-08-01):** `audio::Packetizer` (`rust/silent-disco-core/src/audio/packetizer.rs`) and `audio::StreamingPacketizeHandle` (`rust/silent-disco-core/src/audio/packetizer_worker.rs`) implement the bounded incremental transform and backpressure worker described above, mirroring `StreamingDecodeHandle`'s existing worker/cancellation pattern. 17 new tests in `audio/packetizer_tests.rs`, including a Kotlin-reference fixture comparison (`app/src/test/resources/rust-migration/packetization/pcm_packetization_v1.json`). The one remaining gap — a UniFFI binding and Kotlin/Compose surface so the Android host UI can see packetizer failures — is not yet built; no FFI wrapper exists for the packetizer worker at this point. That wiring, plus feeding the desktop/Android decoder output into this packetizer, is left for Block 25/26.

---

## Block 15 — Implement Rust jitter buffer and playback scheduler

### 15.1 Port packet validation and ordering

- [x] session/stream validation;
- [x] duplicate detection;
- [x] missing sequence accounting;
- [x] bounded reordering window;
- [x] late packet rejection;
- [x] stale stream rejection;
- [x] maximum buffered duration.

**Implementation note (2026-08-01):** `audio::JitterBuffer` (`rust/silent-disco-core/src/audio/jitter_buffer.rs`) implements this as new Rust architecture rather than a direct port: the existing Kotlin `AudioPacketBuffer`/`ListenerPlaybackScheduler` (`app/src/main/java/.../core/audio/{AudioPipeline,PlaybackScheduling}.kt`) has no duplicate detection (a same-sequence insert silently overwrites the sorted map), no bounded reordering window, no stale-stream rejection, and no maximum-buffered-duration bound — all four are genuinely new, not ported. `JitterBuffer::accept` validates session/stream identity, rejects duplicates and already-emitted (late) sequences, enforces a configurable reorder window (default 64 packets, hard ceiling 4096) and buffered-duration bound (default 2000ms, hard ceiling 60000ms), and `pop_in_order`/`missing_sequence_count` expose strict in-order emission and gap accounting for the concealment policy (15.2) and scheduler (15.3) to build on. 13 new tests in `audio/jitter_buffer_tests.rs`. Concealment/silence synthesis, presentation-time pacing, and deterministic clock injection are explicitly out of scope here and remain open in 15.2-15.5.

### 15.2 Port concealment policy

- [x] silence/zero concealment for missing PCM packets;
- [x] monotonic sequence/sample/timestamp progression;
- [x] concealment counter;
- [x] bounded consecutive concealment before desync/rebuffer;
- [x] no use of prior non-silence samples as accidental data leakage.

**Implementation note (2026-08-01):** `audio::ConcealmentPolicy` (`rust/silent-disco-core/src/audio/concealment.rs`) synthesizes a fresh zero-filled `Vec<i16>` per gap (never reuses or caches a prior buffer, verified by a dedicated pointer-identity test) and tracks cumulative/consecutive concealment counters, signalling `ConcealmentOutcome::HardResyncRequired` once a configurable consecutive bound (default 5, hard ceiling 200) is reached. Monotonic sequence/sample/timestamp progression through a gap is the scheduler's responsibility (15.3): it computes the missing slot's sequence, sample index, and presentation time formulaically from the stream's fixed geometry, the same way the packetizer would have, so concealed and real frames are indistinguishable in their metadata progression. 7 new tests in `audio/concealment_tests.rs`.

### 15.3 Implement scheduler

- [x] host-to-local presentation mapping;
- [x] startup buffer target;
- [x] low/high water behavior;
- [x] periodic resync decisions;
- [x] hard-resync threshold;
- [x] render-ring producer pacing;
- [x] explicit stop/reset per stream.

**Implementation note (2026-08-01):** `audio::PlaybackScheduler` (`rust/silent-disco-core/src/audio/scheduler.rs`) owns one `JitterBuffer` and one `ConcealmentPolicy` per stream and exposes `submit_packet`/`poll(local_now_ms)`. `poll` is a `Buffering -> Playing -> (AwaitingRebuffer -> Buffering via rebuffer()) -> Stopped` state machine: it accumulates `startup_buffer_target_ms` (default 400ms) of buffered span before playing, maps each packet's host presentation time to local time via a caller-supplied offset (`host_time - offset`), delivers a real frame or synthesizes a concealed one at each slot's deadline, reports `BufferHealth::{Low,Normal,High}` against configurable low/high water marks (defaults 200ms/700ms), and treats `apply_offset_update` (periodic resync) deltas beyond a configurable hard-resync threshold (default 120ms, matching the existing approved Kotlin `hardResyncThresholdMs`) the same as a concealment-bound breach: both pause the scheduler in `AwaitingRebuffer` until an explicit `rebuffer()` call, which preserves already-buffered packets. Render-ring producer pacing is satisfied by `poll`'s contract (no allocation beyond one `Vec<i16>` per frame, no I/O, no blocking — safe to call at packet-duration cadence); the render ring itself is Block 16. `stop()` is explicit and idempotent; stream restart is a new `PlaybackScheduler` instance, matching the packetizer's own precedent. 18 new tests in `audio/scheduler_tests.rs`.

### 15.4 Add deterministic clock injection

- [x] Unit tests control monotonic time.
- [x] No direct Android clock dependencies.
- [x] Overflow and long-session duration tests.

**Implementation note (2026-08-01):** `PlaybackScheduler::poll` takes `local_now_ms: u64` as a plain parameter rather than reading a real clock, matching the existing `ClockSyncEstimator::decision(&self, now: ...)` precedent in `sync/estimator.rs` — every scheduler test drives time explicitly and there is no Android/system clock dependency anywhere in `audio/scheduler.rs`. One dedicated test exercises multi-year monotonic values and `u64::MAX` through the host-to-local mapping to confirm no panic and correct results at realistic long-session scale.

### 15.5 Add tests

- [x] out-of-order packets;
- [x] duplicate packets;
- [x] missing packet concealment;
- [x] late drops;
- [x] invalid identity;
- [x] underrun transition;
- [x] hard resync;
- [x] bounded memory under hostile packet sequence;
- [x] scheduler shutdown.

**Implementation note (2026-08-01):** Coverage across `jitter_buffer_tests.rs`, `concealment_tests.rs`, and `scheduler_tests.rs` (this block plus 15.1) satisfies every item: out-of-order/duplicate/late-drop/invalid-identity at the jitter-buffer layer, missing-packet concealment and hard-resync/underrun transition at the scheduler layer, a hostile far-future-sequence flood rejected by the bounded reorder window, and explicit `stop()` shutdown semantics.

**Acceptance:** Rust scheduler reproduces or intentionally improves approved Kotlin behavior with executable tests.

---

# Phase 7 — Rust-owned render ring and real-time C ABI

## Block 16 — Implement SPSC render ring

### 16.1 Add render format and capacity validation

- [ ] 48 kHz stereo float32 interleaved internal format.
- [ ] one-second default capacity.
- [ ] 400 ms default target fill.
- [ ] hard minimum/maximum configuration bounds.
- [ ] checked frame/sample/byte arithmetic.

### 16.2 Implement SPSC ownership

Use a proven SPSC implementation or an internally reviewed bounded implementation.

- [ ] Exactly one producer.
- [ ] Exactly one consumer registration.
- [ ] No unread-frame overwrite.
- [ ] No allocation during read/write after initialization.
- [ ] No blocking consumer operation.
- [ ] Cache-line/atomic behavior documented where relevant.

### 16.3 Implement telemetry

Atomic counters:

- [ ] frames produced;
- [ ] frames requested;
- [ ] frames supplied from ring;
- [ ] silence-filled frames;
- [ ] underrun callbacks;
- [ ] ring-full events;
- [ ] callback count;
- [ ] contained panic count.

### 16.4 Add concurrency tests

- [ ] empty read;
- [ ] full write;
- [ ] partial write/read;
- [ ] wraparound;
- [ ] producer faster than consumer;
- [ ] consumer faster than producer;
- [ ] repeated start/stop;
- [ ] thread sanitizer or equivalent host stress where available;
- [ ] no data reordering or duplication.

**Acceptance:** Ring behavior is deterministic, bounded, nonblocking for the consumer, and stress tested.

---

## Block 17 — Implement and harden the real-time C ABI

### 17.1 Check in C header

Create:

```text
rust/silent-disco-ffi/include/silent_disco_audio.h
```

Include:

- [ ] ABI version;
- [ ] opaque engine type;
- [ ] status enum;
- [ ] interleaved float read;
- [ ] available frame query;
- [ ] telemetry queries;
- [ ] documented null/state behavior.

### 17.2 Implement validated handle registry

A safe design may use an opaque token registered in Rust rather than exposing an unvalidated address.

- [ ] Token cannot be guessed into a valid engine.
- [ ] Token generation handles reuse/ABA risk.
- [ ] Acquire/release lifecycle is explicit.
- [ ] Read after release returns invalid state and silence where possible.
- [ ] Registry operations needed by the callback are allocation-free and nonblocking after stream start.

### 17.3 Implement read function

- [ ] Validate pointers and channel count cheaply.
- [ ] Copy frames from ring.
- [ ] Fill missing frames with silence.
- [ ] Set `frames_from_ring` accurately.
- [ ] Return `PARTIAL` for underrun, not success-without-disclosure.
- [ ] Stopping state returns silence and `STOPPING`.

### 17.4 Add panic boundary

- [ ] No Rust unwind crosses C ABI.
- [ ] Contained panic zeroes output.
- [ ] Atomic panic counter increments.
- [ ] Non-real-time fatal notification is scheduled.
- [ ] Test-only panic injection verifies behavior.

### 17.5 Add C ABI tests

- [ ] null engine;
- [ ] null output;
- [ ] zero frames;
- [ ] wrong channel count;
- [ ] partial read/silence fill;
- [ ] full read;
- [ ] stopping;
- [ ] released token;
- [ ] contained panic;
- [ ] ABI version mismatch.

**Acceptance:** C ABI tests prove no panic escapes and no invalid-state call is reported as normal audio success.

---

# Phase 8 — Android Oboe output adapter

## Block 18 — Replace diagnostic native bridge with real Oboe adapter

### 18.1 Implement C++ Oboe adapter

Create or replace native files with:

```text
app/src/main/cpp/OboeOutputAdapter.h
app/src/main/cpp/OboeOutputAdapter.cpp
```

Adapter responsibilities:

- [ ] open low-latency output stream;
- [ ] request float format;
- [ ] request stereo/48 kHz where supported;
- [ ] validate actual stream format;
- [ ] hold engine token for stream lifetime;
- [ ] call only `silent_disco_audio_read_interleaved_f32` in callback;
- [ ] avoid JNI from callback;
- [ ] treat partial read as silence-filled underrun already handled by Rust;
- [ ] return callback continue/stop intentionally;
- [ ] expose non-real-time start/stop/error methods.

Conceptual callback:

```cpp
oboe::DataCallbackResult OboeOutputAdapter::onAudioReady(
        oboe::AudioStream*, void* audioData, int32_t numFrames) {
    uint32_t framesFromRing = 0;
    auto status = silent_disco_audio_read_interleaved_f32(
        engine_,
        static_cast<float*>(audioData),
        static_cast<uint32_t>(numFrames),
        2,
        &framesFromRing);

    if (status == SILENT_DISCO_AUDIO_PANIC_CONTAINED ||
        status == SILENT_DISCO_AUDIO_INVALID_STATE) {
        fatalStatus_.store(status, std::memory_order_relaxed);
    }
    return oboe::DataCallbackResult::Continue;
}
```

Do not copy this blindly; adapt it to the final handle API and Oboe lifecycle.

### 18.2 Add Kotlin platform adapter

- [ ] Kotlin starts/stops native output outside callback.
- [ ] Startup result includes actual format/backend.
- [ ] Failure is returned as `PlatformEvent::AudioOutputFailed`.
- [ ] Route/disconnection errors become explicit events.
- [ ] Kotlin never writes PCM frames.

### 18.3 Enforce shutdown order

- [ ] Rust marks stopping and emits stop effect.
- [ ] Kotlin/C++ stops and closes Oboe.
- [ ] Confirm callback quiescence.
- [ ] Report `AudioOutputStopped`.
- [ ] Rust releases engine token.
- [ ] Tests detect callback-after-release.

### 18.4 Remove production `AudioTrackPlaybackEngine`

- [ ] Oboe/Rust path becomes the only production listener output path.
- [ ] Do not retain `AudioTrackPlaybackEngine` as a silent fallback.
- [ ] A debug-only comparison engine may exist only behind explicit build flag and visible diagnostics.
- [ ] Remove obsolete Oboe diagnostic strings that imply playback is native when it is not.

### 18.5 Add Android tests

- [ ] open/start/stop repeatedly;
- [ ] underrun reports diagnostics;
- [ ] stream disconnect reports failure;
- [ ] background/foreground handling;
- [ ] no callback after release;
- [ ] ABI mismatch fails startup;
- [ ] instrumented playback with generated test tone on physical device.

**Acceptance:** Android production playback is Oboe consuming Rust-owned PCM; no Kotlin frame-write path remains.

---

# Phase 9 — Standard IP transport and platform discovery boundary

## Block 19 — Implement Rust transport runtime

### 19.1 Create bounded transport runtime

- [x] TCP control listener/client.
- [x] UDP synchronization endpoint.
- [x] UDP audio endpoint.
- [x] bounded send/receive queues.
- [x] explicit bind/connect/listen failures.
- [x] worker stop/join.
- [x] byte and packet counters.

### 19.2 Integrate Rust framing

- [x] Control uses protocol-v2 framed messages.
- [x] UDP uses validated fixed headers.
- [x] Oversized packets rejected before allocation.
- [x] Malformed packet counters.
- [x] Session/stream authorization checks before actor events.

### 19.3 Delivery reporting

- [x] Host broadcast reports peer count, success count, failure count.
- [x] Zero peers is not success.
- [x] Partial delivery is explicit.
- [x] Repeated peer failure has bounded, visible removal/recovery policy.
- [x] No log-only socket errors.

### 19.4 Transport tests

- [x] loopback host/listener;
- [x] partial/truncated frames;
- [x] oversized frame;
- [x] wrong version;
- [x] UDP loss/reorder simulation;
- [x] backpressure;
- [x] disconnect during stream;
- [x] shutdown under load;
- [x] multi-listener delivery accounting.

**Acceptance:** Host-independent Rust integration test completes discovery-independent join, sync, and packet exchange over loopback sockets.

---

**Desktop Block 20 evidence:** Actions run `30605377851` passed against direct-master input `09366180e01f65aba04bed2f95d54fb648449fcb`.

## Block 20 — Convert Android networking to platform adapters

### 20.1 Split discovery/establishment from socket transport

Refactor existing Android services so they report facts:

- [ ] BLE session advertisement/discovery.
- [ ] Android NSD/mDNS discovery where implemented.
- [ ] Wi-Fi Direct group/connection establishment.
- [ ] resulting local/remote IP endpoint.
- [ ] permission and platform failures.

They must not own protocol state or authoritative lifecycle state.

### 20.2 Move TCP channel ownership out of Kotlin

- [ ] Rust transport binds/connects after endpoint event.
- [ ] Existing Kotlin `TcpServerChannel`/`TcpClientChannel` production use is removed after parity.
- [ ] No duplicate messages through old and new transports.
- [ ] Migration feature flag is removed after device validation.

### 20.3 Add QR/manual endpoint fallback

- [ ] Core models a manual endpoint request.
- [ ] Android UI can enter or scan endpoint/session information.
- [ ] Endpoint validation occurs in Rust.
- [ ] Invalid or stale information is visible.

### 20.4 Device tests

- [ ] Android-to-Android same-LAN mode.
- [ ] Android Wi-Fi Direct establishment feeding Rust sockets.
- [ ] BLE discovery failure.
- [ ] endpoint connection failure.
- [ ] disconnect/reconnect.
- [ ] partial delivery with multiple listeners.

**Acceptance:** Android-specific APIs establish/discover networks, while Rust owns protocol sockets and delivery semantics.

---

# Phase 10 — Complete Android presentation separation

## Block 21 — Reduce `MainViewModel` to presentation/platform coordination

### 21.1 Remove duplicate domain fields/jobs

Remove Kotlin ownership of:

- [ ] current session/stream domain IDs where Rust snapshot suffices;
- [ ] sync controller;
- [ ] listener scheduler;
- [ ] packet list and pending transport packets;
- [ ] host stream packetization job;
- [ ] listener playback frame-write job;
- [ ] periodic domain resync job;
- [ ] domain diagnostics store;
- [ ] domain metrics store;
- [ ] domain settings persistence.

Retain only platform lifecycle/effect jobs with explicit ownership.

### 21.2 Make Compose render Rust-backed state

- [ ] Map `CoreSnapshot` to localized presentation models.
- [ ] Labels remain Kotlin resources/presentation functions.
- [ ] Button enablement derives from Rust-provided capabilities or legal actions.
- [ ] UI does not reconstruct lifecycle legality independently.
- [ ] Error details can be exported without exposing secrets.

### 21.3 Remove obsolete production helpers

- [ ] Delete copied validators/state transition helpers now owned by Rust.
- [ ] Delete obsolete Kotlin packet/sync/jitter code after equivalent Rust tests.
- [ ] Keep platform-specific decoder/discovery/audio adapters only.
- [ ] Search for dangerous old fallback calls and remove them.

Suggested checks:

```bash
grep -R "AudioTrackPlaybackEngine\|ListenerPlaybackScheduler\|PcmPacketizer\|ClockSyncEstimator" app/src/main/java -n
grep -R "getSharedPreferences\|SQLiteDatabase\|Room\.databaseBuilder" app/src/main/java -n
grep -R "runCatching.*logger\|onFailure.*logger" app/src/main/java -n
```

Every remaining match must be justified as platform-specific or removed.

### 21.4 Update Android tests

- [ ] UI tests submit commands through facade.
- [ ] Effect runner tests report real completion/failure facts.
- [ ] No tests validate copied Rust rules in Kotlin.
- [ ] Existing FIX3/FIX4/FIX5 failure visibility remains covered.

**Acceptance:** Kotlin/Compose is a native presentation and platform adapter shell; Rust is the only domain/data engine.

---

# Phase 11 — iOS binding and audio smoke target

## Block 22 — Package Rust for Apple platforms

### 22.1 Build Apple libraries

- [ ] Build simulator and physical-device architectures.
- [ ] Package XCFramework or equivalent.
- [ ] Include UniFFI Swift bindings.
- [ ] Include C header/module map for audio ABI.
- [ ] No machine-specific paths.
- [ ] Reproducible generation command.

### 22.2 Add minimal iOS smoke target

The target must:

- [ ] create application-support database path;
- [ ] open Rust core;
- [ ] create schema;
- [ ] submit command;
- [ ] receive snapshot notification;
- [ ] query versions;
- [ ] acquire/release audio engine token;
- [ ] link and call C ABI with a test buffer;
- [ ] shut down cleanly.

A full SwiftUI product UI is not required in this block.

### 22.3 Add minimal Apple audio source test

- [ ] Configure `AVAudioSession` in test/sample code.
- [ ] Create `AVAudioSourceNode` or selected output unit.
- [ ] Pull test tone/silence through Rust C ABI.
- [ ] Avoid allocation/UniFFI in render block.
- [ ] Stop callback before token release.
- [ ] Report route/interruption failures visibly.

### 22.4 Add CI/local validation instructions

- [ ] Build simulator smoke target.
- [ ] Document physical-device command.
- [ ] Verify generated bindings are current.

**Acceptance:** The same Rust core, database, commands/snapshots, and audio ABI link from Swift.

---

# Phase 12 — Decoder boundary decision

## Block 23 — Select and implement the long-term audio decoder boundary

Do not start this block until Rust packetization, scheduler, and Android Oboe output are stable.

### 23.1 Measure current platform-decoder bridge

- [ ] Measure PCM copy overhead.
- [ ] Measure memory use.
- [ ] Measure startup time.
- [ ] Test representative MP3/AAC/WAV/FLAC files currently supported by Android.
- [ ] Identify iOS file-access constraints.

### 23.2 Choose one path

#### Path A — Platform decoding

- [ ] Keep Android/iOS decoder adapters.
- [ ] Use bounded bulk PCM transfer to Rust.
- [ ] Document supported formats by platform.
- [ ] Ensure decoder failures map to core errors.

#### Path B — Rust decoding

- [ ] Platform copies security-scoped/content-URI source to stable app-private path.
- [ ] Rust streams decode from file.
- [ ] Pin and test decoder library.
- [ ] Bound decoder buffers.
- [ ] Match required formats on Android/iOS.
- [ ] Handle cancellation and corrupted input.

### 23.3 Remove the unselected temporary path

- [ ] Do not maintain two silent decoder paths.
- [ ] A debug comparison path must be explicit and visibly selected.

**Desktop Block 18 coordination:** Path B (shared Rust decoding) is selected. The desktop Symphonia spike and decision record are complete, but shared Block 23 remains open until Android bridge overhead, mobile physical-device format parity, iOS file-access constraints, and removal of the temporary platform decoder path are recorded. No hidden fallback is introduced during that migration.

**Acceptance:** One documented production decoder ownership model exists with performance/device evidence.

---

# Phase 13 — Hardening, fuzzing, and lifecycle integrity

## Block 24 — FFI and concurrency hardening

- [ ] Audit every `unsafe` block with `// SAFETY:` invariant.
- [ ] Verify no core unsafe code outside FFI crate.
- [ ] Test handle reuse and release races.
- [ ] Test observer removal during notification.
- [ ] Test shutdown during database write.
- [ ] Test shutdown during network load.
- [ ] Test stop while audio callback active.
- [ ] Test repeated initialization failure/retry.
- [ ] Add loom/model tests where practical for ring/handle state.
- [ ] Add sanitizer builds where supported.
- [ ] Ensure no Rust panic crosses UniFFI or C ABI.

**Acceptance:** Concurrency and FFI race tests pass repeatedly without leak, deadlock, use-after-free, or silent loss.

---

## Block 25 — Protocol and storage fuzz/property testing

- [ ] Fuzz control-frame parser.
- [ ] Fuzz audio/sync datagram parser.
- [ ] Property-test encode/decode round trips.
- [ ] Test bounded allocation under hostile lengths.
- [ ] Fuzz migration metadata/checksum parsing where applicable.
- [ ] Test corrupted rows and invalid enum values.
- [ ] Test database busy/full/read-only conditions.
- [ ] Test disk-write failure through injectable storage boundary where feasible.
- [ ] Confirm database failure does not switch to in-memory mode.

**Acceptance:** No parser panic, uncontrolled allocation, or destructive database fallback is found in the configured runs.

---

## Block 26 — Diagnostics and observability completion

### 26.1 Core diagnostics

Expose:

- [ ] actor queue depth/overflow count;
- [ ] effect queue depth;
- [ ] transport bytes/packets/failures;
- [ ] malformed/stale packet counts;
- [ ] sync RTT/offset/confidence/drift;
- [ ] jitter depth/loss/late/concealment;
- [ ] render ring fill/high/low/full;
- [ ] callback frames/underruns/panics;
- [ ] database operation/error/migration version;
- [ ] binding/ABI versions;
- [ ] worker lifecycle states.

### 26.2 Persistence policy

- [ ] High-frequency counters remain atomic/in-memory.
- [ ] Snapshot summaries persist at controlled interval/session end.
- [ ] No per-frame/per-packet SQLite writes.
- [ ] Diagnostic write failure is visible but cannot block audio.

### 26.3 Export

- [ ] Rust creates structured redacted export.
- [ ] Kotlin/Swift share it through platform UI.
- [ ] No private keys, invite codes, or secret values.
- [ ] Export records exact versions and tuning.

**Acceptance:** A single export can explain core, transport, sync, ring, audio callback, and database state without relying on logcat alone.

---

# Phase 14 — Physical-device validation

## Block 27 — Android single-device lifecycle validation

On at least one physical Android device:

- [ ] cold start and database open;
- [ ] legacy import if applicable;
- [ ] host create/end repeated 20 times;
- [ ] Oboe start/stop repeated 50 times;
- [ ] background/foreground during idle, discovery, and playback;
- [ ] audio route change;
- [ ] simulated ring underrun;
- [ ] database busy/failure injection;
- [ ] native library load failure behavior in test build;
- [ ] no callback after engine release;
- [ ] no leaked workers after shutdown.

Record results in `memory.md`.

**Acceptance:** No crash, deadlock, silent fallback, or stale authoritative state.

---

## Block 28 — Two-device Android host/listener validation

Use two physical Android devices.

- [ ] discovery;
- [ ] connection establishment;
- [ ] join request;
- [ ] approval and rejection;
- [ ] initial sync;
- [ ] start playback;
- [ ] pause/resume or pause/start policy;
- [ ] stop;
- [ ] end session;
- [ ] listener disconnect/reconnect;
- [ ] host transport failure;
- [ ] listener transport failure;
- [ ] wrong invite code;
- [ ] zero-listener start disclosure;
- [ ] partial delivery simulation where possible;
- [ ] multi-minute playback diagnostic export.

Measure:

- [ ] startup latency;
- [ ] sync offset/RTT;
- [ ] ring target fill;
- [ ] underruns;
- [ ] packet loss/late drops;
- [ ] callback timing;
- [ ] memory growth.

**Acceptance:** Rust-core Android mode performs the complete current PoC workflow on real devices.

---

## Block 29 — Multi-listener capacity and stability validation

With available devices:

- [ ] test 2 listeners;
- [ ] test 3 listeners;
- [ ] test 4 listeners;
- [ ] record host CPU/memory/network;
- [ ] record per-listener packet loss and underruns;
- [ ] identify practical listener ceiling for tested host;
- [ ] verify partial delivery reporting when one listener fails;
- [ ] run at least one extended session;
- [ ] document device models and network mode.

Do not claim general capacity beyond measured devices.

**Acceptance:** Capacity findings and major failure modes are documented with diagnostic evidence.

---

# Phase 15 — Cleanup and completion

## Block 30 — Remove migration flags and obsolete code

- [ ] Remove temporary Kotlin/Rust feature-selection flag after Rust path passes device validation.
- [ ] Remove obsolete Kotlin protocol codec.
- [ ] Remove obsolete Kotlin state machines.
- [ ] Remove obsolete Kotlin sync/jitter/packetizer code.
- [ ] Remove `AudioTrackPlaybackEngine` production code and tests that only apply to it.
- [ ] Remove old TCP channel production code after Rust transport validation.
- [ ] Remove migrated `SharedPreferences` domain keys/import code only after supported migration window is decided.
- [ ] Remove diagnostic-only Oboe JNI functions.
- [ ] Remove dead dependencies and imports.
- [ ] Confirm no generated native artifacts are stale.

Search for leftovers:

```bash
grep -R "TODO.*Rust\|temporary.*Rust\|legacy.*fallback\|AudioTrackPlaybackEngine" app rust -n
grep -R "ClockSyncEstimator\|ListenerPlaybackScheduler\|PcmPacketizer" app/src/main/java -n
grep -R "getSharedPreferences" app/src/main/java -n
grep -R "TODO\|FIXME" rust app/src/main -n
```

Every remaining result must be resolved or explicitly justified in `memory.md`.

---

## Block 31 — Final quality gate

### Rust

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --workspace --all-features` passes.
- [ ] fuzz/property suites complete configured runs.
- [ ] no undocumented unsafe block.

### Android

- [ ] `./gradlew test` passes.
- [ ] `./gradlew lintDebug` passes.
- [ ] connected instrumented tests pass on physical device.
- [ ] debug and release native libraries package correctly.
- [ ] no duplicate ABI/library collision.

### Apple smoke target

- [ ] simulator build passes.
- [ ] physical-device build command is documented and tested if hardware is available.
- [ ] database/core/audio ABI smoke test passes.

### Architecture review

- [ ] Rust is sole domain state owner.
- [ ] Rust is sole SQLite owner.
- [ ] Rust owns render ring and scheduling.
- [ ] Oboe consumes only C ABI in callback.
- [ ] UniFFI is absent from callback path.
- [ ] database is absent from timing-critical paths.
- [ ] all queues are bounded.
- [ ] shutdown joins every worker.
- [ ] no automatic destructive database recovery.
- [ ] no hidden playback/transport/database fallback.
- [ ] no success before completion.

### Documentation

- [ ] Update README with architecture, build, test, and device requirements.
- [ ] Update `CLAUDE.md` with Rust commands and Ralph-loop boundaries.
- [ ] Update `memory.md` with final migration record, versions, tests, devices, and remaining limitations.
- [ ] Mark every completed item in this TODO.

**Final acceptance:** The Android app runs on the shared Rust core, all SQLite access and authoritative logic are Rust-owned, Android audio uses Oboe over the Rust ring-buffer C ABI, Swift can link and smoke-test the same core, and no dangerous silent fallback remains.