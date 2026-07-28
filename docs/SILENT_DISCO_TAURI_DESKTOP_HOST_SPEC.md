# Silent Disco Tauri Desktop Host Specification

**Status:** Approved implementation specification  
**Date:** 2026-07-27  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Companion TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`  
**Extends:** `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`  
**Coordinates with:** `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`

---

## 1. Purpose

This document specifies a Tauri 2 desktop application for Silent Disco.

The desktop application is a production-capable Silent Disco host. It is not only a database inspector, protocol demo, or Android test utility. A desktop host must be able to select music, create a session, accept Android listeners, synchronize those listeners, transmit audio, control playback, display listener health, and export diagnostics.

The same desktop application also provides a deterministic Lab Mode for testing the shared Rust core with virtual hosts, virtual listeners, injected network faults, controlled clocks, event recording, and replay.

The desktop application must reuse the authoritative Rust code. It must not port the current Android `MainViewModel` into TypeScript or create a second desktop-only implementation of host state, protocol framing, synchronization, packetization, delivery accounting, persistence, or recovery policy.

The first supported desktop platform is Linux. Windows and macOS follow after Linux host-to-Android interoperability is proven. The architecture must not intentionally prevent those later platforms.

---

## 2. Product outcomes

The completed desktop host must provide the following user-visible outcomes.

1. A user can launch Silent Disco on a Linux desktop or laptop.
2. A user can select a supported local audio file.
3. A user can configure and create a host session.
4. Android listener devices on the same LAN can discover the host or connect through a manual endpoint or QR invitation.
5. The host can approve or reject join requests according to the selected approval policy.
6. The host can start, pause, resume, stop, and end a synchronized stream.
7. The host can see listener connection, synchronization, packet delivery, and error status.
8. The host can optionally monitor the same scheduled stream through a selected desktop audio device.
9. The host can export structured diagnostics without exposing private identity material.
10. Developers can run deterministic multi-node scenarios without physical phones.
11. Developers can reproduce network delay, jitter, loss, reordering, clock offset, clock drift, queue pressure, disconnects, and audio-device failure.
12. The same shared Rust state machines and wire implementation are exercised by Android, desktop, host tests, and Lab Mode.

---

## 3. Relationship to the shared Rust migration

### 3.1 Existing architecture remains authoritative

`docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md` remains the primary shared-core architecture. This specification adds a desktop platform shell and desktop test harness. It does not replace the shared-core design.

When requirements overlap, use the stricter requirement. In particular, all existing rules concerning authoritative Rust state, bounded queues, failure visibility, database ownership, protocol ownership, real-time audio, shutdown ordering, and prohibition of silent fallback remain binding.

### 3.2 Current repository baseline

At the date of this document, the Rust migration is complete through Block 9 of `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`.

The repository already contains:

- the `rust/` Cargo workspace;
- `silent-disco-core`;
- `silent-disco-ffi`;
- `silent-disco-test-support`;
- portable domain identifiers and enums;
- structured core errors;
- protocol-v2 framing and golden vectors;
- Rust clock synchronization logic;
- Rust-owned SQLite worker, schema, migrations, and repositories;
- Android integration for Rust-owned domain persistence;
- P2 recent-session, trust, and invitation validation support.

The following major shared-core work remains incomplete and is required by the production desktop host:

- Block 10: authoritative actor, commands, events, effects, snapshots, and notifications;
- Block 12: Rust-owned host validation, host lifecycle, and approval logic;
- Block 14: bounded streaming host packetization;
- Block 16: Rust-owned render ring, where local monitoring or listener output requires it;
- Block 19: shared Rust standard-IP transport runtime;
- related diagnostics, shutdown, and hardening work.

### 3.3 No desktop fork of unfinished Kotlin behavior

The Tauri project may be scaffolded before the shared actor is complete. It may expose implemented Rust inspection and smoke functionality. It must not fill shared-core gaps by copying unfinished Kotlin domain behavior into the Tauri backend or React frontend.

Temporary desktop-only code is permitted only for platform adapters and explicitly marked test doubles. A test double must never be selected automatically in a production build.

---

## 4. Binding architectural decisions

The following decisions are approved and mandatory.

1. The desktop application uses Tauri 2.
2. The frontend uses React, TypeScript, Vite, and Tailwind CSS.
3. The Tauri Rust package lives under `desktop/src-tauri` and remains separate from the existing `rust/` Cargo workspace.
4. `desktop/src-tauri` depends directly on `rust/silent-disco-core` through a path dependency.
5. The desktop backend does not call the shared core through UniFFI.
6. UniFFI remains the mobile control-plane binding for Kotlin and Swift.
7. The same public `CoreHandle` semantics must support direct Rust use and UniFFI wrapping.
8. Rust is the sole owner of authoritative host and listener domain state.
9. Rust is the sole owner of wire serialization, synchronization algorithms, packetization, delivery accounting, scheduling, and SQLite domain persistence.
10. React owns rendering, local form-editing affordances, accessibility, window layout, and other presentation-only state.
11. The Tauri backend owns desktop platform adapters, process lifecycle, filesystem paths, file dialogs, OS credential integration, desktop audio-device integration, and Tauri IPC.
12. PCM audio never crosses the Tauri IPC boundary.
13. Packet payloads do not cross Tauri IPC during normal operation.
14. High-rate telemetry is aggregated in Rust before reaching the frontend.
15. Tauri commands are used for bounded request/response operations.
16. A Tauri channel is used for the long-lived notification stream from Rust to the frontend.
17. Generic Tauri events must not be used as the primary authoritative snapshot stream.
18. The frontend replaces its stored snapshot only when the incoming revision is newer.
19. Linux is the first implementation and validation platform.
20. Standard same-LAN IP transport is the first production networking mode.
21. Manual endpoint entry is implemented before mDNS discovery.
22. mDNS/DNS-SD is a convenience discovery adapter, not a transport protocol.
23. Android Wi-Fi Direct is not required by the desktop host.
24. The desktop host can transmit without local audio output.
25. Optional local monitoring consumes the shared scheduled timeline; it is not an independent media player.
26. Production and Lab Mode profiles are explicitly separated.
27. No production profile silently uses synthetic identity, virtual transport, virtual clock, fake decoder, fake database, or fake audio output.
28. No operation reports success before the responsible core worker or platform adapter reports completion.
29. No broad catch, panic suppression, log-only failure, or automatic destructive recovery may conceal a real failure.
30. All new dependency versions are pinned after a compatibility check against the repository Rust toolchain.

---

## 5. Supported roles and modes

### 5.1 Production Host Mode

Production Host Mode is the first complete desktop product role.

It supports:

- one active host session per profile;
- local audio-file selection;
- host session configuration;
- listener discovery and manual connection information;
- join approval and rejection;
- synchronized audio transmission;
- playback controls;
- listener and transport diagnostics;
- optional local monitor output;
- session history and trusted-device persistence;
- diagnostics export;
- deterministic shutdown.

### 5.2 Lab Mode

Lab Mode is a first-class development mode, clearly labeled and inaccessible from production builds unless the build intentionally includes it.

It supports:

- multiple in-process core instances;
- one virtual host and multiple virtual listeners;
- isolated databases and identities;
- virtual monotonic clocks;
- virtual transport links;
- deterministic event ordering;
- configurable latency, jitter, loss, duplication, corruption, and reordering;
- clock offset and drift injection;
- disconnect and reconnect injection;
- bounded queue pressure;
- scenario recording and replay;
- machine-readable assertions and reports.

### 5.3 Inspection Mode

An early implementation may expose an inspection surface before full hosting is available.

Inspection Mode may include:

- core and schema version display;
- database open and migration smoke tests;
- settings and trusted-device inspection;
- protocol encode/decode fixtures;
- synchronization calculator fixtures;
- P2 invitation validation;
- diagnostics viewing.

Inspection Mode must state which production features are unavailable. It must not present smoke results as a working host session.

### 5.4 Desktop listener role

A production desktop listener is not required by this specification. The architecture must not preclude it, and Lab Mode may instantiate listener cores, but the first product deliverable is the desktop host with Android listeners.

---

## 6. Target architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│ React + TypeScript frontend                                         │
│                                                                     │
│ • screens and navigation                                            │
│ • accessibility and keyboard control                                │
│ • local form editing                                                │
│ • immutable snapshot rendering                                      │
│ • bounded notification history                                      │
│ • no domain state machine                                            │
│ • no PCM or packet processing                                       │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ Tauri commands + notification channel
┌──────────────────────────────▼──────────────────────────────────────┐
│ Tauri desktop backend                                               │
│                                                                     │
│ • application/profile lifecycle                                     │
│ • CoreHandle registry                                               │
│ • command DTO validation and mapping                                │
│ • platform effect execution                                         │
│ • app-data/cache/export path selection                              │
│ • file picker and source staging                                    │
│ • OS credential store adapter                                       │
│ • mDNS adapter                                                       │
│ • desktop audio device adapter                                      │
│ • Tauri notification bridge                                         │
│ • no independent domain rules                                       │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ direct Rust API
┌──────────────────────────────▼──────────────────────────────────────┐
│ silent-disco-core                                                   │
│                                                                     │
│ • authoritative actor and snapshots                                 │
│ • host/listener state machines                                      │
│ • protocol-v2 framing and validation                                │
│ • TCP/UDP transport runtime                                         │
│ • clock synchronization                                             │
│ • streaming packetization                                           │
│ • jitter/scheduler/render ring                                      │
│ • delivery accounting and diagnostics                               │
│ • SQLite worker and repositories                                    │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ safe Rust render consumer
┌──────────────────────────────▼──────────────────────────────────────┐
│ Optional desktop audio adapter                                      │
│                                                                     │
│ • selected output device                                            │
│ • CPAL or approved equivalent                                       │
│ • allocation-free callback after initialization                     │
│ • consumes scheduled render ring                                    │
│ • no Tauri/React calls from callback                                 │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 7. Repository layout

Add the desktop application without moving the existing Android project or Rust workspace.

```text
desktop/
├── README.md
├── package.json
├── package-lock.json
├── tsconfig.json
├── tsconfig.node.json
├── vite.config.ts
├── postcss.config.*
├── tailwind.config.*
├── index.html
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── app/
│   │   ├── store.ts
│   │   ├── coreSlice.ts
│   │   ├── uiSlice.ts
│   │   └── selectors.ts
│   ├── core/
│   │   ├── client.ts
│   │   ├── commands.ts
│   │   ├── generated/
│   │   ├── notificationReducer.ts
│   │   └── revisionGuard.ts
│   ├── screens/
│   │   ├── StartupScreen.tsx
│   │   ├── HostSetupScreen.tsx
│   │   ├── HostSessionScreen.tsx
│   │   ├── ListenerDetailScreen.tsx
│   │   ├── DiagnosticsScreen.tsx
│   │   ├── SettingsScreen.tsx
│   │   └── LabScreen.tsx
│   ├── components/
│   ├── hooks/
│   ├── styles/
│   └── test/
└── src-tauri/
    ├── Cargo.toml
    ├── Cargo.lock
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/
    │   └── default.json
    ├── icons/
    ├── src/
    │   ├── main.rs
    │   ├── lib.rs
    │   ├── app_state.rs
    │   ├── command_api.rs
    │   ├── notification_bridge.rs
    │   ├── dto.rs
    │   ├── error.rs
    │   ├── profile.rs
    │   ├── shutdown.rs
    │   ├── platform/
    │   │   ├── mod.rs
    │   │   ├── paths.rs
    │   │   ├── file_picker.rs
    │   │   ├── source_staging.rs
    │   │   ├── secure_store.rs
    │   │   ├── discovery.rs
    │   │   ├── audio_device.rs
    │   │   └── diagnostics_export.rs
    │   └── lab/
    │       ├── mod.rs
    │       ├── clock.rs
    │       ├── transport.rs
    │       ├── faults.rs
    │       ├── recorder.rs
    │       ├── replay.rs
    │       └── scenario.rs
    └── tests/
        ├── desktop_bridge.rs
        ├── profile_isolation.rs
        ├── notification_order.rs
        └── shutdown.rs
