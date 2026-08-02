# Silent Disco Tauri Desktop Host TODO

**Status:** Ready for staged implementation  
**Date:** 2026-07-27  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Specification:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md`  
**Shared-core specification:** `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`  
**Shared-core TODO:** `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`

---

## 0. How to execute this TODO

Use the Ralph Loop. This is a staged implementation, not a request to scaffold every layer and leave placeholders.

For each block:

1. Read the specification and every production file named by the block.
2. Confirm prerequisite blocks and shared-core dependencies are complete.
3. Inspect the current repository rather than assuming paths or APIs from this document still match.
4. Implement the smallest coherent production change.
5. Add production-facing tests that exercise the real code path.
6. Run every validation command listed for the block.
7. Fix all failures before continuing.
8. Mark only genuinely completed tasks `[x]`.
9. Commit and push the completed block.
10. Record material decisions, commands, failures, measurements, platform versions, and device results in `memory.md`.

Do not batch unrelated blocks into one large commit merely to reduce commit count.

### 0.1 Coordination with the shared Rust migration

This TODO does not replace `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`.

When a desktop block depends on an unfinished shared block:

- implement the shared responsibility in the shared Rust workspace;
- mark completion in the shared migration TODO;
- run shared Rust and Android gates;
- then continue the desktop block.

Do not implement a desktop-only substitute for a missing shared actor, host state machine, packetizer, transport runtime, scheduler, or render ring.

### 0.2 Non-negotiable rules

- [ ] Do not leave `master` unable to build at the end of a committed block.
- [ ] Do not move the Android project or replace the existing `rust/` workspace with a new root workspace.
- [ ] Do not copy Android `MainViewModel` domain behavior into TypeScript or Tauri-specific Rust.
- [ ] Do not make React authoritative for host, listener, transport, sync, packetization, playback, or persistence state.
- [ ] Do not send PCM, per-packet audio payloads, private keys, or native pointers through Tauri IPC.
- [ ] Do not use an HTML media element or Web Audio as the production synchronized host timeline.
- [ ] Do not open the domain SQLite database through a Tauri SQL plugin.
- [ ] Do not use unbounded channels, queues, histories, logs, packet buffers, or decoder buffers.
- [ ] Do not use broad `catch`, `runCatching`, `unwrap`, `expect`, `let _ =`, or detached tasks to convert real failure into log-only behavior.
- [ ] Do not claim session, discovery, approval, playback, delivery, export, or shutdown success before real completion is reported.
- [ ] Do not report zero-recipient delivery as success.
- [ ] Do not silently fall back to temporary profiles, in-memory databases, plaintext identities, synthetic identities, virtual transport, fake audio, or fake decoding in production.
- [ ] Do not delete or recreate user data automatically after migration, checksum, or corruption failure.
- [ ] Do not grant arbitrary shell or filesystem capability to the Tauri frontend.
- [ ] Do not use floating dependency versions.
- [ ] Do not add or reference an assistant-generated companion document unless it is committed at the exact referenced path.
- [ ] Do not add `Co-Authored-By:` lines; this repository rejects them.

### 0.3 Baseline validation commands

Run as applicable after every block:

```bash
./gradlew test
./gradlew lintDebug

cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

cd ../desktop
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test -- --run
npm run build

cd src-tauri
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo check --all-features
```

Before `desktop/` exists, skip only the unavailable desktop commands and record that fact.

---

# Phase 1 — Baseline, dependency decisions, and Tauri scaffold

## Block 1 — Record the desktop-host baseline

### 1.1 Confirm repository state

- [ ] Record the current commit SHA and default branch in `memory.md`.
- [ ] Confirm the two desktop documents exist at their exact paths.
- [ ] Run the current shared Rust quality gates.
- [ ] Run current Android unit tests and lint.
- [ ] Run the current Android instrumentation suite where the environment supports it.
- [ ] Record known physical-device acceptance gaps from the shared migration TODO.

### 1.2 Confirm shared-core completion status

Record the exact status of shared migration Blocks 10, 12, 14, 16, 19, 23, and 26.

- [ ] Do not infer completion from file names.
- [ ] Inspect production code and tests.
- [ ] Record which desktop phases are blocked by each incomplete shared block.

### 1.3 Inventory desktop-relevant platform assumptions

Record:

- [ ] development Linux distribution and version;
- [ ] Node and npm versions;
- [ ] Rust toolchain version;
- [ ] available desktop audio stack: PipeWire, PulseAudio, and/or ALSA;
- [ ] WebKit/webview development packages;
- [ ] Secret Service/keyring availability;
- [ ] multicast/mDNS availability;
- [ ] Android devices available for interoperability testing;
- [ ] test LAN topology.

**Acceptance:** The project has a recorded, reproducible baseline before desktop files are added.

---

## Block 2 — Select and pin the initial desktop toolchain

### 2.1 Verify Tauri compatibility

- [ ] Verify the current Tauri 2 release line builds with the repository Rust toolchain.
- [ ] Verify the selected frontend template supports React, TypeScript, and Vite.
- [ ] Verify required Linux packages on Ubuntu 24.04 or the selected baseline.
- [ ] Record exact versions and commands in `memory.md`.

### 2.2 Select package versions

Pin exact compatible versions for:

- [ ] `tauri`;
- [ ] `tauri-build`;
- [ ] `@tauri-apps/api`;
- [ ] `@tauri-apps/cli`;
- [ ] dialog plugin;
- [ ] any path/filesystem plugin actually required;
- [ ] React and React DOM;
- [ ] TypeScript;
- [ ] Vite;
- [ ] Tailwind CSS;
- [ ] Redux Toolkit and React Redux;
- [ ] test tooling;
- [ ] Rust-to-TypeScript type generator selected in Block 2.3.

Do not add CPAL, Symphonia, mDNS, credential, or QR dependencies until their dedicated decision blocks.

### 2.3 Select Rust-to-TypeScript generation

Evaluate at least the maintained options applicable to the selected Tauri release.

Required evidence:

- [ ] deterministic output;
- [ ] support for tagged enums and bounded records used by desktop DTOs;
- [ ] no requirement to annotate all shared core domain types with Tauri-specific traits;
- [ ] stale-binding verification command;
- [ ] compatible license;
- [ ] compatible Rust version.

- [ ] Record the selected generator and rejected alternatives in `memory.md`.
- [ ] Pin the selected generator.

**Acceptance:** All initial desktop dependencies are exact, justified, and compatible with the pinned toolchain.

---

## Block 3 — Create the Tauri 2 application scaffold

### 3.1 Create frontend structure

Create at least:

```text
desktop/
├── package.json
├── package-lock.json
├── tsconfig.json
├── tsconfig.node.json
├── vite.config.ts
├── index.html
└── src/
    ├── main.tsx
    ├── App.tsx
    ├── app/
    ├── core/
    ├── screens/
    ├── components/
    └── test/
```

- [ ] Use React and TypeScript strict mode.
- [ ] Configure Tailwind without remote assets.
- [ ] Add format, lint, typecheck, test, and build scripts.
- [ ] Add a minimal accessible startup page.
- [ ] Do not add fake host controls that imply functionality.

### 3.2 Create Tauri package

Create:

```text
desktop/src-tauri/
├── Cargo.toml
├── Cargo.lock
├── build.rs
├── tauri.conf.json
├── capabilities/default.json
└── src/
    ├── main.rs
    └── lib.rs
```

Use a shape similar to:

```toml
[package]
name = "silent-disco-desktop"
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"
license = "MIT"

[dependencies]
serde = { version = "=<PINNED>", features = ["derive"] }
tauri = { version = "=<PINNED>" }
silent-disco-core = { path = "../../rust/silent-disco-core" }

[build-dependencies]
tauri-build = { version = "=<PINNED>" }

[lints.rust]
unsafe_code = "deny"

[lints.clippy]
all = "deny"
pedantic = "warn"
```

Adapt exact features to the selected dependencies.

- [ ] Keep `desktop/src-tauri` outside the `rust/` workspace.
- [ ] Commit `desktop/src-tauri/Cargo.lock`.
- [ ] Deny unsafe code in the desktop shell unless a later reviewed audio adapter requires a narrowly isolated exception.
- [ ] Add only least-privilege capabilities.
- [ ] Disable remote content and development-only tooling in production config.

### 3.3 Add direct shared-core smoke call

Expose a command that returns core version information and the deterministic smoke result from the actual path dependency.

Conceptual example:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreSmokeDto {
    major: u16,
    minor: u16,
    patch: u16,
    smoke: u64,
}

#[tauri::command]
fn get_core_smoke(input: u64) -> CoreSmokeDto {
    let version = silent_disco_core::core_version();
    CoreSmokeDto {
        major: version.major,
        minor: version.minor,
        patch: version.patch,
        smoke: silent_disco_core::deterministic_smoke(input),
    }
}
```

Do not preserve this as a substitute for `CoreHandle` after actor integration.

### 3.4 Validate clean checkout

- [ ] `npm ci` succeeds.
- [ ] frontend quality scripts pass.
- [ ] Tauri Rust quality scripts pass.
- [ ] `npm run tauri build` or the selected production build command succeeds on Linux.
- [ ] application launches and displays the real core version.

**Acceptance:** A clean checkout builds a minimal Tauri app that calls the actual shared Rust core.

---

## Block 4 — Add desktop CI jobs

### 4.1 Frontend quality job

Add a GitHub Actions job that:

- [ ] checks out the repository;
- [ ] installs the pinned/supported Node version;
- [ ] runs `npm ci`;
- [ ] runs format check;
- [ ] runs lint;
- [ ] runs TypeScript check;
- [ ] runs frontend tests;
- [ ] runs frontend production build.

### 4.2 Desktop Rust quality job

- [ ] install Rust `1.97.1` or the intentionally updated repository toolchain;
- [ ] run format check;
- [ ] run strict Clippy;
- [ ] run desktop backend tests;
- [ ] run `cargo check` with all production features.

### 4.3 Linux bundle smoke job

