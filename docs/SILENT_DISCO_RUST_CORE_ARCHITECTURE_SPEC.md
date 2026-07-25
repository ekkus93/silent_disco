# Silent Disco Shared Rust Core Architecture Specification

**Status:** Proposed implementation specification  
**Date:** 2026-07-25  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Companion TODO:** `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`

---

## 1. Purpose

This document specifies a major architectural migration of the Silent Disco Android proof of concept into a cross-platform system with:

- a shared Rust core used by Android and iOS;
- native Android presentation implemented with Kotlin and Jetpack Compose;
- native iOS presentation implemented with Swift and SwiftUI;
- Rust-owned domain logic, protocol handling, synchronization, audio scheduling, diagnostics, and SQLite persistence;
- a Rust-owned, bounded, single-producer/single-consumer audio render ring buffer;
- native platform audio callbacks that consume the Rust ring buffer through a narrow C ABI;
- UniFFI bindings for ordinary control-plane commands, events, effects, snapshots, settings, and errors.

The goal is not to wrap the existing Kotlin application in Rust. The goal is to establish one authoritative, platform-independent domain engine while retaining native platform integrations and native user interfaces.

This migration must preserve the working Android application throughout implementation. It must be executed incrementally, with tests and compatibility checks at each boundary. A destructive rewrite in which the existing application stops building for an extended period is not acceptable.

---

## 2. Binding architectural decisions

The following decisions are approved and are requirements of this migration.

1. **Rust is the sole owner of authoritative domain state.**
2. **Rust is the sole owner of SQLite access, schema creation, migrations, SQL statements, transactions, and row-to-domain conversion.**
3. **Rust owns protocol serialization and deserialization.** Kotlin and Swift exchange typed commands or opaque bytes with Rust; they do not implement a second wire codec.
4. **Rust owns clock synchronization, packetization, jitter buffering, packet validation, concealment decisions, playback scheduling, and render-buffer production.**
5. **Rust owns the render PCM ring buffer.** The ring is bounded, preallocated, single-producer/single-consumer, and used without allocation or blocking on the audio callback path.
6. **Android audio output uses Oboe.** The Oboe callback pulls PCM frames from Rust through the real-time C ABI.
7. **iOS audio output uses an Apple native render callback, initially `AVAudioSourceNode` or an equivalent Audio Unit path.** The callback pulls PCM frames through the same C ABI contract.
8. **UniFFI is used for the control plane only.** UniFFI must not be called from a real-time audio callback.
9. **Kotlin and Swift own presentation and platform adapters.** Platform adapters include permissions, lifecycle, file selection, secure storage, discovery APIs, and platform-specific network-establishment APIs.
10. **Rust may own standard TCP/UDP sockets after a platform adapter supplies a usable local endpoint or network context.** Platform discovery and permission APIs remain native.
11. **Private secrets remain protected by Android Keystore and iOS Keychain.** SQLite may store identifiers, public keys, trust metadata, and references to protected secrets, but not unprotected private keys.
12. **No silent fallback, fake success, or automatic destructive recovery is permitted.** Failures must be represented as structured errors and surfaced to diagnostics and presentation state.
13. **The Android application must build and its existing automated tests must pass after every Ralph-loop block unless a block explicitly introduces a temporary compile gate that is completed within the same commit.**
14. **The full iOS user interface is not part of the initial Android migration.** The first iOS deliverable is generated Swift bindings plus a minimal smoke-test target proving the core can initialize, open its database, process commands, and expose the audio ABI.

---

## 3. Current architecture and migration motivation

The current application is primarily Kotlin and Jetpack Compose. The existing `MainViewModel` coordinates:

- Android application and lifecycle dependencies;
- permissions;
- BLE discovery and advertising;
- Wi-Fi Direct setup;
- TCP transport channels;
- audio file description and decoding;
- packetization;
- host and listener state transitions;
- synchronization;
- jitter buffering and playback scheduling;
- audio output;
- diagnostics;
- `SharedPreferences` persistence;
- user-facing status and error messages.

This concentration makes the Android proof of concept workable, but it creates several problems for a shared Android/iOS product:

- domain behavior is coupled to Android types such as `Application`, `Context`, `Uri`, `SystemClock`, `WifiP2pManager`, and Kotlin coroutine scopes;
- state may be changed from UI actions, transport callbacks, audio jobs, and diagnostics paths rather than through one authoritative reducer or actor;
- protocol and state behavior would need to be reimplemented independently in Swift;
- the existing native Oboe code is only a minimal JNI boundary and is not the actual render engine;
- persistence is Android-specific;
- testing platform-independent behavior requires Android/JVM scaffolding.

The migration must separate the system into a shared Rust domain/runtime and thin platform shells without losing the failure visibility and correctness hardening already completed in the Kotlin application.

---

## 4. Target system architecture