```

The exact frontend file split may evolve, but the ownership boundaries are mandatory.

### 7.1 Cargo workspace policy

Do not move the repository to a new root Cargo workspace merely to include Tauri.

`desktop/src-tauri/Cargo.toml` is a standalone application package with a direct path dependency:

```toml
[dependencies]
silent-disco-core = { path = "../../rust/silent-disco-core" }
```

The existing `rust/Cargo.toml` continues to own shared-core crates. Shared reusable code belongs there. Tauri-specific code belongs in `desktop/src-tauri`.

### 7.2 Lockfiles

Commit:

- `rust/Cargo.lock` for the shared Rust workspace;
- `desktop/src-tauri/Cargo.lock` for the Tauri Rust application;
- `desktop/package-lock.json` for the frontend.

Do not use unpinned wildcard dependency versions.

---

## 8. Dependency direction and ownership

Dependency direction is mandatory:

```text
React presentation
    → Tauri IPC client
        → Tauri platform shell
            → silent-disco-core
```

The following reverse dependencies are forbidden:

- `silent-disco-core` importing Tauri types;
- `silent-disco-core` importing React or TypeScript concepts;
- Android code importing desktop code;
- the Tauri frontend importing Android models;
- platform adapters mutating domain state directly;
- React deciding legal host or listener transitions;
- React parsing Silent Disco wire packets;
- React computing synchronization offsets;
- React deciding packet-loss concealment or recovery;
- React opening SQLite directly;
- Tauri SQL plugins accessing the domain database;
- Tauri frontend filesystem plugins reading arbitrary selected audio after initial selection without a backend-owned staging policy.

---

## 9. Shared-core public API requirements

### 9.1 One semantic API, two bindings

The core must expose a Rust-native API whose semantics can also be wrapped by UniFFI.

Conceptual shape:

```rust
pub struct CoreHandle {
    // Opaque internal ownership.
}