- [ ] install exact documented Linux packages;
- [ ] build the Tauri production bundle;
- [ ] upload useful logs on failure;
- [ ] upload bundle artifacts only when useful and with bounded retention;
- [ ] do not label the job Windows/macOS validation.

### 4.4 Preserve existing jobs

- [ ] shared Rust CI still passes;
- [ ] Android CI still passes;
- [ ] Android instrumentation job still runs;
- [ ] desktop jobs do not change Android NDK or Gradle behavior.

**Acceptance:** CI catches frontend, Tauri backend, and Linux bundle failures independently.

---

# Phase 2 — Profiles, paths, storage smoke, and IPC contracts

## Block 5 — Implement desktop profile identifiers and paths

### 5.1 Add profile types

Create modules similar to:

```text
desktop/src-tauri/src/profile.rs
desktop/src-tauri/src/platform/paths.rs
```

Implement bounded validated types:

- [ ] `ProfileId`;
- [ ] `ProfileDisplayName`;
- [ ] `DesktopProfilePaths`;
- [ ] `ProfileMetadata` with a version field.

Conceptual path record:

```rust
#[derive(Debug, Clone)]
pub struct DesktopProfilePaths {
    pub root: std::path::PathBuf,
    pub domain_database: std::path::PathBuf,
    pub p2_database: Option<std::path::PathBuf>,
    pub sources: std::path::PathBuf,
    pub diagnostics: std::path::PathBuf,
    pub cache: std::path::PathBuf,
}
```

### 5.2 Construct paths from trusted roots

- [ ] resolve Tauri application-local-data path in Rust;
- [ ] create only required parent directories;
- [ ] canonicalize or otherwise validate ownership without requiring the final database file to exist;
- [ ] reject traversal and invalid profile IDs;
- [ ] never accept a complete profile root from frontend input;
- [ ] expose safe display information separately from internal paths.

### 5.3 Add profile metadata

- [ ] write metadata atomically;
- [ ] include schema version;
- [ ] reject unsupported newer metadata;
- [ ] do not overwrite malformed metadata automatically;
- [ ] preserve Unicode display names within bounds.

### 5.4 Add tests

- [ ] valid profile creation;
- [ ] traversal rejection;
- [ ] blank and oversized ID rejection;
- [ ] Unicode display name;
- [ ] unsupported metadata version;
- [ ] partial metadata write recovery;
- [ ] path isolation between profiles.

**Acceptance:** Desktop profiles have deterministic, isolated, tested application-owned paths.

---

## Block 6 — Add process-level profile locking

### 6.1 Select lock implementation

- [ ] choose and pin a maintained cross-platform file/process lock implementation or implement a reviewed OS-specific abstraction;
- [ ] record failure semantics;
- [ ] avoid stale-lock deletion without ownership proof.

### 6.2 Implement lock lifecycle

```rust
pub struct ProfileLease {
    profile_id: ProfileId,
    // Private lock ownership.
}

impl ProfileLease {
    pub fn acquire(paths: &DesktopProfilePaths) -> Result<Self, DesktopError>;
}
```

- [ ] acquire before opening mutable databases;
- [ ] hold for the complete core lifetime;
- [ ] release only after core/database shutdown;
- [ ] prevent a second production core from opening the same profile;
- [ ] report holder/process information only when safe and available;
- [ ] do not open a temporary duplicate profile on failure.

### 6.3 Add multiprocess tests

- [ ] first process acquires;
- [ ] second process fails visibly;
- [ ] lock releases after normal shutdown;
- [ ] abnormal process termination recovery follows selected library semantics;
- [ ] separate profiles can open concurrently.

**Acceptance:** Two desktop processes cannot unknowingly mutate the same profile.

---

## Block 7 — Add desktop storage inspection and migration smoke

### 7.1 Open the real Rust database worker

Use the existing `silent-disco-core` storage API. Do not add desktop SQL.

- [ ] pass the complete profile database path to Rust;
- [ ] run schema creation/migration;
- [ ] query database/schema versions through typed APIs;
- [ ] close and join the worker;
- [ ] display real success or structured failure.

### 7.2 Add read-only inspection commands

Temporary inspection commands may expose:

- [ ] database metadata;
- [ ] validated settings;
- [ ] trusted-device summaries;
- [ ] recent session summaries;
- [ ] P2 store metadata when applicable.

Do not expose raw SQL or raw rows.

### 7.3 Add tests

- [ ] first-open schema creation;
- [ ] reopen latest schema;
- [ ] unsupported newer schema;
- [ ] checksum mismatch;
- [ ] read-only or unwritable path;
- [ ] profile lock release after open failure;
- [ ] no in-memory fallback.

**Acceptance:** The desktop shell proves real Rust-owned persistence before the actor is integrated.

---

## Block 8 — Define desktop DTOs and generated TypeScript bindings

### 8.1 Create DTO module

Create:

```text
desktop/src-tauri/src/dto.rs
desktop/src/core/generated/
```

Define desktop bridge DTOs for:

- [ ] versions;
- [ ] profile summaries;
- [ ] bridge lifecycle;
- [ ] structured errors;
- [ ] storage inspection results;
- [ ] later core snapshots and notifications.

DTOs must:

- [ ] use explicit serde tagging and casing;
- [ ] deny unknown fields where appropriate;
- [ ] bound strings and arrays before core submission;
- [ ] avoid private keys and native paths unless explicitly safe;
- [ ] preserve stable error codes.

### 8.2 Add deterministic generation

Provide commands such as:

```bash
npm run bindings:generate
npm run bindings:check
```

- [ ] generated files are stable across two consecutive runs;
- [ ] CI fails on stale output;
- [ ] generated output is committed if that is the selected policy;
- [ ] no manual duplicate TypeScript enum remains.

### 8.3 Add round-trip tests

- [ ] Rust DTO serializes to expected JSON fixture;
- [ ] TypeScript fixture validates expected tagged union shape;
- [ ] unknown kind fails visibly;
- [ ] oversized input is rejected before core submission;
- [ ] error fields survive conversion.

**Acceptance:** Rust is the source of truth for the desktop IPC contract.

---

# Phase 3 — Shared actor prerequisite and direct desktop integration

## Block 9 — Complete shared migration Block 10 before production desktop state

This block is complete only when Block 10 of `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` is complete in production code.

### 9.1 Verify shared actor records

- [x] `CoreCommand` exists;
- [x] `PlatformEvent` exists;
- [x] `TransportEvent` exists;
- [x] `AudioEvent` exists;
- [x] `StorageEvent` exists;
- [x] `PlatformEffect` exists;
- [x] `CoreNotification` exists;
- [x] `CoreSnapshot` exists;
- [x] `CommandReceipt` exists;
- [x] operation IDs and snapshot revisions are tested.

### 9.2 Verify actor behavior

- [x] one serialized owner;
- [x] bounded queue;
- [x] visible queue-full result;
- [x] no blocking database/network work under actor state ownership;
- [x] stale completion rejection;
- [x] notification outside state lock;
- [x] deterministic shutdown and join.

### 9.3 Run shared acceptance

- [x] host-independent simulated state flow passes;
- [x] shared Rust gates pass;
- [x] Android build/tests/lint pass;
- [x] no desktop-specific type appears in the core actor.

**Acceptance:** The desktop can consume a real authoritative core instead of inventing one.

**Implementation status:** Complete. The shared actor records/runtime are production code, remain free of desktop-specific types, and passed the host-independent actor tests plus the repository Rust/Android and desktop gates recorded for PR #38.

---

## Block 10 — Implement direct `CoreHandle` ownership in Tauri

### 10.1 Add desktop app state

Create:

```text
desktop/src-tauri/src/app_state.rs
desktop/src-tauri/src/shutdown.rs
```

Conceptual shape:

```rust
pub struct DesktopAppState {
    runtime: std::sync::Mutex<DesktopRuntimeState>,
}

struct DesktopRuntimeState {
    lifecycle: DesktopLifecycle,
    profile: Option<ProfileLease>,
    core: Option<std::sync::Arc<silent_disco_core::CoreHandle>>,
    subscription: Option<NotificationSubscription>,
}
```

- [x] mutex poisoning is mapped to a fatal bridge error;
- [x] no audio callback accesses this mutex;
- [x] one production core per process;
- [x] opening twice fails explicitly;
- [x] partially opened resources are cleaned in reverse order;
- [x] cleanup failures are preserved.

### 10.2 Implement open command

Conceptual signature:

```rust
#[tauri::command]
pub async fn open_profile(
    request: OpenProfileRequest,
    state: tauri::State<'_, DesktopAppState>,
) -> Result<OpenProfileResponse, DesktopErrorDto> {
    // Resolve validated profile, acquire lease, initialize adapters,
    // open CoreHandle off the UI thread, and return actual snapshot/version state.
}
```

- [x] do blocking open work off the Tauri/UI thread;
- [x] return actual startup stage and failure;
- [x] do not report ready until identity, storage, actor, and required bridge components are ready;
- [x] return current snapshot revision.

### 10.3 Add current snapshot command

- [x] fetch actual `CoreSnapshot`;
- [x] map through one tested DTO conversion;
- [x] do not derive legal actions in Tauri;
- [x] preserve revision.

### 10.4 Add tests

- [x] successful open;
- [x] second open rejection;
- [x] storage failure cleanup;
- [x] observer setup failure cleanup;
- [x] profile lock retained for core lifetime;
- [x] current snapshot after open;
- [x] idempotent shutdown after partial failure.

**Acceptance:** Tauri owns a direct real `CoreHandle` with deterministic resource ownership.

**Implementation status:** Complete in PR #40. Tauri owns one real shared-core actor and one Rust database worker for the open profile; secure identity, storage, actor, and initial notification delivery must all succeed before `Ready`. Open/close work runs off the UI thread, duplicate opens fail, startup and shutdown cleanup is reverse-ordered and fail-visible, and the current snapshot preserves the authoritative revision. Guarded finalizer run `30393427074` passed desktop Rust formatting, strict Clippy, backend tests/check, generated bindings, Biome formatting/lint, TypeScript, frontend tests/build, and the repository source-size invariant.