```text
┌──────────────────────────────────────────────────────────────┐
│ Android application                 iOS application          │
│ Kotlin + Compose                    Swift + SwiftUI           │
│                                                              │
│ • Screens/navigation               • Screens/navigation      │
│ • Permission prompts               • Permission prompts      │
│ • Lifecycle/background policy      • Lifecycle/background    │
│ • File/document picker             • File/document picker    │
│ • BLE/Wi-Fi discovery adapters     • Bonjour/discovery       │
│ • Keystore adapter                 • Keychain adapter         │
│ • Oboe render callback             • AVAudio render callback  │
└───────────────────────┬───────────────────────┬──────────────┘
                        │                       │
               UniFFI control API        real-time C ABI
                        │                       │
┌───────────────────────▼───────────────────────▼──────────────┐
│                         Rust core                            │
│                                                             │
│ • Authoritative actor/state machines                        │
│ • Command/event/effect processing                           │
│ • Protocol framing and validation                           │
│ • Standard IP transport runtime                             │
│ • Clock synchronization                                     │
│ • Packetization and jitter buffer                           │
│ • Concealment and playback scheduler                        │
│ • Rust-owned SPSC render ring                               │
│ • Diagnostics and tuning validation                         │
│ • SQLite worker, schema, migrations, repositories           │
└─────────────────────────────────────────────────────────────┘
```

The Rust core must compile as a normal host library for unit and integration tests, as an Android native library for supported Android ABIs, and as an Apple library packaged for Swift consumption.

---

## 5. Repository layout

Add the following top-level Rust workspace without moving the Android application during the first migration blocks:

```text
rust/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── silent-disco-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── command.rs
│       ├── event.rs
│       ├── effect.rs
│       ├── error.rs
│       ├── snapshot.rs
│       ├── runtime.rs
│       ├── domain/
│       ├── protocol/
│       ├── session/
│       ├── sync/
│       ├── transport/
│       ├── diagnostics/
│       ├── storage/
│       │   ├── mod.rs
│       │   ├── database.rs
│       │   ├── migrations.rs
│       │   ├── settings_repository.rs
│       │   ├── trusted_device_repository.rs
│       │   ├── session_repository.rs
│       │   └── diagnostics_repository.rs
│       └── audio/
│           ├── mod.rs
│           ├── format.rs
│           ├── packetizer.rs
│           ├── jitter_buffer.rs
│           ├── scheduler.rs
│           ├── concealment.rs
│           ├── render_ring.rs
│           ├── telemetry.rs
│           └── engine.rs
├── silent-disco-ffi/
│   ├── Cargo.toml
│   ├── build.rs
│   ├── uniffi.toml
│   ├── include/
│   │   └── silent_disco_audio.h
│   └── src/
│       ├── lib.rs
│       ├── uniffi_api.rs
│       ├── realtime_audio_abi.rs
│       └── panic_boundary.rs
└── silent-disco-test-support/
    ├── Cargo.toml
    └── src/lib.rs
```

Android-specific adapters remain under the Android application module, for example:

```text
app/src/main/java/com/ekkus/silentdisco/platform/
├── CoreFacade.kt
├── CoreObserverAdapter.kt
├── AndroidPlatformEffectRunner.kt
├── AndroidPermissionAdapter.kt
├── AndroidDiscoveryAdapter.kt
├── AndroidSecureStoreAdapter.kt
└── AndroidDatabasePathProvider.kt

app/src/main/cpp/
├── CMakeLists.txt
├── OboeOutputAdapter.cpp
├── OboeOutputAdapter.h
└── RustAudioAbi.h
```

An iOS shell may later be added under `ios/`, but the initial migration must not require the full iOS app to exist.

---

## 6. Dependency direction

Dependency direction is mandatory:

```text
Compose/SwiftUI → platform ViewModel/store → platform adapters → Rust FFI → Rust core
```

The Rust core must not import or refer to:

- Android classes or JNI types;
- Apple frameworks or Objective-C/Swift types;
- Compose or SwiftUI concepts;
- localized display strings;
- platform permission names;
- platform file-picker objects;
- Android `Uri` or Apple security-scoped URL objects.

The Rust core may define semantic concepts such as `PermissionCapability::NearbyDiscovery`, but the platform maps those concepts to platform APIs and user prompts.

Platform UI code must not contain independent copies of:

- legal lifecycle transitions;
- join approval policy;
- packet validation rules;
- synchronization formulas;
- jitter-buffer decisions;
- delivery-success classification;
- database migrations;
- trusted-device policy.

Temporary duplicate implementations are allowed only during a migration block with explicit parity tests and a documented removal block.

---

## 7. Authoritative core runtime

### 7.1 Actor model

The Rust core must use one serialized authoritative domain actor. All state-changing commands and external events enter this actor in order. The actor owns the mutable domain model and produces effects and snapshots.

Inputs:

- `CoreCommand`: user or presentation intent;
- `PlatformEvent`: result of a platform operation;
- `TransportEvent`: data or state from the Rust transport runtime;
- `AudioEvent`: non-real-time notifications from the scheduler/output telemetry;
- `StorageEvent`: database results.

Outputs:

- `PlatformEffect`: request for a platform adapter action;
- `CoreNotification::Snapshot`: authoritative state for presentation;
- `CoreNotification::Error`: structured failure requiring visibility;
- `CoreNotification::Diagnostic`: structured informational event.