impl CoreHandle {
    pub fn open(
        config: CoreConfig,
        observer: std::sync::Arc<dyn CoreObserver>,
    ) -> Result<std::sync::Arc<Self>, CoreError>;

    pub fn submit_command(
        &self,
        command: CoreCommand,
    ) -> Result<CommandReceipt, CoreError>;

    pub fn submit_platform_event(
        &self,
        event: PlatformEvent,
    ) -> Result<(), CoreError>;

    pub fn current_snapshot(&self) -> CoreSnapshot;

    pub fn shutdown(&self) -> Result<(), CoreError>;
}
```

The direct desktop API and UniFFI API must not have diverging lifecycle or error semantics.

### 9.2 Platform kind

Add a desktop platform kind if it does not already exist:

```rust
pub enum PlatformKind {
    Android,
    Ios,
    DesktopLinux,
    DesktopWindows,
    DesktopMacos,
    Test,
}
```

Do not use the platform kind to branch domain rules that should be common. Use it for diagnostics, capability reporting, path/security expectations, and platform effect selection.

### 9.3 Desktop capabilities

The core snapshot should expose semantic capability state sufficient for presentation, for example:

- can select source;
- can create host session;
- can approve or reject a specific request;
- can start playback;
- can pause;
- can resume;
- can stop;
- can end session;
- can retry current failure;
- can enable local monitor;
- can export diagnostics.

The frontend must not reverse-engineer legal actions from combinations of lifecycle enums when the core can expose them directly.

---

## 10. Tauri application state

The Tauri backend owns one process-level `DesktopAppState`.

Conceptual shape:

```rust
pub struct DesktopAppState {
    runtime: std::sync::Mutex<DesktopRuntimeState>,
}

struct DesktopRuntimeState {
    active_profile: Option<ProfileId>,
    core: Option<std::sync::Arc<CoreHandle>>,
    notification_subscription: Option<NotificationSubscription>,
    effect_runner: Option<DesktopEffectRunner>,
    monitor: Option<DesktopMonitorHandle>,
    lifecycle: DesktopLifecycle,
}
```

This mutex is not part of the audio callback path. It protects desktop shell ownership, not shared domain state.

### 10.1 Single production core per process

Production Host Mode supports one active `CoreHandle` per desktop process. Opening a second profile while a core is active requires orderly shutdown first.

Lab Mode may own multiple core instances through a separate `LabRuntime`. Production and Lab Mode registries must not share mutable profiles or databases.

### 10.2 No raw handle exposure

The frontend receives opaque profile IDs, operation IDs, and subscription IDs. It never receives native pointers or unrestricted integer handles.

---

## 11. Tauri IPC contract

### 11.1 Commands

Use Tauri commands for bounded operations such as:

```text
get_desktop_versions
list_profiles
create_profile
open_profile
close_profile
attach_notification_channel
submit_host_draft_patch
select_audio_source
create_host_session
approve_join_request
reject_join_request
start_playback
pause_playback
resume_playback
stop_playback
end_host_session
set_local_monitor
update_tuning
retry_recoverable_failure
export_diagnostics
shutdown_application
```

Command names may be grouped in Rust modules, but must remain unique in the generated Tauri handler.

### 11.2 Request DTOs

Tauri request DTOs perform only transport-shape validation and mapping. Domain validation remains in `silent-disco-core`.

Example:

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartHostRequest {
    pub expected_snapshot_revision: u64,
}
```

Use `deny_unknown_fields` where forward-compatibility does not require permissive parsing. Bound every string and collection before passing it to the core.

### 11.3 Revision-aware writes

Commands that act on visible state should include the snapshot revision on which the user acted when stale intent could be dangerous. The core remains the final authority and may reject stale commands.

### 11.4 Notification channel

Use a Tauri IPC channel for the long-lived notification stream.