---

## Block 11 — Implement the Tauri notification channel

### 11.1 Add bridge

Create:

```text
desktop/src-tauri/src/notification_bridge.rs
```

Conceptual observer:

```rust
struct DesktopCoreObserver {
    sender: BoundedNotificationSender,
}

impl silent_disco_core::CoreObserver for DesktopCoreObserver {
    fn on_notification(&self, notification: silent_disco_core::CoreNotification) {
        if let Err(error) = self.sender.try_send(notification) {
            self.sender.record_delivery_failure(error);
        }
    }
}
```

Adapt to the actual actor API. Do not block the core notification thread on the webview.

### 11.2 Add bounded dispatcher

- [x] bounded bridge queue;
- [x] snapshot coalescing only if latest revision is guaranteed;
- [x] effects and errors never silently dropped;
- [x] channel send failure becomes visible bridge state;
- [x] explicit subscription ID;
- [x] replace or reject duplicate subscriptions intentionally;
- [x] worker stop and join.

### 11.3 Attach Tauri channel

- [x] frontend attaches after open;
- [x] backend sends current snapshot immediately after attachment;
- [x] frontend reload can reattach;
- [x] stale old subscription cannot continue consuming;
- [x] private data is filtered before send.

### 11.4 Add tests

- [x] monotonically increasing snapshots;
- [x] stale snapshot handling;
- [x] closed channel;
- [x] duplicate attach;
- [x] frontend reload;
- [x] bounded queue pressure;
- [x] effect/error delivery under snapshot load;
- [x] shutdown while notification pending.

**Acceptance:** The frontend receives revisioned authoritative notifications without blocking or silent loss.

**Implementation status:** Complete on `master` at commit `3ce9aa8143bf943864d2d30a0db38c6f043bcae8`. The production bridge uses `notification_buffer.rs` for the bounded revision-aware dispatcher and `notification_channel.rs` for the redacted Tauri channel adapter. It replaces and joins stale subscriptions, retains failed non-snapshot notifications for reload delivery, exposes channel and worker failures, and keeps the frontend single-flight and reload-aware. CI #728, Source file line limit #76, and Desktop CI #133 passed for the final implementation commit.

---

## Block 12 — Implement frontend authoritative snapshot storage

### 12.1 Configure Redux Toolkit

Create:

```text
desktop/src/app/store.ts
desktop/src/app/coreSlice.ts
desktop/src/app/uiSlice.ts
desktop/src/app/selectors.ts
```

Core state contains only:

- [ ] latest authoritative snapshot;
- [ ] bridge lifecycle;
- [ ] pending command receipts;
- [ ] bounded errors/diagnostics;
- [ ] stale-notification counters.

UI state contains presentation-only fields.

### 12.2 Add revision guard

```ts
export const shouldAcceptSnapshot = (
  current: number | null,
  incoming: number,
): boolean => current === null || incoming > current;
```

- [ ] equal/older snapshots are rejected;
- [ ] stale count increments;
- [ ] newer snapshot replaces the complete authoritative snapshot;
- [ ] no reducer locally advances host lifecycle.

### 12.3 Add typed client

Create `desktop/src/core/client.ts`.

- [ ] wrap Tauri invokes with generated types;
- [ ] attach notification channel;
- [ ] convert invocation transport failure into bridge error;
- [ ] do not convert failed invoke into successful empty result;
- [ ] do not retry non-idempotent commands automatically.

### 12.4 Add tests

- [ ] initial snapshot;
- [ ] newer revision accepted;
- [ ] equal revision rejected;
- [ ] older revision rejected;
- [ ] pending command remains pending until core evidence;
- [ ] command failure displayed;
- [ ] frontend reconnect obtains current snapshot;
- [ ] no copied transition function exists.

**Acceptance:** React renders the core snapshot and never becomes a competing state owner.

---

# Phase 4 — Rust-authoritative desktop host lifecycle

## Block 13 — Complete shared migration Block 12

Before implementing the production Host UI, complete and verify Block 12 of the shared migration TODO.

### 13.1 Verify Rust host draft validation

- [x] session name;
- [x] audio source requirement;
- [x] invite code;
- [x] approval mode;
- [x] tuning normalization;
- [x] cross-field constraints.

### 13.2 Verify host lifecycle

- [x] idle;
- [x] creating;
- [x] advertising/waiting;
- [x] ready;
- [x] streaming;
- [x] paused;
- [x] ending;
- [x] error/retry.

### 13.3 Verify approval logic

- [x] request deduplication;
- [x] trusted-device policy;
- [x] delivery-first approval;
- [x] delivery-first rejection;
- [x] stale request rejection;
- [x] partial and zero-recipient reporting.

### 13.4 Preserve Android behavior

- [x] Android host UI routes through Rust according to the shared TODO;
- [x] Android tests pass;
- [x] no temporary desktop-only host reducer exists.

**Acceptance:** Host semantics are shared before the desktop exposes real host controls.

**Implementation status:** Complete. The final authority closure makes Android start, resume, pause, and stop await a newer Rust-confirmed playback snapshot before any playback-engine, transport-broadcast, stream-loop, cancellation, or stop side effect. Transition rejection, timeout, and cancellation do not execute success-side effects. The dormant manual trusted-device bypass and its dead persistence helpers were removed; trusted-device persistence now occurs only through Rust storage effects. Guarded Actions run `30524538283` ran the shared Rust, Android, desktop frontend/backend, and source-size regression gates.

---

## Block 14 — Build the desktop Host Setup screen

### 14.1 Add screen

Create:

```text
desktop/src/screens/HostSetupScreen.tsx
```

Include:

- [x] session name;
- [x] approval mode;
- [x] invite code when applicable;
- [x] remember-approved-device setting;
- [x] selected source summary;
- [x] network interface policy summary;
- [x] local monitor preference placeholder only when supported;
- [x] advanced tuning navigation;
- [x] startup and draft validation errors.

### 14.2 Draft behavior

- [x] text entry may remain local until submitted as a typed patch;
- [x] core validation result is authoritative;
- [x] fields display core validation without duplicating rules;
- [x] create button derives from core capability;
- [x] command submission shows pending, not success;
- [x] stale revision rejection refreshes snapshot and preserves safe user edits.

### 14.3 Tests

- [x] keyboard navigation;
- [x] approval-mode conditional controls;
- [x] core validation display;
- [x] pending create state;
- [x] create rejection;
- [x] no transition to hosting without newer snapshot;
- [x] screen-reader labels.

**Acceptance:** A desktop user can edit and submit a real Rust-owned host draft.

**Implementation status:** Complete on `master` at commit `acb15e42400a9c9a18ced1e5f27c3f130a5e54d8`. Guarded Actions run `30522530161` passed generated bindings, source-size enforcement, shared Rust formatting/strict Clippy/tests, desktop formatting/lint/typecheck/tests/build, desktop Rust formatting/strict Clippy/tests/check, and Android assemble/unit-tests/lint.

---

## Block 15 — Implement host session platform-effect runner skeleton

### 15.1 Create effect runner

Create:

```text
desktop/src-tauri/src/platform/mod.rs
desktop/src-tauri/src/platform/discovery.rs
desktop/src-tauri/src/platform/audio_device.rs
desktop/src-tauri/src/platform/diagnostics_export.rs
```

The runner:

- [x] receives `PlatformEffect` with operation ID;
- [x] routes only desktop-owned effects;
- [x] returns `PlatformEvent` with the same operation ID;
- [x] rejects unknown/unsupported effects visibly;
- [x] owns every spawned task;
- [x] supports cancellation and shutdown;
- [x] never mutates core state directly.

### 15.2 Implement initially supported effects

At this stage implement only effects backed by real code, such as:

- [x] request/select source interaction where effect semantics require it;
- [x] diagnostics export path/save operation;
- [x] desktop capability state;
- [x] placeholder unsupported result for discovery/audio output only when the core explicitly expects an unsupported failure.

Do not report unimplemented effects as successful no-ops.

### 15.3 Tests

- [x] operation correlation;
- [x] stale completion rejected by core;
- [x] unknown effect visible;
- [x] task panic/error contained and reported;
- [x] cancellation;
- [x] shutdown joins tasks.

**Acceptance:** Desktop effects follow the same command/effect/completion contract as Android adapters.

**Implementation status:** Complete. The desktop core observer now diverts only `PlatformEffect` notifications into a bounded, owned worker and leaves transport/storage effects on their existing paths. Capability resolution and profile-owned diagnostics JSON export are real operations. Discovery, advertising, network, source preparation, and native audio output remain fail-closed with correlated structured errors until their dedicated blocks; no unsupported effect reports success. Cancellation suppresses success, adapter panics are contained as failed events, shutdown cancels queued effects and joins the worker, and guarded Actions run `30529942712` passed the complete shared Rust, Android, desktop frontend/backend, and source-size regression matrix.

---

# Phase 5 — Audio source selection, staging, and decoding

## Block 16 — Implement secure audio file selection

### 16.1 Add backend-driven dialog

Create:

```text
desktop/src-tauri/src/platform/file_picker.rs
```

- [x] invoke the pinned Tauri dialog plugin from Rust or a narrowly typed frontend command;
- [x] allow only intentional file selection, not directory-wide access;
- [x] treat cancellation separately from failure;
- [x] do not expose unrestricted filesystem capability;
- [x] do not trust extension or MIME alone.

### 16.2 Inspect source metadata safely

- [x] regular file check;
- [x] maximum size check;
- [x] bounded display name;
- [x] canonical source identity where safe;
- [x] no unbounded cover-art/metadata load;
- [x] explicit unsupported source result.

### 16.3 Tests

Use an injectable dialog/file boundary.

- [x] cancellation;
- [x] nonexistent file;
- [x] directory selected;
- [x] oversized file;
- [x] Unicode filename;
- [x] deceptive extension;
- [x] permission denied;
- [x] no success on dialog failure.