The actor must not block on database work, networking, or platform callbacks. Long-running operations are represented by effects or worker commands and later completion events.

### 7.2 Commands

Initial command surface:

```rust
pub enum CoreCommand {
    SelectRole { role: AppRole },
    UpdateHostDraft(HostDraftPatch),
    CreateHostSession,
    EndHostSession,
    StartDiscovery,
    StopDiscovery,
    SelectSession { session_id: SessionId },
    SubmitJoin { invite_code: Option<String> },
    CancelJoin,
    ApproveJoin { request_id: RequestId },
    RejectJoin { request_id: RequestId },
    RemoveListener { listener_id: DeviceId },
    StartPlayback { source: AudioSourceDescriptor },
    PausePlayback,
    StopPlayback,
    SetLocalVolume { linear_gain: f32 },
    RequestResync,
    RetryRecoverableFailure,
    UpdateTuning(TuningPatch),
    ExportDiagnostics,
    Shutdown,
}
```

Commands must be validated in Rust. Invalid commands return a structured rejection and must not mutate state.

### 7.3 Platform events

Examples:

```rust
pub enum PlatformEvent {
    CapabilityStateChanged(CapabilitySnapshot),
    DiscoveryStarted,
    DiscoveryStopped,
    SessionDiscovered(SessionAdvertisement),
    SessionExpired { session_id: SessionId },
    DiscoveryFailed(CoreError),
    NetworkEndpointReady(NetworkEndpoint),
    NetworkEstablishmentFailed(CoreError),
    AudioSourcePrepared(AudioSourceInfo),
    AudioSourcePreparationFailed(CoreError),
    AudioOutputStarted(AudioOutputInfo),
    AudioOutputStopped,
    AudioOutputFailed(CoreError),
    SecureValueLoaded { key: SecretKeyId, value: Vec<u8> },
    SecureStoreFailed(CoreError),
    AppEnteredForeground,
    AppEnteredBackground,
}
```

Platform events describe facts. They must not instruct Rust to pretend that an operation succeeded.

### 7.4 Effects

Examples:

```rust
pub enum PlatformEffect {
    RequestCapabilities(Vec<PermissionCapability>),
    StartAdvertising(SessionAdvertisement),
    StopAdvertising,
    StartDiscovery(DiscoveryRequest),
    StopDiscovery,
    EstablishNetwork(NetworkEstablishmentRequest),
    ReleaseNetwork,
    PrepareAudioSource(AudioSourceDescriptor),
    StartAudioOutput(AudioOutputRequest),
    StopAudioOutput,
    LoadSecureValue(SecretKeyId),
    StoreSecureValue { key: SecretKeyId, value: Vec<u8> },
    DeleteSecureValue(SecretKeyId),
    ShareDiagnostics(DiagnosticsExport),
}
```

Effects require a unique operation ID. Completion events must include the same ID so stale or duplicate results can be rejected deterministically.

### 7.5 Snapshots

`CoreSnapshot` is the only authoritative presentation model. It must be immutable at the FFI boundary and include:

- selected role;
- host draft and validation state;
- host lifecycle;
- listener lifecycle;
- discovery state and discovered sessions;
- pending join requests;
- approved and connected listeners;
- playback state;
- synchronization state and confidence;
- tuning settings;
- host/listener diagnostics summaries;
- current recoverable action;
- last structured error;
- monotonically increasing snapshot revision.

Kotlin and Swift may derive localized labels and display formatting, but must not infer legal transitions that disagree with the snapshot.

---

## 8. UniFFI control-plane contract

### 8.1 Core handle

Expose one long-lived `CoreHandle` through UniFFI.

```rust
pub struct CoreConfig {
    pub database_path: String,
    pub files_directory: String,
    pub cache_directory: String,
    pub device_id: String,
    pub platform: PlatformKind,
}

pub trait CoreObserver: Send + Sync {
    fn on_notification(&self, notification: CoreNotification);
}

pub struct CoreHandle { /* opaque */ }

impl CoreHandle {
    pub fn open(config: CoreConfig, observer: Arc<dyn CoreObserver>)
        -> Result<Arc<Self>, CoreError>;

    pub fn submit_command(&self, command: CoreCommand)
        -> Result<CommandReceipt, CoreError>;

    pub fn submit_platform_event(&self, event: PlatformEvent)
        -> Result<(), CoreError>;

    pub fn current_snapshot(&self) -> CoreSnapshot;

    pub fn audio_engine_token(&self) -> Result<AudioEngineToken, CoreError>;

    pub fn shutdown(&self) -> Result<(), CoreError>;
}
```

The exact UniFFI syntax may differ, but the semantics are required.

### 8.2 Observer rules

- Observer callbacks occur on a Rust-controlled non-real-time notification thread.
- Kotlin/Swift adapters must marshal UI updates to their main thread.
- Observer callbacks must never execute while the core actor holds a state lock.
- Effects and errors must not be silently dropped.
- Snapshot notifications may be coalesced only if revision order remains monotonic and the latest snapshot is guaranteed to be delivered.
- Observer exceptions or panics must be contained and converted into a visible bridge failure.