Conceptual desktop notification shape:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum DesktopNotification {
    Snapshot(DesktopSnapshotDto),
    EffectStatus(DesktopEffectStatusDto),
    Error(DesktopErrorDto),
    Diagnostic(DesktopDiagnosticDto),
    BridgeState(DesktopBridgeStateDto),
}
```

Rules:

- snapshot revisions are monotonically increasing;
- stale snapshots are discarded by the frontend and counted;
- effects and errors are never silently dropped;
- high-rate metrics are aggregated before emission;
- a closed channel becomes visible bridge state;
- frontend reload requires explicit resubscription and current-snapshot retrieval;
- the core continues safe domain operation during a transient frontend reload where possible;
- loss of the only required platform effect consumer is a visible operational failure, not a log-only condition.

### 11.5 Generated TypeScript types

Rust is the source of truth for IPC DTO shapes.

Select and pin one deterministic Rust-to-TypeScript generator during implementation. Generated files live under `desktop/src/core/generated/`. CI must fail when generated bindings are stale.

Do not manually maintain a second independent TypeScript definition for every Rust notification and error record.

### 11.6 Prohibited IPC payloads

Do not send through Tauri IPC:

- decoded PCM chunks;
- render-ring frames;
- per-packet audio payloads;
- private identity keys;
- raw database files;
- unbounded diagnostic logs;
- native pointers;
- unrestricted filesystem paths that have not passed backend validation.

---

## 12. Frontend state model

Use Redux Toolkit or an equivalently explicit immutable store. The initial implementation should use Redux Toolkit unless a repository decision recorded in `memory.md` approves a replacement.

Separate state into:

1. `core` — latest authoritative snapshot, bridge state, and bounded notifications;
2. `ui` — selected tab, modal state, local text-field edits, filters, expansion state, and other presentation-only data;
3. `lab` — scenario editor and run presentation state when Lab Mode is compiled in.

### 12.1 Snapshot replacement rule

```ts
export function acceptSnapshot(
  currentRevision: number | null,
  incomingRevision: number,
): boolean {
  return currentRevision === null || incomingRevision > currentRevision;
}
```

Equal or older revisions are ignored and counted. A revision jump may be accepted but should generate a bridge diagnostic if the notification contract promised contiguous delivery.

### 12.2 No optimistic domain success

The frontend may show a command as pending after Tauri accepts it. It must not show the resulting domain operation as successful until a newer core snapshot or explicit completion notification confirms success.

Examples:

- clicking **Start session** may show “Starting…”;
- it must not show “Hosting” until the core reports the hosting state;
- clicking **Approve** may disable the button while pending;
- it must not remove the request as successfully approved until delivery-first approval completes according to core policy.

### 12.3 Bounded histories

Notification and diagnostic views must be bounded. Old informational entries may be summarized or evicted with a count. Errors must retain a summary count and remain available through diagnostics export.

---

## 13. Desktop profile and path model

### 13.1 Profile structure

Each production profile has isolated paths:

```text
<AppLocalData>/silent-disco/profiles/<profile-id>/
├── profile.json
├── domain.sqlite3
├── p2.sqlite3                 # only if the existing separate P2 store remains required
├── sources/
├── diagnostics/
└── cache/
```

The final filenames must match the actual storage architecture. Do not open one SQLite database through competing connection owners.

### 13.2 Profile identifiers

Profile IDs are validated bounded identifiers. User-facing names are separate and may contain Unicode within explicit length limits.

### 13.3 Path safety

All application-owned paths are constructed in Rust from trusted base directories. The frontend does not concatenate filesystem paths.

File-dialog results are treated as untrusted external paths. Before use, the backend:

1. validates that a file was selected;
2. validates file type and size policy;
3. opens the source safely;
4. stages or copies it into the profile source directory when required;
5. verifies the staged file;
6. returns a stable semantic `AudioSourceDescriptor` to the core.

### 13.4 Concurrent profile protection

A profile lock prevents two Silent Disco processes from opening the same mutable profile simultaneously unless the storage layer explicitly supports and tests that use case.

Lock acquisition failure is visible and includes the profile path and safe recovery guidance. Do not silently copy the profile or open a temporary database.

---

## 14. Persistence

Rust remains the only owner of domain SQLite access.

The desktop shell supplies the database path and lifecycle facts. It does not use a Tauri SQL plugin for domain state.

Requirements:

- create only required parent directories;
- open the database through the existing Rust database worker;
- verify migrations and checksums;
- reject unsupported newer schemas;
- surface corruption and busy errors;
- preserve the database after migration failure;
- never delete and recreate automatically;
- checkpoint and close during orderly shutdown;
- include database and schema versions in diagnostics;
- keep Lab Mode databases isolated from production profiles.

Desktop profile export or backup is not required for the first host milestone, but the path design must allow a later explicit backup feature.

---

## 15. Identity and secure storage

### 15.1 Production identity

Production hosts require a persistent device identity.

Private identity material must be protected through the operating system credential facility where practical:

- Linux: selected Secret Service/keyring backend with an explicit unavailable-state policy;
- Windows: Windows credential protection selected during platform implementation;
- macOS: Keychain.

The core stores public identity, trust metadata, and a protected-secret reference. It does not store an unencrypted private key in SQLite unless a separately reviewed encrypted-storage design approves it.

### 15.2 Linux development constraint

Some Linux desktop or CI environments do not provide a running Secret Service. Production startup must not silently substitute a plaintext key file.

Approved behaviors are:

- fail production identity initialization visibly;
- allow an explicitly selected, visibly labeled development identity provider only in a development/Lab build;
- use an in-memory synthetic identity only in deterministic tests.

### 15.3 Secret handling

Private keys must not appear in:

- React state;
- Tauri IPC;
- logs;
- diagnostics export;
- panic messages;
- SQLite diagnostic JSON;
- scenario recordings.

---

## 16. Host setup workflow

### 16.1 Startup

1. Resolve application directories.
2. Load the selected profile metadata.
3. Acquire the profile lock.
4. Initialize secure identity.
5. Open Rust storage.
6. Open the core actor.
7. Attach the notification bridge.
8. Emit the initial snapshot.
9. Present host setup only after required startup components report success.

A partial startup must not be presented as ready.

### 16.2 Host draft

The host draft includes at least:

- session name;
- approval mode;
- optional invite code according to approval mode;
- remember-approved-device policy;
- selected audio source;
- network bind policy;
- local monitor selection;
- validated tuning settings.

The frontend may preserve in-progress text edits locally. Submission produces a typed patch command. Rust performs final validation.

### 16.3 Session creation

Session creation is effect-driven:

1. frontend submits `CreateHostSession`;
2. core validates the draft and enters creating state;
3. core requests required platform or transport effects;
4. desktop adapters bind endpoints and start discovery advertisement;
5. completion facts return with operation IDs;
6. core enters waiting/ready state only after required operations succeed;
7. failures stop or roll back started resources according to explicit core policy;
8. cleanup failures are reported separately and do not overwrite the primary failure.

---

## 17. Discovery and connection information

### 17.1 Manual endpoint first

The first interoperable connection path is manual endpoint information.

The host displays:

- host address or addresses selected by policy;
- control port;
- synchronization port if separate;
- audio port if separate;
- session ID or bounded invitation payload;
- invite code requirement;
- protocol version;
- QR representation where supported.

Do not display loopback, link-local, container, VPN, or unrelated interfaces without an explicit interface-selection policy.

### 17.2 mDNS/DNS-SD

Add mDNS after manual endpoint interoperability passes.

The desktop discovery adapter publishes a bounded service advertisement containing only information approved by the wire/discovery design. The core owns semantic session advertisement data. The adapter owns operating-system discovery calls.

mDNS failure must not claim discovery is active. Manual connection may remain available and must be shown as an explicit alternative, not a hidden fallback.

### 17.3 QR invitation

QR payload validation remains Rust-owned. Payloads are versioned, bounded, signed where the P2 design requires, and expire according to core policy.

The desktop frontend may render QR visual output from a safe invitation string. It must not build or sign the invitation independently.

---

## 18. Shared standard-IP transport

The desktop host uses the shared Rust transport runtime specified by Block 19 of the Rust migration TODO.

Required transport roles:

- TCP listener for reliable control frames;
- UDP endpoint for clock synchronization;
- UDP endpoint for audio datagrams;
- bounded send and receive queues;
- explicit peer registry;
- delivery accounting;
- structured connection and socket errors;
- deterministic worker stop and join.

The Tauri backend may supply bind preferences and network-interface information. It must not implement a second protocol transport stack.

### 18.1 Multi-listener delivery

For every host broadcast, Rust records:

- intended recipient count;
- successful recipient count;
- failed recipient count;
- per-peer failure context when bounded and safe;
- aggregate delivery severity.

Zero recipients is not successful delivery. Partial delivery is explicit.

### 18.2 Backpressure

All transport queues are bounded.

When a queue is full:

- the item is not silently overwritten;
- the result is visible to the core actor;
- audio policy distinguishes recoverable packet pressure from fatal transport failure;
- control messages are never discarded without a structured result;
- diagnostics include counts and high-water marks.

---

## 19. Audio source selection and staging

### 19.1 File selection

Use the Tauri dialog plugin or an equivalent backend-driven dialog integration. The selected path is never considered safe merely because it came from the dialog.

### 19.2 Stable source

Before session creation or playback, the source must be stable for the session lifetime.

Approved initial policy:

- open and inspect the selected file;
- copy it into the profile `sources/` directory using a temporary filename;
- flush and close the destination;
- verify expected length and content hash where practical;
- atomically rename to the final staged name;
- retain source metadata and hash in the descriptor;
- remove incomplete temporary files after reporting failure;
- never delete the user’s original file.

A later optimization may use a directly opened source handle if lifetime and permission semantics are proven across platforms.

### 19.3 Source limits

Define explicit limits for:

- maximum file size;
- maximum duration where available;
- supported formats;
- metadata string lengths;
- decoded channel count;
- decoded sample rate;
- corrupt or adversarial input behavior.

No unbounded metadata or cover-art allocation is permitted.

---

## 20. Decoder boundary

### 20.1 Desktop preference

The desktop host should prefer a Rust streaming decoder because the selected file is available as a normal stable path and the Tauri backend already runs Rust.

The initial candidate is Symphonia or an approved equivalent. The implementation must pin an exact version compatible with the repository Rust toolchain and validate supported formats before production use.

### 20.2 Shared-core coordination

Do not create a desktop-only decoder ownership model that conflicts with Block 23 of the shared Rust migration.

Approved implementation options are:

1. implement Rust streaming decode in a shared Rust module or crate usable by the desktop host and eligible for later Android/iOS use; or
2. implement a temporary desktop decoder adapter that feeds the core through the same bounded decoded-PCM ingestion API defined by Block 14.

Option 1 is preferred after the required format and performance spike passes.

### 20.3 Initial required formats

The Linux desktop host must prove at least:

- WAV with supported PCM encoding;
- FLAC;
- MP3.

Additional formats are optional until explicitly tested. Unsupported formats fail before the host claims the source is ready.

### 20.4 Streaming requirements

The decoder:

- reads incrementally;
- emits bounded PCM chunks;
- supports cancellation;
- does not decode the entire track into one allocation;
- rejects unexpected format changes unless an explicit conversion path exists;
- converts to the core’s selected packetization format through bounded workers;
- exposes duration and position only when known and validated;
- reports corrupt input distinctly from unsupported format;
- has deterministic shutdown and join.

---

## 21. Host audio pipeline

```text
Staged source file
      │
      ▼