**Completion evidence:** Secure single-file selection, bounded signature inspection, opaque backend registration, authoritative capability publication, profile-lifecycle cleanup, frontend integration, and all automated gates passed in GitHub Actions run `30539622045` from source commit `bf9664058c9ca239e6d1995d512782aed81c5921`. Physical interaction with a native desktop file dialog was not performed by this CI run.

**Acceptance:** File selection grants only the access needed to stage one explicit source.

---

## Block 17 — Implement atomic source staging

### 17.1 Create staging module

Create:

```text
desktop/src-tauri/src/platform/source_staging.rs
```

Conceptual algorithm:

```rust
pub fn stage_source(
    source: &std::path::Path,
    profile: &DesktopProfilePaths,
) -> Result<StagedSource, DesktopError> {
    // 1. Open source without following unsafe assumptions.
    // 2. Create unique temporary file inside profile sources directory.
    // 3. Copy with bounded buffer and cancellation checks.
    // 4. Flush and sync according to selected durability policy.
    // 5. Verify length/hash.
    // 6. Atomically rename to final content-addressed or generated name.
    // 7. Return stable descriptor.
}
```

### 17.2 Requirements

- [x] bounded copy buffer;
- [x] progress aggregation at a bounded UI rate;
- [x] cancellation;
- [x] temporary file in destination filesystem;
- [x] collision-safe final naming;
- [x] verify byte length;
- [x] compute a streaming hash if selected;
- [x] atomic rename;
- [x] never delete original source;
- [x] preserve primary and cleanup errors;
- [x] startup cleanup only removes provably incomplete owned temporary files.

### 17.3 Tests

- [x] successful copy;
- [x] cancellation;
- [x] source disappears mid-copy;
- [x] destination full or write failure through injectable boundary;
- [x] hash/length mismatch;
- [x] collision;
- [x] incomplete temporary cleanup;
- [x] existing staged source reuse only after verification;
- [x] source outside profile remains untouched.

**Completion evidence:** Atomic, content-addressed source staging; bounded progress; explicit cancellation; verified reuse; strict owned-temp startup cleanup; frontend integration; and the complete regression matrix passed in GitHub Actions run `30576293784` from validated input commit `7948e62a6526a84c3b4fceacc7971acd9c8e9bbb`. Native file-dialog interaction was not performed by this CI run.

**Acceptance:** The core receives a stable app-owned source path with no destructive or silent recovery.

---

## Block 18 — Resolve the decoder decision gate

Coordinate with Block 23 of the shared Rust migration TODO.

### 18.1 Run decoder spike

Evaluate the current compatible Symphonia release or approved alternatives using representative files.

Measure:

- [x] WAV/PCM support;
- [x] FLAC support;
- [x] MP3 support;
- [x] corrupt-file behavior;
- [x] metadata bounds;
- [x] cancellation behavior;
- [x] decode throughput;
- [x] peak memory;
- [x] startup latency;
- [x] output format conversion needs;
- [x] license and feature set;
- [x] Rust toolchain compatibility.

### 18.2 Choose ownership

Preferred:

- [x] shared Rust streaming decoder module/crate usable by desktop and eligible for mobile.

Temporary allowed alternative:

- [x] desktop Rust decoder adapter feeding the exact shared bounded PCM-ingestion API from Block 14.

Prohibited:

- [x] TypeScript decoder;
- [x] HTML audio decoding;
- [x] full-track decode into one vector;
- [x] hidden fallback between two decoders.

### 18.3 Record decision

- [x] exact crate/version/features;
- [x] supported initial formats;
- [x] measured results;
- [x] rejected alternatives;
- [x] removal plan for any temporary adapter.

**Completion evidence:** Symphonia `0.6.0` with minimal WAV/PCM, FLAC, MP3, and ID3 features was compiled and measured against deterministic valid, corrupt, truncated, oversized-metadata, and cancellation fixtures. Shared Rust streaming decode was selected in evidence commit `0cecbc38cfca68620131ed4c072968896fac2e65`; the executable spike and complete regression matrix were freshly revalidated in GitHub Actions run `30589549529` against audit input `a5e07308e0fc5fdb0bca36b04c58112036643e98`. Results are recorded in `docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md`; measurements are specific to the CI host and are not universal product limits.

**Acceptance:** One explicit decoder path is selected with executable evidence.

---

## Block 19 — Implement bounded streaming decode

### 19.1 Add worker

Implement according to the selected ownership location.

Required interface semantics:

```rust
pub struct DecodedPcmChunk {
    pub format: AudioFormat,
    pub first_sample_index: SampleIndex,
    pub frames: Vec<i16>,
    pub end_of_stream: bool,
}
```

Use the actual shared format selected by Block 14; do not copy this type if it already exists.

### 19.2 Requirements

- [x] bounded chunk frames;
- [x] bounded queued decoded duration;
- [x] checked frame/sample arithmetic;
- [x] explicit channel/sample-rate conversion policy;
- [x] no format change mid-stream without explicit event;
- [x] cancellation and join;
- [x] corrupt and unsupported errors distinguished;
- [x] source position and duration only when valid;
- [x] no entire-track allocation;
- [x] decoder backpressure visible.

### 19.3 Tests

- [x] WAV fixture;
- [x] FLAC fixture;
- [x] MP3 fixture;
- [x] truncated input;
- [x] invalid metadata;
- [x] empty source;
- [x] very short final chunk;
- [x] cancellation;
- [x] queue full;
- [x] restart with new stream/source;
- [x] no memory growth across repeated open/close.

**Completion evidence:** The shared Rust worker, canonical conversion, bounded queue, desktop staged-source adapter, fixture/error/backpressure/lifecycle tests, and complete repository regression matrix passed in GitHub Actions run `30599085238` against direct-master input `4c05e5763b1771fc2c7a04690d46b8c76665aa43`. Architecture and scope are recorded in `docs/DESKTOP_BLOCK19_STREAMING_DECODE.md`.

**Acceptance:** A staged source is decoded incrementally into the shared host packetization boundary.

---

# Phase 6 — Manual LAN host transport and listener control

## Block 20 — Complete shared migration Block 19 transport runtime

Do not implement desktop production transport before the shared runtime passes its own acceptance criteria.

### 20.1 Verify shared runtime

- [x] TCP control listener/client;
- [x] UDP sync endpoint;
- [x] UDP audio endpoint;
- [x] bounded send/receive queues;
- [x] explicit bind/connect errors;
- [x] worker stop/join;
- [x] protocol-v2 framing;
- [x] malformed/oversized rejection;
- [x] peer authorization;
- [x] delivery accounting;
- [x] loopback multi-listener tests.

### 20.2 Verify dependency injection needed by Lab Mode

- [x] transport boundary can be replaced by virtual transport in tests;
- [x] production runtime remains the default in production build;
- [x] no global socket singleton prevents multiple Lab nodes;
- [x] clock dependency remains injectable.

### 20.3 Preserve Android

- [x] Android transport migration tests pass;
- [x] no second desktop framing implementation exists.

**Acceptance:** Shared Rust owns production socket and delivery semantics.

---

**Completion evidence:** Actions run `30605377851` passed against direct-master input `09366180e01f65aba04bed2f95d54fb648449fcb`. See `docs/DESKTOP_BLOCK20_TRANSPORT_RUNTIME.md`.

## Block 21 — Add desktop network-interface and bind policy

### 21.1 Enumerate safe candidate interfaces

- [x] list active interfaces and addresses through a reviewed Rust/platform API;
- [x] classify loopback, link-local, private LAN, VPN, container, and other addresses;
- [x] apply a documented automatic selection policy;
- [x] allow explicit user selection when ambiguous;
- [x] never advertise every interface blindly;
- [x] handle interface changes visibly.

### 21.2 Bind through shared transport

- [x] pass validated bind preference to core/transport API;
- [x] report actual bound addresses and ports;
- [x] no success until sockets bind;
- [x] release partially bound endpoints after failure;
- [x] preserve cleanup errors.

### 21.3 Tests

- [x] loopback-only environment;
- [x] one LAN interface;
- [x] multiple LAN interfaces;
- [x] VPN present;
- [x] container interface present;
- [x] requested address disappears;
- [x] port already in use;
- [x] IPv4/IPv6 policy according to selected baseline.

**Acceptance:** The desktop host exposes intentional, real LAN connection information.

---

**Completion evidence:** Actions run `30613572498` passed against direct-master input `fd081a1574f54956754adcd40c0578933e468c1f`. See `docs/DESKTOP_BLOCK21_NETWORK_BIND_POLICY.md`.

## Block 22 — Implement manual endpoint host workflow

### 22.1 Add host connection DTO

Expose bounded safe data:

- [x] host address;
- [x] control/sync/audio ports;
- [x] session identifier or invitation payload;
- [x] protocol version;
- [x] invite-code requirement;
- [x] expiration where applicable.

### 22.2 Add Host Session screen

Create:

```text
desktop/src/screens/HostSessionScreen.tsx
```

Show:

- [x] authoritative host state;
- [x] manual connection information;
- [x] copy controls;
- [x] pending join requests;
- [x] connected listeners;
- [x] playback controls disabled until supported;
- [x] visible transport errors;
- [x] end-session action.

### 22.3 Add control-only loopback test

- [x] desktop host creates session;
- [x] shared test listener connects manually;
- [x] protocol hello/join exchange succeeds;
- [x] join request appears in snapshot;
- [x] no audio success is claimed;
- [x] disconnect is visible.

**Acceptance:** A listener can reach the desktop host without mDNS.

---

**Completion evidence:** Actions run `30620932603` passed against direct-master input `3f9b90aca0549e5870b34d12cee83c514a2ccd40`. See `docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md`.

## Block 23 — Implement desktop join approval and listener management UI

### 23.1 Pending requests

- [x] render core request ID, safe device summary, age, trust, and invite status;
- [x] approve/reject commands include expected snapshot revision;
- [x] one pending operation per request;
- [x] stale request failure remains visible;
- [x] do not remove request optimistically.

### 23.2 Connected listeners

Create:

```text
desktop/src/screens/ListenerDetailScreen.tsx
```