### 8.3 Versioning

Expose:

- core semantic version;
- UniFFI API version;
- real-time C ABI version;
- database schema version;
- wire protocol version.

Android and iOS startup must reject incompatible combinations explicitly.

---

## 9. Real-time audio architecture

### 9.1 Internal render format

The Rust render ring uses:

- sample rate: 48,000 Hz;
- channels: 2;
- sample type: IEEE 32-bit floating point;
- logical layout: interleaved stereo frames;
- normalized sample range: `[-1.0, 1.0]`;
- default ring capacity: 48,000 frames, equal to one second;
- initial target fill: 19,200 frames, equal to 400 ms;
- low-water threshold: 9,600 frames, equal to 200 ms;
- high-water threshold: 33,600 frames, equal to 700 ms.

Values must be configurable through validated tuning settings, but hard safety bounds are enforced in Rust.

### 9.2 Separate packet and render buffers

The system must contain two distinct buffering stages.

1. **Jitter buffer**
   - stores validated protocol packets;
   - reorders by sequence/timeline;
   - tracks missing and late packets;
   - makes concealment and resynchronization decisions;
   - may use ordinary synchronization because it is not called by the audio callback.

2. **Render ring buffer**
   - stores only ordered render-ready PCM frames;
   - is preallocated;
   - has exactly one producer and one consumer;
   - contains no protocol objects;
   - never grows;
   - never blocks the consumer.

### 9.3 Producer and consumer

- Producer: one Rust playback-scheduling worker.
- Consumer: one native platform audio callback.

The producer must never write through multiple clones that violate SPSC assumptions. If output must be restarted, ownership is transferred through a controlled lifecycle state rather than creating an additional consumer.

### 9.4 Real-time C ABI

Check in a stable header at `rust/silent-disco-ffi/include/silent_disco_audio.h`.

Required conceptual API:

```c
#ifndef SILENT_DISCO_AUDIO_H
#define SILENT_DISCO_AUDIO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SilentDiscoAudioEngine SilentDiscoAudioEngine;

typedef enum SilentDiscoAudioStatus {
    SILENT_DISCO_AUDIO_OK = 0,
    SILENT_DISCO_AUDIO_PARTIAL = 1,
    SILENT_DISCO_AUDIO_STOPPING = 2,
    SILENT_DISCO_AUDIO_INVALID_ARGUMENT = -1,
    SILENT_DISCO_AUDIO_INVALID_STATE = -2,
    SILENT_DISCO_AUDIO_PANIC_CONTAINED = -3
} SilentDiscoAudioStatus;

uint32_t silent_disco_audio_abi_version(void);

SilentDiscoAudioStatus silent_disco_audio_read_interleaved_f32(
    SilentDiscoAudioEngine *engine,
    float *output,
    uint32_t requested_frames,
    uint32_t output_channels,
    uint32_t *frames_from_ring
);

uint32_t silent_disco_audio_available_frames(
    const SilentDiscoAudioEngine *engine
);

uint64_t silent_disco_audio_underrun_count(
    const SilentDiscoAudioEngine *engine
);

uint64_t silent_disco_audio_frames_rendered(
    const SilentDiscoAudioEngine *engine
);

#ifdef __cplusplus
}
#endif

#endif
```

The exact opaque-handle acquisition method may be integrated with UniFFI through a token-to-pointer registry, but raw pointers must never be represented as normal Kotlin or Swift integers without lifetime validation.

### 9.5 Callback behavior

`silent_disco_audio_read_interleaved_f32` must:

- validate only inexpensive preconditions;
- read up to `requested_frames` from the ring;
- copy available interleaved stereo frames;
- adapt mono or stereo output only through preconfigured, allocation-free logic;
- fill any missing output with zeroes;
- update atomic counters;
- return a status without throwing or unwinding;
- complete in bounded time.

It must not:

- allocate memory;
- resize a collection;
- wait on a mutex, condition variable, channel, future, or database;
- perform file I/O or network I/O;
- deserialize protocol messages;
- call UniFFI;
- call Kotlin or Swift;
- write logs;
- format strings;
- run SQLite;
- initiate recovery operations.

### 9.6 Panic and exception containment

Rust must not unwind across the C ABI. Each exported C function must use a panic boundary. On a contained panic during render:

- zero the requested output buffer;
- increment an atomic panic counter;
- return `SILENT_DISCO_AUDIO_PANIC_CONTAINED`;
- schedule a non-real-time fatal audio error for the core actor;
- never continue claiming normal playback.

The platform callback must treat a fatal status as silence for the current callback and request output shutdown through a non-real-time path.

### 9.7 Android Oboe adapter

The Android native adapter must:

- open a low-latency Oboe output stream;
- request float output where supported;
- use a data callback;
- retain a validated Rust engine handle for the stream lifetime;
- call only the real-time C ABI in `onAudioReady`;
- avoid JNI calls from the callback;
- handle stream disconnection and route changes explicitly;
- report startup, stop, and failure events through Kotlin to Rust outside the callback;
- stop and join the stream before releasing the Rust engine token.