Rust streaming decoder
      │ bounded decoded PCM chunks
      ▼
Shared host packetizer
      │ protocol-v2 audio datagrams
      ▼
Shared Rust transport runtime
      │
      ├── Android listener 1
      ├── Android listener 2
      └── Android listener N
```

The core owns:

- stream ID generation;
- packet sequence;
- sample index;
- presentation timestamps;
- packet duration;
- payload limits;
- end-of-stream policy;
- pause/resume behavior;
- delivery accounting;
- source and packetizer backpressure;
- stream restart semantics.

The Tauri frontend only submits user intent and renders the resulting snapshot.

---

## 22. Optional local monitor

### 22.1 Modes

The desktop host supports:

- **Transmit only** — no desktop audio output;
- **Local monitor** — play the shared scheduled stream through a selected output device.

Transmit-only mode is valid and should be the default for an actual silent disco unless product UX later chooses otherwise.

### 22.2 Shared timeline

Local monitoring must consume audio derived from the same host timeline used for transmitted packets. It must not launch a separate HTML audio element or independent media player.

### 22.3 Desktop audio adapter

Use CPAL or an approved equivalent after an implementation spike.

The callback:

- consumes a preconfigured Rust render consumer;
- does not allocate after stream initialization;
- does not call Tauri;
- does not emit frontend events;
- does not perform file or network I/O;
- does not access SQLite;
- does not log;
- fills missing frames with silence;
- updates atomic counters;
- reports fatal device errors through a non-real-time path.

### 22.4 Device changes

Output-device enumeration and selection are desktop platform responsibilities. Device disappearance, default-device change, unsupported format, and stream failure become explicit platform events.

The core decides whether local-monitor failure stops only monitoring or the entire host stream. That policy must be explicit and tested. The default policy should allow transmit-only hosting to continue when local monitoring is optional and fails, while clearly reporting monitor failure.

---

## 23. Join requests and listener management

The desktop host renders join requests from the authoritative core snapshot.

For each request, show bounded safe information such as:

- display name;
- device ID or shortened fingerprint;
- trust state;
- request age;
- invite-code status;
- transport endpoint summary;
- pending operation state;
- last structured failure.

Approval and rejection are delivery-first according to the shared core policy. The UI must not remove a request or label a listener approved until the core confirms the required control delivery result.

Connected listeners show:

- lifecycle state;
- last contact age;
- synchronization confidence;
- estimated offset and RTT summaries;
- packet delivery or receive health where available;
- resync state;
- recoverable action;
- last error.

Private keys and unnecessary network details are never displayed.

---

## 24. Diagnostics and observability

### 24.1 Desktop diagnostics screen

Provide:

- core, protocol, schema, desktop bridge, and Tauri app versions;
- profile and platform kind;
- selected network interface and bound endpoints;
- active listener count;
- transport queue high-water marks;
- packet delivery summaries;
- synchronization summaries;
- decoder state and throughput;
- source queue state;
- local monitor backend and format;
- render and underrun counters where applicable;
- notification backlog and stale-revision counts;
- database status;
- last structured errors;
- shutdown state.

### 24.2 Diagnostics export

Diagnostics export is created in Rust and saved through a desktop save dialog or app-owned export directory.

Export requirements:

- versioned schema;
- bounded size;
- no private keys;
- no invite secrets unless explicitly redacted;
- no complete audio payloads;
- no unbounded raw packet capture by default;
- include scenario seed and fault profile for Lab Mode;
- report omitted or truncated sections explicitly.

### 24.3 Logging

Logs are supplemental diagnostics, not authoritative state.

A failure is not handled merely because it was logged. Every operational failure must also affect a result, event, notification, snapshot, or controlled shutdown path.

---

## 25. Error model and failure visibility

Map `CoreError` to a stable desktop DTO without losing:

- error code;
- subsystem;
- severity;
- retryability;
- operation ID;
- bounded context;
- diagnostic message.

Desktop platform errors require stable codes for at least:

- app directory unavailable;
- profile locked;
- profile metadata invalid;
- credential store unavailable;
- identity load/store failure;
- file dialog failure;
- source open failure;
- source staging failure;
- decoder unsupported format;
- decoder corrupt source;
- decoder cancelled;
- network interface unavailable;
- bind failure;
- mDNS publish failure;
- audio device unavailable;
- audio stream failure;
- notification channel closed;
- generated binding version mismatch;
- shutdown timeout;
- Lab scenario invalid.

Rules:

- no generic `Unknown` for recognized paths;
- no `unwrap` or `expect` on production fallible operations;
- no `let _ =` discarding a meaningful result;
- no detached task whose failure is unobserved;
- no automatic fallback to temporary profile or in-memory database;
- no automatic fallback to fake audio or virtual transport;
- no success toast based only on command submission;
- cleanup failures are retained even when a primary failure already exists;
- fatal bridge failure prevents new commands and displays recovery guidance.

---

## 26. Threading, queues, and task ownership

Expected execution contexts include:

1. Tauri main/application thread;
2. Tauri async command tasks;
3. shared core actor;
4. shared notification dispatcher;
5. shared database worker;
6. shared transport runtime;
7. decoder worker;
8. packetizer worker;
9. optional scheduling/render producer;
10. optional desktop audio callback;
11. mDNS platform worker;
12. Lab deterministic scheduler.

Every worker has:

- an owner;
- a bounded input queue where applicable;
- explicit startup result;
- explicit stop request;
- join or quiescence confirmation;
- timeout policy;
- visible failure propagation.

Uncontrolled `spawn`, detached futures, and fire-and-forget cleanup are prohibited.

---

## 27. Shutdown order

Production shutdown must be deterministic.

Required conceptual order:

```text
1. Frontend requests shutdown or window close initiates controlled shutdown.
2. Desktop bridge rejects new user commands except shutdown/status.
3. Core enters shutting-down state.
4. Host playback and packet production stop.
5. Discovery advertisement stops.
6. Transport stops accepting peers and closes workers.
7. Optional local monitor stream stops and callback becomes quiescent.
8. Decoder and source workers stop and join.
9. Core completes pending controlled cleanup or reports timeout.
10. Database worker checkpoints, closes, and joins.
11. Notification dispatcher emits final state and stops.
12. Profile lock is released.
13. Tauri process exits.
```

A timeout does not authorize freeing callback-visible memory. Timeout is a visible failure requiring a safe terminal policy.

Window close must not bypass this sequence. The app may delay close while shutdown is in progress, but it must not hang indefinitely without presenting state.

---

## 28. User interface requirements

### 28.1 Startup screen

Show:

- selected profile;
- startup stages;
- exact failed subsystem;
- retry when safe;
- diagnostics export when available;
- Lab Mode entry only when compiled and clearly labeled.

### 28.2 Host setup screen

Provide:

- session name;
- approval mode;
- invite code controls;
- remember-approved-device option;
- audio source selector;
- source metadata and validation state;
- network interface selection or automatic policy summary;
- local monitor toggle and device selection;
- tuning summary with advanced settings link;
- create-session action enabled according to core capability.

### 28.3 Host session screen

Provide:

- session name and state;
- connection information and QR code;
- pending join requests;
- connected listeners;
- playback controls;
- track name, duration, and position where valid;
- stream delivery summary;
- synchronization health summary;
- local monitor state;
- visible warnings and errors;
- end-session control.

### 28.4 Listener detail

Provide listener-specific diagnostics and approved actions without exposing secrets.

### 28.5 Diagnostics screen

Provide human-readable summaries and structured export.

### 28.6 Accessibility

The desktop host must support:

- complete keyboard navigation;
- visible focus;
- semantic labels;
- no color-only status communication;
- scalable text;
- reduced-motion preference where animation exists;
- screen-reader-readable status and error changes;
- confirmation for destructive session-ending actions.

---

## 29. Lab Mode architecture

### 29.1 Separate runtime

Lab Mode uses a `LabRuntime` that owns multiple `CoreHandle` instances through test adapters. It must not inject test behavior into a production core instance.

### 29.2 Virtual clock

Shared algorithms must depend on an injectable monotonic clock abstraction.

The virtual clock supports:

- deterministic starting time;
- manual advancement;
- scheduled callbacks;
- per-node offset;
- per-node drift in parts per million;
- discontinuity injection only when a scenario explicitly tests invalid clocks.

Wall-clock time is not used for scheduling assertions.

### 29.3 Virtual transport

The virtual transport uses the same encoded protocol frames and datagrams as production.

It may intercept packets only at a documented transport boundary. It must not bypass protocol serialization by directly injecting high-level success events for tests that claim wire coverage.

Configurable faults include:

- fixed latency;
- random or scripted jitter;
- packet loss;
- duplication;
- reordering;
- corruption;
- bandwidth limit;
- queue saturation;
- connection refusal;
- half-open connection;
- abrupt disconnect;
- reconnect delay.

### 29.4 Scenario format

Use a versioned, bounded, human-readable scenario file, for example JSON or YAML selected and pinned during implementation.

Conceptual shape:

```yaml
schemaVersion: 1
seed: 12345
nodes:
  - id: host
    role: host
  - id: listener-1
    role: listener