Show:

- [x] lifecycle;
- [x] last contact;
- [x] sync confidence;
- [x] RTT/offset summaries;
- [x] delivery state;
- [x] retry/resync capability;
- [x] last structured failure;
- [x] remove/disconnect action only when core allows it.

### 23.3 Tests

- [x] approve success after delivery confirmation;
- [x] approval zero-recipient failure;
- [x] approval partial delivery;
- [x] rejection delivery failure;
- [x] stale request;
- [x] trusted-device policy;
- [x] duplicate click prevention;
- [x] keyboard and screen-reader behavior.

**Acceptance:** Desktop listener management uses the shared delivery-first policy.

**Completion evidence:** Actions run `30678111276` passed against direct-master input `8f9d156d5d94cba7178cc01ad8cb546d691da003`. The run validated revision-aware approval/rejection/removal, real pending-control delivery, trusted-device persistence, authoritative UI reconciliation, Linux bundle creation, shared Rust, Android builds/tests/lint, ABI packaging, and Android instrumentation. See `docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md`. Physical Android-to-desktop control interoperability remains Block 24.

---

## Block 24 — First physical Android control interoperability

This block requires a physical Android device.

### 24.1 Prepare test topology

Record:

- [x] desktop OS/hardware;
- [x] Android model/version;
- [x] application build SHA;
- [x] Wi-Fi access point/router (same private-LAN subnet, no isolation confirmed by IP/ping);
- [x] desktop connection type;
- [x] IP addresses;
- [ ] firewall configuration (not independently inspected; no interactive `sudo` available in this
      session — no interference observed in any scenario, but the configuration itself is
      unverified);
- [x] exact commands.

### 24.2 Run control tests

- [x] desktop creates host session;
- [x] Android uses manual endpoint;
- [x] join request reaches desktop;
- [x] approve succeeds;
- [x] reject path succeeds in a separate run;
- [x] disconnect is visible on both sides;
- [x] desktop end-session is visible on Android;
- [x] invalid endpoint fails clearly;
- [x] wrong protocol version fails clearly.

### 24.3 Preserve evidence

- [x] record results in `memory.md`;
- [x] export diagnostics for failures;
- [x] add regression test for each code defect found;
- [x] do not mark audio interoperability complete.

**Acceptance:** One Android listener completes a real control-plane session with the desktop host over the LAN.

---

# Phase 7 — Streaming packetization and desktop-to-Android audio

## Block 25 — Complete shared migration Block 14 packetization

Verify:

- [x] bounded decoded PCM ingestion;
- [x] streaming packetizer;
- [x] session/stream identity;
- [x] sequence/sample index;
- [x] presentation timestamps;
- [x] bounded datagrams;
- [x] end-of-stream;
- [x] queue backpressure;
- [x] worker stop/join;
- [x] no full-track concatenation;
- [x] golden/compatibility tests.

Verified against the pre-existing shared `silent-disco-core` packetizer
(`src/audio/packetizer.rs`, `packetizer_worker.rs`, committed in the earlier
"Implement Rust streaming host audio packetizer (Block 14.2-14.4)" work) via
its existing test suite (`packetizer_tests.rs`: full/short packets,
end-of-stream, format-mismatch/zero-sample/oversized-datagram rejection,
sequence/stream-id restart, worker backpressure/drain/join) and its existing
golden vectors (`testdata/protocol/v2/audio_vectors.txt`,
`boundary_vectors.txt`'s `max_audio_datagram`). No new packetizer code was
needed; the desktop side only had to consume it (Block 26).

Run shared Rust and Android gates.

**Acceptance:** The desktop decoder can feed the actual shared packetizer.

---

## Block 26 — Integrate desktop source, decoder, packetizer, and transport

### 26.1 Source-ready flow

- [x] file selection result is staged;
- [x] core receives stable descriptor;
- [x] decoder initialization reports real format;
- [x] core snapshot reports source ready only after success;
- [x] source failure preserves host draft and user recovery options.

Pre-existing from Blocks 17/19 (`source_staging.rs`, `audio_decode.rs`,
`file_picker.rs`); this block only had to consume it via
`prepare_staged_audio_source`, exercised again by the new end-to-end test.

### 26.2 Playback commands

Wire:

- [x] start;
- [x] pause;
- [x] resume;
- [x] stop;
- [x] end of stream;
- [ ] restart/new stream ID.

The frontend remains pending until core state confirms each transition.

`start`/`pause`/`resume`/`stop_host_playback` are synchronous Tauri commands
that call `CoreActorHandle::submit_audio_event` directly, so by the time the
frontend's `await` resolves the actor has already validated and applied the
transition (unlike the async-delivery join/reject commands, there is no
separate confirmation round trip to wait for). Natural end-of-stream converges
on the same `Stop` broadcast + `Stopped` transition as an explicit stop
(`playback_streamer.rs`'s `run_pump` exit path). Restarting with a *new*
source after a stream completes is wired (`start_playback::start` always
computes a fresh `stream_id`) but is not yet covered by a dedicated test --
left unchecked.

### 26.3 Data flow

- [x] decoder feeds bounded chunks;
- [x] packetizer feeds bounded transport queue;
- [ ] transport reports per-peer delivery;
- [ ] queue pressure becomes snapshot/diagnostic state;
- [x] stop cancels and joins decoder/packetizer workers;
- [x] no PCM or datagram payload enters Tauri IPC.

Real-time audio/sync broadcast frames go through `host_transport.rs`'s
`process_broadcast_frames`, which records only an aggregate last-error string
on failure -- there is no structured per-peer `DeliveryReport` or queue-depth
diagnostic for the playback broadcast path yet (control-plane messages like
join approval already get per-peer `TransportDelivery` reporting; audio/sync
frames do not). This is a real gap against CLAUDE.md's mandatory diagnostics
list (queue depth/overflow) -- left unchecked rather than glossed over.

### 26.4 Tests

- [x] one loopback listener receives expected packet sequence;
- [ ] pause stops future presentation progression according to policy;
- [ ] resume behavior is explicit;
- [ ] stop clears pending stream data;
- [ ] decoder failure mid-stream;
- [ ] transport failure mid-stream;
- [ ] queue full;
- [ ] end-of-stream;
- [ ] second source creates new stream identity.

Only the happy-path end-to-end test
(`start_playback_tests::desktop_host_streams_real_audio_and_answers_sync_requests`)
exists so far: join, approval, `StreamStart`, real audio datagrams, a correct
sync-response round trip, then an explicit `stop_playback`. The remaining
items are real, unimplemented test gaps, not just undocumented behavior --
left unchecked for a follow-up session.

**Acceptance:** The desktop host transmits real bounded audio datagrams through shared Rust code.

---

## Block 27 — Add playback and delivery UI

### 27.1 Playback controls

- [x] start/pause/resume/stop derive from core capabilities;
- [x] pending operation state;
- [ ] source name and validated duration;
- [ ] position based on authoritative timeline;
- [ ] end-of-stream state;
- [x] no HTML audio element.

`HostSessionScreen`'s Play/Resume/Pause/Stop buttons derive their individual
enabled state from `playbackControlsEnabled` (now real, from host lifecycle +
transport-worker + selected-source state) and `playbackState`, and disable
during an in-flight command. Not yet done: no display of the selected
source's name/duration, no playback-position indicator (the actor already
tracks `playbackPositionMs`; it is not surfaced in `HostSessionSnapshotDto`
yet), and no distinct "ended naturally" indicator beyond the generic
`playbackState` status card.

### 27.2 Delivery health

Show bounded aggregate data:

- [x] intended peers;
- [x] successful peers;
- [x] failed peers;
- [x] partial delivery severity;
- [ ] queue pressure;
- [x] per-listener last failure;
- [x] zero-recipient warning.

Intended/successful/failed/severity and per-listener last failure predate
this block; this block added a distinct zero-recipient banner (red, `role="alert"`)
instead of folding it into the generic partial-failure amber styling. Queue
pressure remains unimplemented (see the 26.3 note -- no queue-depth
diagnostic exists yet for the playback broadcast path).

### 27.3 Tests

- [ ] zero-recipient start policy;
- [x] partial delivery display;
- [x] failure not overwritten by later informational state;
- [ ] stale command rejection;
- [x] stop pending;
- [ ] source completion.

Partial-delivery display and failure-persistence-across-refresh were already
tested pre-block (join/approval flows); this block added
`HostSessionScreen playback controls` tests covering per-button
enable/disable by `playbackState`, the Play-becomes-Resume-when-paused
behavior, and a failed `stop` surfacing visibly. Not covered: a specific
"zero recipients" *policy* test for starting playback with no listeners
(today nothing prevents starting with zero listeners -- it is allowed and
simply broadcasts to nobody), rejecting a stale/duplicate playback command,
and a natural end-of-stream completion test.

**Acceptance:** The desktop never presents packet submission as universal listener success.

---

## Block 28 — First physical desktop-to-Android audio test

Not yet attempted. Blocks 25-27's automated work (packetizer verification,
desktop playback wiring, UI wiring) is committed and gated, but this block
specifically requires a human to confirm audio is actually audible and in
sync on a real phone -- something this session cannot verify itself (no way
to hear audio, and no GUI-automation tool available to drive the Tauri
desktop window interactively). A real Android device (`adb devices` shows
one attached) and the Android app (`com.ekkus.silentdisco`) are available in
this environment, so a scripted, non-GUI variant (drive the real Rust
backend directly, connect the real phone via its already-verified manual
endpoint flow, inspect Android logcat for real packet/playback evidence)
is possible as a sanity check, but that is not a substitute for a human
actually listening to confirm sync -- do not check any box in this block
from that alone.