The existing diagnostic-only `native-lib.cpp` must be replaced or retired once the Oboe adapter is functional. There must not be two competing audio output engines in production.

### 9.8 iOS adapter

The first iOS adapter must:

- configure `AVAudioSession` outside the render callback;
- create an `AVAudioSourceNode` or equivalent render source;
- call only the real-time C ABI from the render block;
- translate interleaved Rust frames to the platform buffer layout without allocation;
- report route changes and interruptions through normal platform events;
- stop the graph and guarantee the render block is quiescent before Rust engine destruction.

### 9.9 Shutdown order

Required output shutdown sequence:

```text
1. Rust actor marks audio engine STOPPING.
2. Producer stops adding new render frames.
3. Rust emits StopAudioOutput effect.
4. Platform stops Oboe/AVAudioEngine.
5. Platform confirms no callback is executing.
6. Platform reports AudioOutputStopped.
7. Rust releases the consumer registration and ring resources.
8. Core may destroy the audio engine.
```

Timeouts are explicit failures. The core must not free callback-visible memory merely because a timeout elapsed.

---

## 10. Audio scheduling and synchronization

Rust owns all calculations currently represented by Kotlin synchronization and playback helpers.

Required responsibilities:

- monotonic timestamp representation;
- four-timestamp offset and RTT estimation;
- outlier rejection;
- confidence classification;
- drift detection;
- periodic synchronization policy;
- translation from host presentation time to listener-local time;
- packet validation by session and stream ID;
- missing sequence detection;
- late-packet rejection;
- concealment frame generation;
- hard resynchronization decisions;
- render-ring target-fill control;
- underrun recovery state.

All clocks are monotonic durations relative to platform-provided monotonic epochs. Wall-clock time must never be used for packet playback deadlines.

Define explicit newtypes rather than untyped integers where practical:

```rust
pub struct MonotonicMillis(pub u64);
pub struct HostMonotonicMillis(pub u64);
pub struct LocalMonotonicMillis(pub u64);
pub struct SampleIndex(pub u64);
pub struct PacketSequence(pub u64);
```

The platform provides monotonic time through an initialization calibration or small platform adapter call. Repeated high-frequency FFI clock calls from the render callback are prohibited.

---

## 11. Audio source and decoding boundary

For the initial migration:

- Android and iOS own file selection and permission handling;
- the selected source is copied or made available at an application-private, stable file path;
- the platform reports an `AudioSourceDescriptor` containing a stable path, display metadata, and content type;
- the current platform decoder may remain temporarily during the first Rust-core blocks;
- decoded PCM must be handed to Rust through a bounded bulk-transfer API, not one UniFFI call per audio frame;
- the long-term target is Rust-owned streaming decode if format support and platform tests justify it.

A required decision gate later in the TODO will choose between:

1. platform decoder feeding Rust PCM through a bulk C/UniFFI bridge; or
2. Rust decoder reading the copied application-private source file.

Until that gate, no new feature may depend on a particular decoder implementation.

The system must not concatenate an entire decoded track into one giant byte array as the long-term architecture. Host packetization must be streaming and bounded.

---

## 12. Protocol and standard IP transport

### 12.1 Discovery versus transport

Platform adapters own discovery and network-establishment APIs:

- Android may use BLE discovery, Android NSD/mDNS, and Wi-Fi Direct establishment;
- iOS may use Bonjour/Network.framework discovery and Apple-supported local-network establishment;
- QR/manual endpoint entry is a required fallback for testability.

After a usable IP endpoint exists, the Rust transport runtime should own standard socket behavior where platform constraints permit.

### 12.2 Cross-platform baseline

The first Android/iOS interoperable mode is devices on the same IP network.

Baseline:

- discovery: mDNS/DNS-SD through platform adapters;
- control: reliable, framed TCP;
- synchronization: UDP request/response with correlation IDs, or a dedicated low-latency socket selected during implementation;
- audio: UDP datagrams with sequence numbers, stream IDs, presentation timestamps, payload length, and integrity validation;
- fallback discovery: QR/manual host endpoint.

Android Wi-Fi Direct may remain an Android-to-Android network-establishment adapter. It must not be treated as the cross-platform protocol.

### 12.3 Wire ownership

Rust owns all wire serialization. Kotlin and Swift must not duplicate message encoding.

Use two explicit framing families:

1. **Control frames**
   - fixed magic;
   - protocol version;
   - message kind;
   - flags;
   - payload length;
   - bounded structured payload;
   - maximum size enforced before allocation.

2. **Timing/audio datagrams**
   - fixed-width binary header;
   - network byte order;
   - explicit payload length;
   - session/stream identifiers represented in a bounded binary form;
   - sequence and presentation timestamp;
   - optional checksum or authenticated encryption tag in a later security phase.

The initial codec may use a Rust Serde-compatible bounded format for control payloads, but the chosen crate and exact format must be pinned and accompanied by golden vectors. Language-native Java serialization, Swift `Codable` as an independent wire implementation, and unbounded JSON parsing are prohibited.