links:
  - from: host
    to: listener-1
    latencyMs: 30
    jitterMs: 8
    lossPercent: 1.0
clocks:
  listener-1:
    offsetMs: 120
    driftPpm: 20
steps:
  - atMs: 0
    command: createHostSession
  - atMs: 100
    command: listenerJoin
  - atMs: 1000
    command: startPlayback
assertions:
  - byMs: 3000
    node: listener-1
    state: playing
```

Do not copy this blindly; final command names and records must use production semantic types.

### 29.5 Recording and replay

Record:

- scenario version and seed;
- node identities as test identifiers only;
- commands;
- platform events;
- encoded transport metadata and hashes;
- effects;
- snapshots and revisions;
- errors;
- clock advances;
- injected faults;
- assertion results.

Replay must detect version incompatibility rather than silently reinterpret an old recording.

---

## 30. Testing strategy

### 30.1 Shared Rust tests

Add or reuse tests for:

- host validation;
- host lifecycle;
- approval and rejection delivery semantics;
- command rejection without mutation;
- stale operation completion;
- snapshot revisions;
- protocol bounds and golden vectors;
- sync offset, RTT, confidence, and drift;
- packetization boundaries;
- queue backpressure;
- multi-listener delivery accounting;
- transport shutdown;
- database path and migration behavior;
- decoder cancellation and corrupt input;
- render ring behavior;
- local monitor failure policy.

### 30.2 Desktop backend tests

Test:

- profile ID and path validation;
- profile locking;
- isolated databases;
- secure-store unavailable behavior;
- file staging atomicity;
- incomplete staging cleanup;
- DTO bounds and unknown fields;
- command-to-core mapping;
- core-notification-to-DTO mapping;
- notification revision order;
- channel closure;
- frontend reload and resubscription;
- deterministic shutdown;
- no second production core per process;
- Lab and production isolation.

### 30.3 Frontend tests

Use a modern React test stack and test:

- snapshot revision guard;
- no optimistic success;
- host setup validation display;
- pending command state;
- join request actions;
- partial delivery display;
- recoverable and fatal errors;
- keyboard navigation;
- bounded diagnostic history;
- startup failure states;
- Lab Mode labeling.

### 30.4 Loopback integration tests

Run shared-core and desktop-backend integration tests over loopback sockets:

- one host and one listener;
- one host and multiple listeners;
- manual endpoint join;
- sync exchange;
- audio packet flow;
- partial peer failure;
- disconnect and reconnect;
- queue pressure;
- shutdown under load.

### 30.5 Android interoperability tests

Validate on physical devices:

1. Linux desktop host and one Android listener;
2. Linux desktop host and at least two Android listeners;
3. manual endpoint connection;
4. mDNS discovery;
5. approval and rejection;
6. start, pause, resume, stop, and end;
7. packet loss and Wi-Fi interruption;
8. listener resynchronization;
9. desktop host restart and listener failure visibility;
10. diagnostics export after failure.

Record exact device models, Android versions, network topology, commands, pass/fail results, and measured synchronization data in `memory.md`.

### 30.6 Performance tests

Measure rather than infer:

- decoder throughput;
- packetizer throughput;
- host scheduling jitter;
- transport send latency;
- per-listener delivery failure rate;
- notification backlog;
- UI update rate;
- memory growth during a multi-minute stream;
- CPU use with one, five, and selected higher listener counts;
- local monitor callback duration;
- underrun count;
- shutdown latency.

No production listener-count claim is accepted without measured results.

---

## 31. Build and CI requirements

### 31.1 Frontend commands

Provide reproducible commands such as:

```bash
cd desktop
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test -- --run
npm run build
```

Exact scripts may differ but must fail nonzero on failure.

### 31.2 Tauri backend commands

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml --all-features
cargo check --manifest-path desktop/src-tauri/Cargo.toml --all-features
```