**Update**: the scripted sanity check was attempted (see `manual_real_android_listener_receives_streamed_audio`
in `start_playback_tests.rs`, `#[ignore]`d) and memory.md's 2026-08-02T09:41:24Z
entry for full detail. It confirmed the desktop backend genuinely binds and
listens on a real LAN address and that the manual-connect JSON payload
format is exactly right (the Android UI parsed it and displayed the correct
host/session/protocol version), but the app's own connect attempt was
refused (`os error 111`/`ECONNREFUSED`) on every attempt, even though a raw
`adb shell nc -z` TCP probe from the same phone to the same address/port
succeeded moments apart. Root cause unresolved -- a per-app Android network
routing quirk is the leading unconfirmed hypothesis. Not chased further past
~40 minutes since the user framed this as a lower-stakes pre-check, not the
main deliverable. This is a real, reproducible blocker for whoever attempts
Block 28 next.

**Update 2**: the `ECONNREFUSED` root cause above was found and resolved --
it was the phone's Battery Saver mode (Android's per-app `netpolicy` was
actively blocking the app's UID under `BATTERY_SAVER|APP_BACKGROUND`; `adb`/
`nc` are exempt, which is why they worked while the app didn't). With
Battery Saver off, a real 40-second stream over the real manual-connect path
worked end to end: join, approval, and real audio broadcast all succeeded.

- [ ] **New, more consequential blocker found this same session**: even with
      the connection genuinely working, no sound was heard, because
      `ManualEndpointScreen.kt` (`feature/listener/ManualEndpointScreen.kt:136`)
      shows a static "Audio streaming is not part of this build yet" message
      and was never wired to the real playback pipeline. Manual connect is
      the *only* way to reach the desktop host at all (no BLE/Wi-Fi-Direct
      broadcast from desktop), so this screen must be unified with the
      actor-driven playback pipeline before Block 28 can produce any audible
      result. Full detail and the primary tracking entry: shared Rust
      migration TODO, Block 13.3 note (search `ManualEndpointScreen.kt:136`).
      Also found: a silent-failure bug where `network.stop_playback()` can
      report success even when the actor never actually reaches
      `PlaybackState::Stopped` (`DesktopPlaybackStreamer::join()`'s
      `drop(pump.join())` swallows a panicking/failing pump-thread exit) --
      not yet fixed, see memory.md 2026-08-02 entries for the reproduction.

### 28.1 One listener

- [ ] select supported WAV fixture;
- [ ] join and approve one Android listener;
- [ ] start stream;
- [ ] confirm Android buffers and plays;
- [ ] exercise pause/resume/stop;
- [ ] record sync, RTT, packet-loss, and underrun diagnostics;
- [ ] repeat with FLAC;
- [ ] repeat with MP3.

### 28.2 Failure tests

- [ ] disable Android Wi-Fi during playback;
- [ ] restore network;
- [ ] verify disconnect/recovery policy;
- [ ] stop desktop transport;
- [ ] corrupt source fixture fails visibly;
- [ ] host source read failure does not claim continued normal streaming.

### 28.3 Add regressions

- [ ] every software defect receives an automated regression test;
- [ ] record exact results in `memory.md`.

**Acceptance:** One Android listener plays synchronized audio transmitted by the Linux desktop host.

---

## Block 29 — Multi-listener physical validation

### 29.1 Two Android listeners

- [ ] both join and approve;
- [ ] both complete initial sync;
- [ ] both play the same stream;
- [ ] pause/resume/stop affects both;
- [ ] one listener disconnect does not become full delivery success;
- [ ] remaining listener policy is correct;
- [ ] reconnect/resync behavior is visible.

### 29.2 Measure synchronization

Use an approved measurement method and record:

- [ ] device models;
- [ ] sample count;
- [ ] observed inter-device skew;
- [ ] network conditions;
- [ ] tuning settings;
- [ ] packet loss;
- [ ] underruns;
- [ ] confidence classifications.

Do not claim a maximum skew without measurement.

### 29.3 Listener scaling smoke

- [ ] run with every available physical listener;
- [ ] record CPU, memory, queue high-water marks, and delivery failures;
- [ ] define only evidence-backed limits.

**Acceptance:** At least two Android listeners receive and control the desktop-hosted stream with recorded evidence.

---

# Phase 8 — mDNS and QR convenience

## Block 30 — Select and implement desktop mDNS publication

This block is the home for the honest, fail-loud `"desktop session
advertising/discovery is not implemented yet"` and
`"desktop standard-IP transport is not implemented yet"` errors returned by
`desktop/src-tauri/src/platform/discovery.rs`'s `unsupported_effect` --
confirmed via a 2026-08-02 codebase sweep that these correctly return a real
error rather than silently claiming success; they are simply unbuilt until
this block.

### 30.1 Dependency gate

Evaluate maintained implementations.

Verify:

- [ ] Rust/toolchain compatibility;
- [ ] Linux interface behavior;
- [ ] service update/withdrawal;
- [ ] bounded TXT record handling;
- [ ] license;
- [ ] shutdown/join;
- [ ] testability.

Record the exact selected version and alternatives.

### 30.2 Implement adapter

- [ ] publish only after real host endpoints exist;
- [ ] use core-owned semantic advertisement;
- [ ] validate service and field lengths;
- [ ] withdraw on session end and shutdown;
- [ ] update after endpoint/interface change according to explicit policy;
- [ ] report publication failure;
- [ ] retain manual endpoint as visibly available alternative;
- [ ] never claim discovery active after publication failure.

### 30.3 Tests

- [ ] publish;
- [ ] discover from a test client;
- [ ] withdraw;
- [ ] duplicate service name;
- [ ] interface disappears;
- [ ] daemon/multicast unavailable;
- [ ] oversized metadata;
- [ ] shutdown.

**Acceptance:** mDNS is a real convenience layer and not a hidden requirement for transport.

---

## Block 31 — Add desktop QR invitation display

### 31.1 Core-generated invitation

- [ ] invitation payload comes from Rust;
- [ ] version, bounds, expiration, and signature validation use existing P2/core code;
- [ ] frontend receives only the safe encoded invitation;
- [ ] no private signing key crosses IPC.

### 31.2 Render QR

- [ ] select and pin a maintained QR rendering library or render from backend-generated safe data;
- [ ] no remote service;
- [ ] copyable text fallback;
- [ ] expiration displayed;
- [ ] refresh command is explicit;
- [ ] stale invitation is not silently reused.

### 31.3 Tests

- [ ] valid invitation;
- [ ] expired invitation;
- [ ] tampered invitation;
- [ ] oversized payload;
- [ ] QR rendering failure with text fallback;
- [ ] Android scan/join physical test.

**Acceptance:** Android can join the desktop host through a Rust-generated invitation QR code.

---

# Phase 9 — Optional local monitor audio

## Block 32 — Complete render-ring prerequisite

Before local monitoring, complete the applicable shared render-ring work from Blocks 16 and 17 of the shared migration TODO.

Verify:

- [ ] 48 kHz stereo float32 internal format or intentionally updated approved format;
- [ ] bounded preallocated ring;
- [ ] single producer;
- [ ] single consumer registration;
- [ ] no unread overwrite;
- [ ] nonblocking consumer;
- [ ] telemetry;
- [ ] stress tests;
- [ ] controlled consumer acquire/release lifecycle.

The desktop may use a safe Rust consumer API rather than the mobile C ABI, but semantics must remain equivalent.

**Acceptance:** A safe desktop render consumer can be acquired without creating a second scheduling path.

---

## Block 33 — Select and validate CPAL or approved audio backend

### 33.1 Spike

With CPAL 0.18.x or current approved candidate, test:

- [ ] default device enumeration;
- [ ] explicit device selection;
- [ ] supported format negotiation;
- [ ] 48 kHz stereo float output where available;
- [ ] fallback conversion policy where approved;
- [ ] PipeWire;
- [ ] PulseAudio/ALSA behavior present on the test system;
- [ ] device removal;
- [ ] stream error callback;
- [ ] callback timing;
- [ ] shutdown quiescence;
- [ ] Rust version and license.

### 33.2 Decide policy

Record:

- [ ] selected backend and features;
- [ ] supported Linux audio stacks;
- [ ] device-selection UX;
- [ ] format conversion location;
- [ ] monitor-failure effect on host transmission;
- [ ] rejected alternatives.

**Acceptance:** The selected backend has measured Linux behavior and explicit failure semantics.

---

## Block 34 — Implement desktop local monitor adapter

### 34.1 Add adapter

Create:

```text
desktop/src-tauri/src/platform/audio_device.rs
```

- [ ] enumerate devices outside callback;
- [ ] configure stream outside callback;
- [ ] acquire one validated render consumer;
- [ ] callback performs bounded ring read and silence fill only;
- [ ] callback performs no Tauri, logging, SQLite, file, network, allocation, or blocking work;
- [ ] atomic telemetry only;
- [ ] errors reach core through non-real-time event path;
- [ ] callback is quiescent before consumer release.

### 34.2 Transmit-only default

- [ ] host can stream with monitor disabled;
- [ ] monitor enable is explicit;
- [ ] monitor failure follows recorded policy;
- [ ] no fake monitor success on headless systems;
- [ ] no automatic switch to HTML audio.

### 34.3 Tests

- [ ] generated test tone through render ring;
- [ ] start/stop repeated;
- [ ] underrun and silence fill;
- [ ] device removal;
- [ ] wrong format;
- [ ] callback after release prevention;
- [ ] host transmit continues or stops exactly according to policy;
- [ ] shutdown under active callback.

**Acceptance:** Optional desktop monitoring uses the same scheduled Rust timeline and respects real-time constraints.

---

# Phase 10 — Diagnostics, lifecycle, and controlled shutdown

## Block 35 — Build desktop diagnostics screen and export

### 35.1 Diagnostics DTO

Expose bounded summaries for:

- [ ] versions;
- [ ] profile/platform;
- [ ] storage;
- [ ] identity availability without secrets;
- [ ] endpoints/interface;
- [ ] transport queues and delivery;
- [ ] listeners;
- [ ] synchronization;
- [ ] decoder/source queues;
- [ ] packetizer;
- [ ] local monitor and render counters;
- [ ] notification bridge;
- [ ] last structured errors;
- [ ] shutdown state.

### 35.2 Screen

Create:

```text
desktop/src/screens/DiagnosticsScreen.tsx
```

- [ ] bounded display;
- [ ] severity and subsystem filters;
- [ ] no color-only communication;
- [ ] safe copy behavior;
- [ ] no private identity or invite secrets;
- [ ] clear stale-data indicator.

### 35.3 Export

- [ ] Rust creates versioned bounded export;
- [ ] save dialog selects destination;
- [ ] temporary write then atomic rename where supported;
- [ ] cancellation distinct from failure;
- [ ] truncation/omission reported;
- [ ] no audio payloads;
- [ ] no raw private paths unless redacted policy approves;
- [ ] no success until file is committed.

### 35.4 Tests

- [ ] secret redaction;
- [ ] bounded size;
- [ ] destination failure;
- [ ] cancellation;
- [ ] existing file policy;
- [ ] partial write cleanup;
- [ ] export after startup failure;
- [ ] export after transport failure.

**Acceptance:** Operational state and failures are diagnosable without leaking secrets.

---

## Block 36 — Implement deterministic application shutdown

### 36.1 Add lifecycle state

States include at least:

- [ ] closed;
- [ ] opening;
- [ ] ready;
- [ ] shutting down;
- [ ] shutdown failed;
- [ ] terminated.

### 36.2 Implement ordered shutdown

Required order:

- [ ] reject new commands;
- [ ] core enters shutdown;
- [ ] stop playback/packet production;
- [ ] withdraw mDNS;
- [ ] stop transport;
- [ ] stop local monitor and confirm callback quiescence;
- [ ] stop decoder/source workers;
- [ ] close core/database workers;
- [ ] stop notification dispatcher;
- [ ] release profile lock;
- [ ] allow process/window exit.

### 36.3 Window close interception

- [ ] close event initiates controlled shutdown;
- [ ] duplicate close is idempotent;
- [ ] progress is visible;
- [ ] timeout becomes visible failure;
- [ ] timeout does not free callback-visible memory unsafely;
- [ ] development forced-exit behavior, if any, is explicitly gated and labeled.

### 36.4 Tests

- [ ] normal shutdown;
- [ ] shutdown during open;
- [ ] shutdown during source copy;
- [ ] shutdown during decode;
- [ ] shutdown during streaming;
- [ ] shutdown during database write;
- [ ] shutdown with mDNS failure;
- [ ] shutdown with monitor callback active;
- [ ] repeated shutdown;
- [ ] profile can reopen after clean shutdown.

**Acceptance:** The desktop process does not depend on OS termination to clean up shared-core resources.

---

# Phase 11 — Deterministic Lab Mode

## Block 37 — Add explicit Lab Mode build feature and isolation

### 37.1 Feature gates

- [ ] add Rust feature such as `lab-mode`;
- [ ] add frontend build flag derived from backend capability, not only JavaScript environment;
- [ ] production release defaults Lab Mode off unless intentionally selected;
- [ ] UI is visibly labeled;
- [ ] Lab profiles use separate roots;
- [ ] production profile cannot be opened in Lab runtime;
- [ ] synthetic identity and virtual adapters compile only where intended.

### 37.2 Lab runtime

Create:

```text
desktop/src-tauri/src/lab/mod.rs
```

- [ ] owns multiple core handles;
- [ ] unique node IDs;
- [ ] isolated databases;
- [ ] isolated identities;
- [ ] explicit start/stop/join;
- [ ] no global production singleton reuse;
- [ ] bounded node count.

### 37.3 Tests

- [ ] production build has no Lab entry;
- [ ] Lab build is labeled;
- [ ] profile roots differ;
- [ ] production secure-store failure cannot select Lab identity automatically;
- [ ] Lab shutdown releases every node.

**Acceptance:** Lab facilities cannot become a silent production fallback.

---

## Block 38 — Implement deterministic virtual clocks

### 38.1 Shared clock abstraction

If not already present, add a platform-independent trait in the shared core or test-support crate:

```rust
pub trait MonotonicClock: Send + Sync {
    fn now(&self) -> MonotonicMillis;
}
```

Use the actual existing time abstractions where present. Do not duplicate them.

### 38.2 Virtual clock features

- [ ] deterministic initial time;
- [ ] manual advance;
- [ ] scheduled wakeups through the Lab scheduler;
- [ ] per-node offset;
- [ ] per-node drift in ppm;
- [ ] checked arithmetic;
- [ ] no wall-clock sleep required for deterministic scenarios;
- [ ] explicit invalid-discontinuity injection only for negative tests.

### 38.3 Tests

- [ ] exact offset;
- [ ] positive and negative drift;
- [ ] long-run arithmetic;
- [ ] overflow rejection;
- [ ] deterministic repeated seed;
- [ ] scheduler event order at equal timestamps;
- [ ] no production direct system clock remains in shared scheduling logic.

**Acceptance:** Sync and scheduling scenarios can run deterministically without real time.

---

## Block 39 — Implement virtual transport and fault injection

### 39.1 Transport boundary

Use production serialized frames/datagrams.

- [ ] host encodes through production protocol;
- [ ] virtual link receives bytes plus metadata;
- [ ] listener decodes through production protocol;
- [ ] tests claiming wire coverage do not inject high-level success events.

### 39.2 Fault model

Implement bounded deterministic configuration for:

- [ ] latency;
- [ ] jitter;
- [ ] loss;
- [ ] duplication;
- [ ] reordering;
- [ ] corruption;
- [ ] bandwidth limit;
- [ ] queue saturation;
- [ ] connection refusal;
- [ ] disconnect;
- [ ] reconnect delay.

Use a seeded deterministic PRNG where randomness is required.

### 39.3 Tests

- [ ] zero-fault parity;
- [ ] exact fixed latency;
- [ ] deterministic loss sequence;
- [ ] duplicate detection;
- [ ] reorder window;
- [ ] malformed/corrupt packet diagnostics;
- [ ] backpressure;
- [ ] disconnect/reconnect;
- [ ] identical seed produces identical trace;
- [ ] different seed changes trace where expected.

**Acceptance:** Lab Mode tests the real codec and state machines under reproducible transport faults.

---

## Block 40 — Implement scenario schema, runner, and assertions

### 40.1 Select format

- [ ] choose JSON or YAML with exact pinned parser;
- [ ] version schema;
- [ ] bound nodes, links, steps, strings, and duration;
- [ ] reject unknown versions;
- [ ] reject unknown commands and assertions;
- [ ] no arbitrary code execution.

### 40.2 Scenario types

Create:

```text
desktop/src-tauri/src/lab/scenario.rs
desktop/src-tauri/src/lab/recorder.rs
desktop/src-tauri/src/lab/replay.rs
```

Include:

- [ ] seed;
- [ ] nodes;
- [ ] links;
- [ ] clocks;
- [ ] source fixture references restricted to Lab assets;
- [ ] timed commands;
- [ ] fault changes;
- [ ] assertions;
- [ ] timeout and termination policy.

### 40.3 Assertions

Support typed assertions for:

- [ ] lifecycle by deadline;
- [ ] snapshot capability;
- [ ] listener count;
- [ ] sync confidence;
- [ ] bounded offset/RTT;
- [ ] expected error code;
- [ ] delivery severity;
- [ ] underrun/concealment bounds;
- [ ] clean shutdown;
- [ ] no unexpected fatal error.

### 40.4 Tests

- [ ] minimal happy path;
- [ ] invalid schema;
- [ ] unknown version;
- [ ] impossible assertion;
- [ ] timeout;
- [ ] deterministic report;
- [ ] bounded malformed file behavior.

**Acceptance:** Scenarios are executable specifications, not ad hoc UI macros.

---

## Block 41 — Add recording and replay

### 41.1 Record bounded trace

Record:

- [ ] schema/protocol/core versions;
- [ ] seed;
- [ ] clock advances;
- [ ] commands;
- [ ] events;
- [ ] effects;
- [ ] snapshot revisions and safe hashes/full bounded snapshots;
- [ ] packet metadata and payload hashes, not complete audio payload by default;
- [ ] faults;
- [ ] errors;
- [ ] assertion results.

### 41.2 Replay

- [ ] verify compatible versions;
- [ ] reconstruct deterministic schedule;
- [ ] detect divergence at the first meaningful event;
- [ ] produce bounded diff;
- [ ] never silently reinterpret incompatible recording;
- [ ] support conversion only through an explicit versioned future tool.

### 41.3 Tests

- [ ] record then replay identical;
- [ ] changed core behavior produces divergence;
- [ ] incompatible version rejected;
- [ ] truncated recording rejected;
- [ ] secret redaction;
- [ ] bounded output.

**Acceptance:** A difficult failure can be saved and replayed against a later core build.

---

## Block 42 — Build Lab Mode UI

Create:

```text
desktop/src/screens/LabScreen.tsx
```

Provide:

- [ ] node list and state panels;
- [ ] scenario open/save through restricted dialogs;
- [ ] start/pause/step/stop controls;
- [ ] virtual time;
- [ ] fault configuration;
- [ ] bounded event timeline;
- [ ] assertion results;
- [ ] recording export;
- [ ] clear Lab Mode labeling.

UI must not mutate node domain state directly. It submits scenario/test commands to `LabRuntime`.

Tests:

- [ ] keyboard control;
- [ ] invalid scenario display;
- [ ] running-state command disablement;
- [ ] deterministic timeline rendering;
- [ ] bounded history;
- [ ] production build absence.

**Acceptance:** Developers can run reproducible multi-node tests without physical devices.

---

# Phase 12 — Hardening, packaging, and release readiness

## Block 43 — Security and Tauri capability audit

### 43.1 Capability review

- [ ] list every Tauri permission/capability;
- [ ] justify each in `memory.md`;
- [ ] remove unused filesystem access;
- [ ] no shell plugin unless separately approved;
- [ ] no remote URL loading;
- [ ] restrictive CSP;
- [ ] no `eval`;
- [ ] production devtools policy explicit;
- [ ] dialog access scoped;
- [ ] path access constructed in backend.

### 43.2 IPC review