### 12.4 Validation

Every incoming frame must validate:

- magic and version;
- message kind;
- maximum length;
- exact payload length;
- session and stream identity;
- numeric ranges;
- sequence/timestamp plausibility;
- duplicate or stale operation IDs;
- authorization state.

Malformed input produces a structured protocol error and diagnostic counter. The transport must not silently ignore malformed messages unless the protocol explicitly classifies a datagram as safe to drop; such drops still increment a diagnostic counter.

---

## 13. Rust-owned SQLite architecture

### 13.1 Ownership

Only Rust opens the SQLite database. Kotlin and Swift:

- determine an application-private database path;
- ensure the parent directory exists;
- pass the complete path in `CoreConfig`;
- apply platform backup/file-protection attributes where required;
- never execute SQL or migrations.

### 13.2 Database worker

One dedicated Rust database worker owns one primary SQLite connection.

```text
Core actor/workers
       │ typed bounded requests
       ▼
Database worker thread
       │ owns connection
       ▼
SQLite database
```

No SQLite call may execute on:

- the real-time audio callback;
- the playback scheduling timing-critical loop;
- packet reception callbacks;
- the platform UI thread.

Database requests must be typed. Arbitrary SQL strings must not cross UniFFI.

### 13.3 Connection policy

On open, Rust must configure and verify:

- `PRAGMA foreign_keys = ON`;
- WAL journal mode when supported;
- a bounded busy timeout;
- an explicit synchronous policy selected for low-write-volume durable mobile data;
- schema version compatibility;
- database integrity metadata.

A failed pragma or unsupported mode must be reported. Do not log and continue with unknown durability semantics.

### 13.4 Initial schema

Migration version 1 should create at least:

```sql
CREATE TABLE schema_migrations (
    version       INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL,
    checksum      TEXT NOT NULL
);

CREATE TABLE app_settings (
    id                         INTEGER PRIMARY KEY CHECK (id = 1),
    sync_sample_window         INTEGER NOT NULL,
    sync_cadence_ms            INTEGER NOT NULL,
    startup_buffer_ms          INTEGER NOT NULL,
    late_packet_threshold_ms   INTEGER NOT NULL,
    hard_resync_threshold_ms   INTEGER NOT NULL,
    sync_drift_threshold_ms    REAL NOT NULL,
    scan_window_ms             INTEGER NOT NULL,
    updated_at_ms              INTEGER NOT NULL
);

CREATE TABLE trusted_devices (
    device_id          TEXT PRIMARY KEY,
    display_name       TEXT NOT NULL,
    public_key         BLOB,
    private_key_ref    TEXT,
    trust_state        TEXT NOT NULL,
    first_seen_ms      INTEGER NOT NULL,
    last_seen_ms       INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL
);

CREATE TABLE session_history (
    session_id         TEXT PRIMARY KEY,
    role               TEXT NOT NULL,
    session_name       TEXT NOT NULL,
    started_at_ms      INTEGER NOT NULL,
    ended_at_ms        INTEGER,
    listener_count     INTEGER NOT NULL DEFAULT 0,
    outcome            TEXT NOT NULL,
    failure_code       TEXT,
    failure_message    TEXT
);

CREATE TABLE diagnostic_runs (
    run_id             TEXT PRIMARY KEY,
    session_id         TEXT,
    started_at_ms      INTEGER NOT NULL,
    ended_at_ms        INTEGER,
    summary_json       TEXT NOT NULL,
    FOREIGN KEY(session_id) REFERENCES session_history(session_id)
);

CREATE INDEX idx_session_history_started
    ON session_history(started_at_ms DESC);

CREATE INDEX idx_diagnostic_runs_session
    ON diagnostic_runs(session_id, started_at_ms DESC);
```

The final migration SQL may vary, but it must preserve the semantics above.

### 13.5 Migration rules

- Migrations are ordered, immutable, checksummed, and compiled into Rust.
- Each migration runs in a transaction.
- A checksum mismatch is a fatal database compatibility error.
- A failed migration rolls back and prevents normal core startup.
- The application must show a recoverable or fatal database error state.
- The core must never delete and recreate the database automatically.
- Destructive migration requires an explicit future product decision and backup/export path.
- Downgrade to an older binary that cannot read the current schema must fail explicitly.

### 13.6 Repository APIs

Rust repository interfaces must include:

- load/save validated settings;
- list/get/upsert/delete trusted-device metadata;
- begin/update/end session history;
- append summarized diagnostic run;
- export database metadata for diagnostics;
- cleanly close and checkpoint.

High-frequency packet, callback, or per-frame telemetry must not be persisted directly. Aggregate in memory and persist summaries at controlled intervals or session end.

### 13.7 Database failure behavior

No broad `unwrap`, `expect`, or log-only database error handling is allowed in production paths.

Every failure must include:

- stable error code;
- human-readable diagnostic message;
- operation name;
- whether retry is safe;
- whether the core remains usable;
- database schema version when available.

If settings cannot load, do not silently use defaults while claiming persistence succeeded. A documented first-run case may insert defaults transactionally. Existing-data read failure must be visible.