### 31.3 Shared core commands

Continue running:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

### 31.4 Linux Tauri build

CI installs documented Linux webview and audio development dependencies, then runs a production desktop build.

Do not claim Windows or macOS support based only on cross-compilation or Linux checks.

### 31.5 CI job separation

Add jobs for:

- shared Rust quality gates;
- Android build/tests/lint;
- desktop frontend quality gates;
- desktop Rust quality gates;
- Linux Tauri bundle smoke build;
- deterministic Lab scenarios;
- loopback transport integration.

Desktop failures must upload useful bounded diagnostics such as test reports, generated-binding diffs, and build logs.

---

## 32. Packaging and platform rollout

### 32.1 Linux first

Initial Linux development target: Ubuntu 24.04 or the repository’s selected supported baseline.

Validate:

- WebKit/webview requirements;
- ALSA/PulseAudio/PipeWire behavior through the selected audio library;
- mDNS behavior;
- Secret Service behavior;
- AppImage/deb or selected package formats;
- desktop entry and icons;
- clean uninstall behavior that does not silently delete user data.

### 32.2 Windows

Windows work begins only after Linux desktop-to-Android hosting passes. Add platform credential, firewall, audio-device, mDNS, installer, and signing decisions explicitly.

### 32.3 macOS

macOS work begins after Linux hosting passes and coordinates with future iOS/Apple packaging. Add Keychain, local-network permission, audio-device, notarization, and signing decisions explicitly.

### 32.4 Update mechanism

Automatic updates are not required by the first desktop host milestone. Do not add a partially configured updater that weakens signing or trust requirements.

---

## 33. Security requirements

- Apply least-privilege Tauri capabilities.
- Do not grant arbitrary shell execution.
- Do not expose a generic filesystem read/write API to the frontend.
- Restrict dialog and path access to required operations.
- Validate every IPC request.
- Bound every string, collection, and diagnostic payload.
- Keep private keys out of IPC and logs.
- Treat mDNS and LAN peers as untrusted input.
- Validate protocol version and lengths before allocation.
- Reject stale or mismatched session/stream identifiers.
- Do not trust file extensions as codec proof.
- Handle malicious audio metadata and corrupt files with bounded allocation.
- Do not enable remote Tauri debugging in production.
- Do not load remote web content in the production webview.
- Use a restrictive content security policy.
- Do not permit `eval` or arbitrary script execution.
- Record dependency and license review for Tauri plugins, audio, decoder, discovery, credential, QR, and type-generation crates.

---

## 34. Dependency selection and version policy

At the time of drafting, suitable candidate release lines include:

- Tauri 2.x;
- CPAL 0.18.x;
- Symphonia 0.6.x.

These are candidates, not permission to use floating versions.

Before adding each dependency:

1. verify compatibility with Rust `1.97.1` or the intentionally updated pinned toolchain;
2. verify Linux build prerequisites;
3. verify license compatibility;
4. review default features;
5. disable unnecessary features;
6. pin an exact compatible version through manifest constraints and lockfiles;
7. add a smoke or contract test;
8. record the decision in `memory.md`.

Reference documentation:

- Tauri commands and channels: <https://v2.tauri.app/develop/calling-rust/>
- Tauri filesystem security: <https://v2.tauri.app/plugin/file-system/>
- Tauri permissions: <https://v2.tauri.app/security/permissions/>
- CPAL: <https://docs.rs/cpal/>
- Symphonia: <https://docs.rs/symphonia/>