- [ ] every command validates input;
- [ ] every command has bounded payload;
- [ ] no private keys;
- [ ] no PCM/datagrams;
- [ ] no native pointers;
- [ ] no raw SQL;
- [ ] no arbitrary absolute path operation;
- [ ] stale revision policy tested;
- [ ] non-idempotent commands not automatically retried.

### 43.3 Dependency review

For every desktop dependency:

- [ ] exact version;
- [ ] license;
- [ ] features;
- [ ] security advisory check;
- [ ] reason required;
- [ ] platform behavior;
- [ ] transitive native requirements.

**Acceptance:** Desktop privileges and dependencies are intentional and minimal.

---

## Block 44 — Silent-failure and fallback audit

Search production desktop and shared paths.

Suggested commands:

```bash
grep -R "unwrap()\|expect(" desktop/src-tauri/src rust/silent-disco-core/src -n
grep -R "let _ =\|\.ok()" desktop/src-tauri/src rust/silent-disco-core/src -n
grep -R "catch.*console\|console\.error" desktop/src -n
grep -R "fallback\|temporary\|in.memory\|mock\|fake\|demo" desktop rust/silent-disco-core/src -n
grep -R "Audio\|createMediaElement\|WebAudio\|AudioContext" desktop/src -n
grep -R "sql\|sqlite" desktop/src desktop/src-tauri/src -n
```

For every match:

- [ ] remove it;
- [ ] prove it is test-only;
- [ ] prove the ignored result is intentionally non-material;
- [ ] or document and test the explicit visible policy.

Specifically verify:

- [ ] no in-memory database fallback;
- [ ] no plaintext identity fallback;
- [ ] no synthetic production identity;
- [ ] no virtual transport production fallback;
- [ ] no fake decoder/audio fallback;
- [ ] no log-only operational failure;
- [ ] no optimistic success;
- [ ] no automatic destructive database reset;
- [ ] no detached worker hiding shutdown failure.

**Acceptance:** Every production failure has an observable result and controlled state consequence.

---

## Block 45 — Performance and soak testing

### 45.1 Define test matrix

At minimum:

- [ ] one listener;
- [ ] two listeners;
- [ ] five virtual listeners;
- [ ] selected higher virtual count;
- [ ] WAV, FLAC, MP3;
- [ ] transmit only;
- [ ] local monitor;
- [ ] no faults;
- [ ] moderate jitter/loss;
- [ ] reconnect event;
- [ ] one-hour or selected long soak.

### 45.2 Measure

- [ ] CPU;
- [ ] resident memory;
- [ ] decoder throughput;
- [ ] packetizer throughput;
- [ ] transport queue high-water marks;
- [ ] notification backlog;
- [ ] UI update rate;
- [ ] packet delivery severity;
- [ ] sync confidence and offset;
- [ ] underrun/concealment;
- [ ] callback duration;
- [ ] shutdown time;
- [ ] database latency outside real-time paths.

### 45.3 Enforce evidence-based limits

- [ ] choose maximum supported listener count only from results;
- [ ] choose diagnostic/UI aggregation cadence from results;
- [ ] add performance regression thresholds where stable;
- [ ] record environment and raw summaries in `memory.md`;
- [ ] do not hide outliers.

**Acceptance:** Release expectations are based on measured operation rather than desktop hardware assumptions.

---

## Block 46 — Linux packaging

### 46.1 Select package formats

Evaluate and record selected initial formats, for example:

- [ ] AppImage;
- [ ] `.deb`;
- [ ] another intentional format.

### 46.2 Package behavior

- [ ] application ID and product name stable;
- [ ] icons complete;
- [ ] desktop entry correct;
- [ ] required native dependencies documented;
- [ ] clean install on supported Linux baseline;
- [ ] clean upgrade preserving profile data;
- [ ] uninstall does not silently destroy user data;
- [ ] bundle launches without development server;
- [ ] production CSP/capabilities apply;
- [ ] Lab Mode inclusion policy explicit.

### 46.3 Fresh-machine validation

- [ ] install on a clean supported Linux VM/machine;
- [ ] create profile;
- [ ] stage source;
- [ ] host Android listener;
- [ ] export diagnostics;
- [ ] shut down and reopen;
- [ ] verify package uninstall behavior.

**Acceptance:** A packaged Linux build performs the validated desktop-host workflow outside the development tree.

---

## Block 47 — Final Android interoperability acceptance

Run the complete production matrix using packaged Linux build and physical Android devices.

- [ ] manual endpoint join;
- [ ] mDNS discovery;
- [ ] QR invitation;
- [ ] approval/rejection;
- [ ] one listener audio;
- [ ] two listener audio;
- [ ] pause/resume/stop/end;
- [ ] Android disconnect/reconnect;
- [ ] desktop interface disruption;
- [ ] host source failure;
- [ ] local monitor failure with transmit policy;
- [ ] desktop restart;
- [ ] diagnostics export;
- [ ] clean shutdown;
- [ ] reopen profile and session history.

Record exact:

- [ ] desktop package version and SHA;
- [ ] Android APK version and SHA;
- [ ] devices and OS versions;
- [ ] network topology;
- [ ] commands;
- [ ] pass/fail;
- [ ] measured synchronization;
- [ ] known limitations.

**Acceptance:** The packaged Linux desktop is a production-capable Silent Disco host for Android listeners.

---

## Block 48 — Final documentation and completion audit

### 48.1 Update developer documentation

- [ ] add desktop prerequisites;
- [ ] add clean build commands;
- [ ] add development launch command;
- [ ] add production bundle command;
- [ ] add test commands;
- [ ] add physical interoperability procedure;
- [ ] add Lab scenario procedure;
- [ ] add diagnostics location and export procedure;
- [ ] add secure-store troubleshooting without insecure fallback.

Use existing repository guidance files where appropriate. Do not create additional design documents unless required and committed.

### 48.2 Audit ownership

Confirm:

- [ ] Rust actor is authoritative;
- [ ] React is presentation-only;
- [ ] Tauri backend is platform-only;
- [ ] protocol is Rust-only;
- [ ] synchronization is Rust-only;
- [ ] packetization is Rust-only;
- [ ] transport semantics are Rust-only;
- [ ] SQLite is Rust-only;
- [ ] PCM does not cross IPC;
- [ ] local monitor uses shared timeline;
- [ ] Lab adapters cannot activate silently in production.

### 48.3 Run final gates

- [ ] shared Rust format;
- [ ] shared Rust strict Clippy;
- [ ] shared Rust tests;
- [ ] Android tests;
- [ ] Android lint;
- [ ] Android instrumentation;
- [ ] frontend format/lint/typecheck/tests/build;
- [ ] Tauri format/strict Clippy/tests/check;
- [ ] Linux bundle build;
- [ ] deterministic Lab scenarios;
- [ ] loopback transport integration;
- [ ] physical Android acceptance.

### 48.4 Mark completion honestly

- [ ] unresolved platform/device limitations are listed;
- [ ] Windows/macOS are not claimed unless validated;
- [ ] every skipped test has a reason and owner;
- [ ] every referenced file exists at the exact path;
- [ ] `memory.md` contains the final ledger.

**Acceptance:** The implementation satisfies `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md` with executable evidence and no known silent fallback.

---

# Future phases — Not part of Linux desktop-host completion

## Future A — Windows platform adapters

Do not begin until the Linux acceptance block passes.

Future work includes:

- Windows credential protection;
- firewall prompts and network profile behavior;
- Windows audio-device validation;
- mDNS behavior;
- installer/signing;
- native interoperability measurements.

## Future B — macOS platform adapters

Do not begin until Linux acceptance passes.

Future work includes:

- Keychain;
- local-network permission;
- audio-device validation;
- signing/notarization;
- package behavior;
- coordination with the Apple/iOS Rust packaging work.

## Future C — Production desktop listener

A production desktop listener requires a separate reviewed scope. Reuse the same actor, transport, scheduler, render ring, and desktop audio adapter. Do not infer completion from Lab virtual listener support.

This block is the home for the honest, fail-loud
`"desktop native audio output is not implemented yet"` error returned by
`desktop/src-tauri/src/platform/audio_device.rs`'s `unsupported_effect` for
`StartAudioOutput`/`StopAudioOutput` -- confirmed via a 2026-08-02 codebase
sweep that this correctly returns a real error rather than silently claiming
success. Desktop can *host* (broadcast) real audio today; it cannot yet
*receive and play* audio as a listener, and that is what this block covers.

---

# Final completion checklist

- [ ] Tauri 2 desktop application exists under `desktop/`.
- [ ] React/TypeScript/Tailwind frontend passes all gates.
- [ ] Tauri backend directly uses `silent-disco-core`.
- [ ] Shared actor and host lifecycle are Rust-authoritative.
- [ ] Profiles and databases are isolated and locked.
- [ ] Production identity has no insecure silent fallback.
- [ ] Source selection and staging are safe and atomic.
- [ ] Decoder is streaming, bounded, and explicit.
- [ ] Manual LAN hosting works.
- [ ] Android control interoperability works.
- [ ] Bounded Rust audio transmission works.
- [ ] One Android listener plays desktop-hosted audio. **Blocked on:**
      `ManualEndpointScreen.kt`'s playback wiring -- see Block 28's "New,
      more consequential blocker" note and the shared Rust migration TODO's
      Block 13.3 note.
- [ ] At least two Android listeners pass recorded validation.
- [ ] mDNS and QR convenience work without replacing manual connection.
- [ ] Optional local monitor uses the shared timeline.
- [ ] PCM and packet payloads never cross Tauri IPC.
- [ ] Diagnostics are useful and secret-safe.
- [ ] Shutdown is deterministic.
- [ ] Lab Mode is deterministic, isolated, and visibly labeled.
- [ ] Fault injection, recording, replay, and assertions pass.
- [ ] Linux package passes fresh-machine validation.
- [ ] No silent fallback, fake success, destructive recovery, or log-only operational failure remains.
- [ ] All referenced files exist.
- [ ] `memory.md` records final evidence and limitations.