---

## 14. Secure storage boundary

SQLite does not replace platform secure storage.

Rust defines logical secret operations:

- load private identity key;
- create/store private identity key;
- delete private identity key;
- query whether a key exists.

Android implements these with Keystore. iOS implements these with Keychain/Secure Enclave where appropriate.

SQLite stores a `private_key_ref`, public key, and trust metadata. It must not contain a raw unencrypted private identity key unless a future encrypted-database design explicitly approves that behavior.

Secure-store operation failure is a structured platform event and cannot be downgraded to an anonymous or temporary identity without explicit user-visible behavior.

---

## 15. Threading and bounded queues

Required execution contexts:

1. **Core actor thread** — serialized authoritative state changes.
2. **Transport runtime** — standard socket I/O and framing.
3. **Playback scheduling worker** — jitter buffer to render ring.
4. **Database worker** — sole SQLite connection owner.
5. **Notification dispatcher** — UniFFI observer notifications.
6. **Platform render callback** — consumer only; external to ordinary Rust scheduling.

All queues must be bounded and have an explicit overflow policy.

- Commands: reject with `CoreBusy` rather than silently dropping.
- Platform events: failure to enqueue is fatal or recoverable according to event type, but always visible.
- Effects: never silently drop.
- Snapshot notifications: may coalesce older snapshots while guaranteeing the latest revision.
- Audio ring: producer handles full capacity according to scheduling policy; it must not overwrite unread frames silently.
- Diagnostics: counters may aggregate, but error records must not disappear without a count and summary.

Uncontrolled `spawn` calls and detached tasks without shutdown ownership are prohibited.

---

## 16. Error model and failure visibility

Define a stable `CoreError` record:

```rust
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

Error codes must be stable enough for tests and UI action selection. UI may localize messages based on code but must retain the diagnostic message for export.

Required principles:

- do not report success before real transport, storage, or platform completion;
- zero recipients is not delivery success;
- partial delivery is explicit;
- malformed packets are explicit diagnostics;
- unavailable database is not replaced with an in-memory database silently;
- unavailable native audio is not replaced with a fake playback engine;
- debug/demo behavior is compile-time gated and visibly marked;
- recovery that loses state requires explicit user action;
- a Rust panic at an FFI boundary is contained and surfaced as fatal bridge failure.

---

## 17. Android presentation and adapter responsibilities

After migration, Android `MainViewModel` should become a thin platform/presentation coordinator. It may:

- own Android lifecycle integration;
- map Compose actions to `CoreCommand`;
- expose `CoreSnapshot` through `StateFlow`;
- run `PlatformEffect` operations;
- map semantic capabilities to Android permission requests;
- obtain file selections and stable application-private paths;
- configure BLE/NSD/Wi-Fi Direct discovery adapters;
- establish or release Android-specific networks;
- configure and stop Oboe output;
- use Android Keystore;
- share diagnostic exports.

It must not:

- maintain an independent host/listener state machine;
- packetize audio;
- calculate sync offset;
- own a jitter buffer;
- write audio frames from Kotlin;
- execute SQL;
- duplicate trusted-device rules;
- convert delivery failures into success messages.

Compose screens should render immutable presentation state and emit user intents. They must not directly call native audio or transport services.

---

## 18. Build and packaging requirements

### 18.1 Rust quality gates

Required commands:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Unsafe Rust is denied by default in `silent-disco-core`. Any unsafe code is isolated in `silent-disco-ffi`, documented with safety invariants, and reviewed through dedicated tests.

### 18.2 Android

- Build Rust for the Android ABIs currently supported by the app.
- Integrate native artifacts into Gradle reproducibly.
- Do not require developers to copy `.so` files manually.
- CMake links Oboe and the Rust library.
- Debug and release builds use matching Rust profiles intentionally.
- Existing `./gradlew test`, lint, and instrumented tests remain available.

### 18.3 Apple

- Produce an XCFramework or equivalent Swift-consumable package.
- Include generated Swift UniFFI bindings and C audio header/module map.
- Support simulator and physical-device architectures required by the smoke target.
- Do not check in machine-specific absolute paths.

### 18.4 Reproducibility

Pin:

- Rust toolchain;
- Cargo dependency versions through `Cargo.lock`;
- UniFFI generation process;
- Android NDK version compatibility;
- generated binding regeneration command.

Generated bindings must either be checked in with a verification task or generated deterministically during the build. The chosen policy must be documented and tested.

---

## 19. Testing strategy

### 19.1 Golden behavior tests

Before deleting Kotlin logic, create fixtures for:

- control messages;
- sync request/response calculations;
- packetization output;
- jitter-buffer ordering;
- late and missing packet behavior;
- state transitions;
- delivery classification;
- tuning validation;
- database settings and trust records.

Rust must reproduce approved behavior or document an intentional protocol version change.

### 19.2 Rust unit tests

Cover:

- every state transition;
- invalid commands;
- stale operation completion;
- protocol bounds;
- fuzz/property tests for parsers;
- sync estimator edge cases;
- ring wraparound, full, empty, partial reads, and producer/consumer races;
- panic boundary behavior;
- migrations from every supported schema version;
- database rollback and checksum mismatch;
- queue overflow behavior;
- shutdown ordering.

### 19.3 FFI tests

- Kotlin calls Rust core initialization and commands.
- Swift smoke target does the same.
- C ABI tests validate null pointers, incorrect channels, partial reads, silence fill, stopping state, and contained panic behavior.
- Binding version mismatch is rejected.

### 19.4 Android integration tests

- existing UI state behavior remains correct;
- core startup and database initialization;
- Oboe output starts and stops repeatedly;
- no callback after engine release;
- app background/foreground transitions;
- BLE/discovery failures remain visible;
- Wi-Fi Direct adapter reports endpoint facts rather than mutating domain state;
- two-device host/listener playback validation.

### 19.5 Performance tests

At minimum measure:

- audio callback duration distribution;
- ring underrun count;
- scheduling worker latency;
- packet receive-to-render latency;
- database operation latency outside audio path;
- notification backlog;
- memory growth during multi-minute playback.

No performance claim is accepted without measured device results.

---

## 20. Migration strategy

Migration is staged behind interfaces and feature flags where necessary.

1. Establish Rust workspace and build gates.
2. Capture Kotlin golden fixtures.
3. Move protocol and sync math.
4. Introduce Rust SQLite and migrate settings/trust persistence.
5. Add core actor, commands, events, effects, and snapshots.
6. Move host/listener state ownership into Rust.
7. Move streaming packetization and jitter scheduling.
8. Add Rust render ring and C ABI.
9. Implement Oboe consumer and remove `AudioTrackPlaybackEngine` from production.
10. Move standard IP socket/framing runtime to Rust.
11. Reduce Kotlin ViewModel to platform/presentation responsibilities.
12. Add Swift binding and iOS smoke target.
13. Validate Android devices before beginning full iOS UI work.

At each stage:

- the Android app must compile;
- tests must pass;
- old and new ownership must not both be authoritative;
- temporary compatibility adapters must be marked and scheduled for removal;
- failures must remain visible.

Rollback means selecting the previous complete implementation at a feature boundary. It must not mean maintaining two indefinitely diverging state machines.

---

## 21. Non-goals for this migration

The following are not required to complete the shared-core migration:

- full consumer-ready iOS UI;
- mesh or relay networking;
- host failover/election;
- Internet relay service;
- DRM-protected audio support;
- playlists or social features;
- end-to-end protocol encryption, unless added as a separate reviewed security phase;
- automatic database reset after corruption;
- background playback behavior beyond what is needed for controlled tests;
- support for arbitrary sample rates inside the render ring;
- retaining the current Kotlin domain API for backward compatibility.

---

## 22. Acceptance criteria

The architecture migration is complete only when all of the following are true.

### Core ownership

- Rust is the sole owner of host/listener/session/playback authoritative state.
- Rust owns protocol serialization, sync estimation, packetization, jitter buffer, scheduling, and diagnostics.
- Kotlin contains no duplicate authoritative state transition implementation.

### Persistence

- All SQLite access occurs in Rust.
- Migrations are transactional and checksummed.
- Android `SharedPreferences` domain settings/trust persistence has been migrated or intentionally removed.
- Database failures are visible and never cause silent recreation.

### Audio

- Rust owns a bounded SPSC render ring.
- Android production playback uses Oboe consuming the Rust C ABI.
- No UniFFI, allocation, logging, database, JNI call, or blocking lock occurs in the audio callback.
- Shutdown guarantees no callback accesses released Rust memory.
- Underruns and callback failures are visible in diagnostics.
- `AudioTrackPlaybackEngine` is not used as a hidden production fallback.

### Bindings

- Kotlin and Swift bindings are generated and version-checked.
- An iOS smoke target opens the core and database, submits a command, receives a snapshot, and links the audio ABI.
- Rust panics cannot unwind across FFI.

### Networking

- Platform discovery produces semantic endpoint events.
- Rust owns the selected cross-platform wire framing and standard IP transport behavior.
- Android Wi-Fi Direct is treated as a platform network-establishment adapter, not the cross-platform protocol.
- Malformed and oversized inputs are rejected with diagnostics.

### Quality

- Rust format, clippy, and tests pass.
- Android unit tests and lint pass.
- Required Android instrumented tests pass.
- Two-device Android playback works with measured diagnostics.
- No dangerous silent fallback or fake-success path is introduced.

---

## 23. Implementation handoff rules

The implementation must follow `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` in order unless a task explicitly permits parallel work.

For every TODO block:

1. implement the smallest complete vertical change;
2. add or update tests;
3. run the listed validation commands;
4. do not mark the task complete unless acceptance criteria pass;
5. commit the block intentionally;
6. keep `master` buildable;
7. record any deviation in `memory.md` with the reason and follow-up task.

Do not create or reference additional assistant-generated design documents unless they are also committed to the exact repository path named in the spec/TODO. This specification and its companion TODO are a self-contained handoff package.