These external references are implementation aids. This specification remains the repository requirement.

---

## 35. Implementation phases

### Phase A — Desktop scaffold and inspection surface

Create the Tauri application, direct core dependency, type-safe IPC shell, profile paths, version display, database smoke operation, frontend tests, and CI. Do not claim hosting capability.

### Phase B — Shared actor integration

Complete and consume shared-core Blocks 10 and 11 semantics. Desktop uses direct `CoreHandle`; Android uses UniFFI. Prove identical command/snapshot behavior.

### Phase C — Rust-authoritative host lifecycle

Complete and consume Block 12. Desktop can configure and create a simulated host session through real core state.

### Phase D — Manual same-LAN control interoperability

Complete and consume Block 19 control transport. Android listener joins desktop host through manual endpoint. No audio claim yet.

### Phase E — Streaming audio transmission

Complete Block 14 and selected decoder path. Desktop transmits bounded audio datagrams to Android listeners.

### Phase F — Discovery and invitation convenience

Add mDNS publication and QR/manual invitation UX after manual connection is stable.

### Phase G — Optional local monitoring

Add shared-timeline desktop output after render-ring and audio callback requirements are met.

### Phase H — Lab Mode

Add deterministic clocks, virtual transport, fault injection, recording, replay, and scenario assertions.

### Phase I — Linux production packaging

Add Linux bundles, platform identity validation, physical Android interoperability evidence, performance results, and release documentation.

### Phase J — Windows and macOS

Implement and validate platform adapters without changing shared domain semantics.

---

## 36. Acceptance criteria

The Linux desktop host is accepted only when all of the following are true.

1. `desktop/` builds from a clean checkout using documented commands.
2. Frontend format, lint, typecheck, tests, and production build pass.
3. Tauri Rust format, strict Clippy, tests, and production build pass.
4. Shared Rust workspace quality gates still pass.
5. Android quality gates still pass.
6. The desktop backend links directly to `silent-disco-core`.
7. React contains no copied host state machine, protocol codec, sync estimator, packetizer, or SQL.
8. The desktop host opens a real Rust-owned profile database.
9. Startup and profile-lock failures are visible.
10. Production identity failure does not fall back to plaintext or synthetic identity silently.
11. A desktop host can create a session through the Rust actor.
12. An Android listener can connect through manual same-LAN information.
13. Join approval and rejection use Rust delivery-first policy.
14. A supported source is staged and decoded incrementally.
15. Audio packetization is bounded and Rust-owned.
16. One Android listener receives synchronized audio.
17. At least two Android listeners receive synchronized audio in a recorded physical test.
18. Pause, resume, stop, disconnect, and end-session behavior are correct.
19. Partial or zero-recipient delivery is never reported as full success.
20. mDNS failure remains visible while manual connection remains explicitly available.
21. PCM and audio packet payloads never pass through Tauri IPC.
22. Optional local monitor, when enabled, consumes the shared scheduled timeline.
23. Audio-device failure follows the approved monitor-failure policy.
24. Shutdown joins all workers and releases the profile lock.
25. Diagnostics export contains no private keys or audio payloads.
26. Lab Mode is isolated and visibly labeled.
27. At least one deterministic Lab scenario covers latency, jitter, loss, and clock drift.
28. Recorded performance measurements support documented operating limits.
29. No production silent fallback, fake success, destructive database recovery, or log-only operational failure remains.
30. `memory.md` records commands, platform versions, physical devices, network topology, measurements, and unresolved limitations.

---

## 37. Non-goals

The following are not required to complete this specification:

- a production desktop listener role;
- Internet relay or cloud-hosted sessions;
- NAT traversal across unrelated networks;
- mesh networking;
- host failover or election;
- DRM-protected sources;
- streaming-service integration;
- microphone broadcasting;
- playlists or DJ library management;
- audio editing;
- automatic updates;
- account systems;
- cloud synchronization;
- arbitrary Tauri plugin access from the frontend;
- retaining obsolete Kotlin domain APIs for compatibility;
- claiming Windows or macOS support before native validation;
- using Lab Mode as a production fallback.

---

## 38. Decision gates

The following decisions must be resolved with evidence during implementation.

### Gate 1 — Rust-to-TypeScript generator

Select a maintained generator that produces deterministic checked bindings without adding domain logic to TypeScript.

### Gate 2 — Desktop secure-store backend

Select and test the Linux production credential backend and its unavailable-state behavior.

### Gate 3 — Rust decoder ownership

Confirm shared Rust decoding versus temporary desktop adapter after format, performance, and portability tests. Shared Rust decoding is preferred.

### Gate 4 — Desktop audio backend

Validate CPAL or an approved equivalent on the supported Linux audio stacks.

### Gate 5 — mDNS crate or platform implementation

Select a bounded, maintained implementation and verify service withdrawal and interface-change behavior.

### Gate 6 — Linux package formats

Select initial package formats after dependency and install testing.

Every gate decision is recorded in `memory.md` with the tested alternatives and reason. A gate may not be resolved by silently using the first dependency Claude Code finds.

---

## 39. Prohibited shortcuts

The following shortcuts are explicitly prohibited.

- Copying `MainViewModel` host logic into Rust under `desktop/` instead of the shared core.
- Reimplementing the protocol in TypeScript.
- Using Web Audio or an HTML media element as the production host timeline.
- Sending PCM through Tauri commands or events.
- Using an unbounded channel because desktop memory is larger.
- Opening the domain database through a Tauri SQL plugin.
- Treating command acceptance as operation success.
- Treating zero listeners as successful broadcast delivery.
- Continuing after database migration failure with defaults.
- Creating an anonymous identity after secure-store failure.
- Falling back from real transport to virtual transport in production.
- Falling back from the real decoder to a fake source in production.
- Falling back from monitor output to a fake playback engine.
- Ignoring mDNS withdrawal or bind errors.
- Detaching workers to make shutdown appear complete.
- Freeing audio resources before callback quiescence.
- Catching a panic or exception and reporting normal operation.
- Committing generated TypeScript bindings without a stale-check command.
- Referencing an assistant-created companion file that is not committed at the exact path.

---

## 40. Completion definition

This specification is complete when the desktop application is a real Linux Silent Disco host for Android listeners, the host workflow is driven by the same Rust actor and transport code used by the mobile architecture, audio transmission is bounded and measurable, failures remain visible, shutdown is deterministic, and Lab Mode provides reproducible multi-node testing without weakening production behavior.
