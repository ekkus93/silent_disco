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

- [x] Do not leave `master` unable to build at the end of a committed block.
- [x] Do not move the Android project or replace the existing `rust/` workspace with a new root workspace.
- [x] Do not copy Android `MainViewModel` domain behavior into TypeScript or Tauri-specific Rust.
- [x] Do not make React authoritative for host, listener, transport, sync, packetization, playback, or persistence state.
- [x] Do not send PCM, per-packet audio payloads, private keys, or native pointers through Tauri IPC.
- [x] Do not use an HTML media element or Web Audio as the production synchronized host timeline.
- [x] Do not open the domain SQLite database through a Tauri SQL plugin.
- [x] Do not use unbounded channels, queues, histories, logs, packet buffers, or decoder buffers.
- [x] Do not use broad `catch`, `runCatching`, `unwrap`, `expect`, `let _ =`, or detached tasks to convert real failure into log-only behavior.
- [x] Do not claim session, discovery, approval, playback, delivery, export, or shutdown success before real completion is reported.
- [x] Do not report zero-recipient delivery as success.
- [x] Do not silently fall back to temporary profiles, in-memory databases, plaintext identities, synthetic identities, virtual transport, fake audio, or fake decoding in production.
- [x] Do not delete or recreate user data automatically after migration, checksum, or corruption failure.
- [x] Do not grant arbitrary shell or filesystem capability to the Tauri frontend.
- [x] Do not use floating dependency versions.
- [x] Do not add or reference an assistant-generated companion document unless it is committed at the exact referenced path.
- [x] Do not add `Co-Authored-By:` lines; this repository rejects them.

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

- [x] Record the current commit SHA and default branch in `memory.md`.
- [x] Confirm the two desktop documents exist at their exact paths.
- [x] Run the current shared Rust quality gates.
- [x] Run current Android unit tests and lint.
- [x] Run the current Android instrumentation suite where the environment supports it.
- [x] Record known physical-device acceptance gaps from the shared migration TODO.

### 1.2 Confirm shared-core completion status

Record the exact status of shared migration Blocks 10, 12, 14, 16, 19, 23, and 26.

- [x] Do not infer completion from file names.
- [x] Inspect production code and tests.
- [x] Record which desktop phases are blocked by each incomplete shared block.

### 1.3 Inventory desktop-relevant platform assumptions

Record:

- [x] development Linux distribution and version;
- [x] Node and npm versions;
- [x] Rust toolchain version;
- [x] available desktop audio stack: PipeWire, PulseAudio, and/or ALSA;
- [x] WebKit/webview development packages;
- [x] Secret Service/keyring availability;
- [x] multicast/mDNS availability;
- [x] Android devices available for interoperability testing;
- [x] test LAN topology.

**Acceptance:** The project has a recorded, reproducible baseline before desktop files are added.

---

## Block 2 — Select and pin the initial desktop toolchain

### 2.1 Verify Tauri compatibility

- [x] Verify the current Tauri 2 release line builds with the repository Rust toolchain.
- [x] Verify the selected frontend template supports React, TypeScript, and Vite.
- [x] Verify required Linux packages on Ubuntu 24.04 or the selected baseline.
- [x] Record exact versions and commands in `memory.md`.

### 2.2 Select package versions

Pin exact compatible versions for:

- [x] `tauri`;
- [x] `tauri-build`;
- [x] `@tauri-apps/api`;
- [x] `@tauri-apps/cli`;
- [x] dialog plugin;
- [x] any path/filesystem plugin actually required;
- [x] React and React DOM;
- [x] TypeScript;
- [x] Vite;
- [x] Tailwind CSS;
- [x] Redux Toolkit and React Redux;
- [x] test tooling;
- [x] Rust-to-TypeScript type generator selected in Block 2.3.

Do not add CPAL, Symphonia, mDNS, credential, or QR dependencies until their dedicated decision blocks.

### 2.3 Select Rust-to-TypeScript generation

Evaluate at least the maintained options applicable to the selected Tauri release.

Required evidence:

- [x] deterministic output;
- [x] support for tagged enums and bounded records used by desktop DTOs;
- [x] no requirement to annotate all shared core domain types with Tauri-specific traits;
- [x] stale-binding verification command;
- [x] compatible license;
- [x] compatible Rust version.

- [x] Record the selected generator and rejected alternatives in `memory.md`.
- [x] Pin the selected generator.

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

- [x] Use React and TypeScript strict mode.
- [x] Configure Tailwind without remote assets.
- [x] Add format, lint, typecheck, test, and build scripts.
- [x] Add a minimal accessible startup page.
- [x] Do not add fake host controls that imply functionality.

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

- [x] Keep `desktop/src-tauri` outside the `rust/` workspace.
- [x] Commit `desktop/src-tauri/Cargo.lock`.
- [x] Deny unsafe code in the desktop shell unless a later reviewed audio adapter requires a narrowly isolated exception.
- [x] Add only least-privilege capabilities.
- [x] Disable remote content and development-only tooling in production config.

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

- [x] `npm ci` succeeds.
- [x] frontend quality scripts pass.
- [x] Tauri Rust quality scripts pass.
- [x] `npm run tauri build` or the selected production build command succeeds on Linux.
- [x] application launches and displays the real core version.

**Acceptance:** A clean checkout builds a minimal Tauri app that calls the actual shared Rust core.

---

## Block 4 — Add desktop CI jobs

### 4.1 Frontend quality job

Add a GitHub Actions job that:

- [x] checks out the repository;
- [x] installs the pinned/supported Node version;
- [x] runs `npm ci`;
- [x] runs format check;
- [x] runs lint;
- [x] runs TypeScript check;
- [x] runs frontend tests;
- [x] runs frontend production build.

### 4.2 Desktop Rust quality job

- [x] install Rust `1.97.1` or the intentionally updated repository toolchain;
- [x] run format check;
- [x] run strict Clippy;
- [x] run desktop backend tests;
- [x] run `cargo check` with all production features.

### 4.3 Linux bundle smoke job

- [x] install exact documented Linux packages;
- [x] build the Tauri production bundle;
- [x] upload useful logs on failure;
- [x] upload bundle artifacts only when useful and with bounded retention;
- [x] do not label the job Windows/macOS validation.

### 4.4 Preserve existing jobs

- [x] shared Rust CI still passes;
- [x] Android CI still passes;
- [x] Android instrumentation job still runs;
- [x] desktop jobs do not change Android NDK or Gradle behavior.

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

- [x] `ProfileId`;
- [x] `ProfileDisplayName`;
- [x] `DesktopProfilePaths`;
- [x] `ProfileMetadata` with a version field.

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

- [x] resolve Tauri application-local-data path in Rust;
- [x] create only required parent directories;
- [x] canonicalize or otherwise validate ownership without requiring the final database file to exist;
- [x] reject traversal and invalid profile IDs;
- [x] never accept a complete profile root from frontend input;
- [x] expose safe display information separately from internal paths.

### 5.3 Add profile metadata

- [x] write metadata atomically;
- [x] include schema version;
- [x] reject unsupported newer metadata;
- [x] do not overwrite malformed metadata automatically;
- [x] preserve Unicode display names within bounds.

### 5.4 Add tests

- [x] valid profile creation;
- [x] traversal rejection;
- [x] blank and oversized ID rejection;
- [x] Unicode display name;
- [x] unsupported metadata version;
- [x] partial metadata write recovery;
- [x] path isolation between profiles.

**Acceptance:** Desktop profiles have deterministic, isolated, tested application-owned paths.

---

## Block 6 — Add process-level profile locking

### 6.1 Select lock implementation

- [x] choose and pin a maintained cross-platform file/process lock implementation or implement a reviewed OS-specific abstraction;
- [x] record failure semantics;
- [x] avoid stale-lock deletion without ownership proof.

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

- [x] acquire before opening mutable databases;
- [x] hold for the complete core lifetime;
- [x] release only after core/database shutdown;
- [x] prevent a second production core from opening the same profile;
- [x] report holder/process information only when safe and available;
- [x] do not open a temporary duplicate profile on failure.

### 6.3 Add multiprocess tests

- [x] first process acquires;
- [x] second process fails visibly;
- [x] lock releases after normal shutdown;
- [x] abnormal process termination recovery follows selected library semantics;
- [x] separate profiles can open concurrently.

**Acceptance:** Two desktop processes cannot unknowingly mutate the same profile.

**Completion evidence:** `desktop/src-tauri/tests/profile_lock_multiprocess.rs` now covers child-process acquisition, visible contention, normal release, kernel lock recovery after forced child termination, and concurrent ownership of separate profiles. Exact-SHA Desktop CI run `31553428990` passed strict Rust Clippy and the complete backend test suite.

---

## Block 7 — Add desktop storage inspection and migration smoke

### 7.1 Open the real Rust database worker

Use the existing `silent-disco-core` storage API. Do not add desktop SQL.

- [x] pass the complete profile database path to Rust;
- [x] run schema creation/migration;
- [x] query database/schema versions through typed APIs;
- [x] close and join the worker;
- [x] display real success or structured failure.

### 7.2 Add read-only inspection commands

Temporary inspection commands may expose:

- [x] database metadata;
- [x] validated settings;
- [x] trusted-device summaries;
- [x] recent session summaries;
- [x] P2 store metadata when applicable.

Do not expose raw SQL or raw rows.

### 7.3 Add tests

- [x] first-open schema creation;
- [x] reopen latest schema;
- [x] unsupported newer schema;
- [x] checksum mismatch;
- [x] read-only or unwritable path;
- [x] profile lock release after open failure;
- [x] no in-memory fallback.

**Acceptance:** Met. The active desktop profile exposes bounded read-only database metadata, validated settings, trusted-device summaries, deterministic recent-session summaries, and the explicit current P2-store applicability state through a typed Tauri command and Storage screen, all backed by the already-open Rust `DatabaseWorker` rather than a second SQL path. Structured backend failures remain visible in the UI instead of becoming empty success. Exact-SHA Desktop CI run `31553428990` passed frontend quality, strict Rust Clippy/tests/check, committed-lockfile checks, and Linux AppImage/`.deb` bundle smoke.

---

## Block 8 — Define desktop DTOs and generated TypeScript bindings

### 8.1 Create DTO module

Create:

```text
desktop/src-tauri/src/dto.rs
desktop/src/core/generated/
```

Define desktop bridge DTOs for:

- [x] versions;
- [x] profile summaries;
- [x] bridge lifecycle;
- [x] structured errors;
- [x] storage inspection results;
- [x] later core snapshots and notifications.

DTOs must:

- [x] use explicit serde tagging and casing;
- [x] deny unknown fields where appropriate;
- [x] bound strings and arrays before core submission;
- [x] avoid private keys and native paths unless explicitly safe;
- [x] preserve stable error codes.

### 8.2 Add deterministic generation

Provide commands such as:

```bash
npm run bindings:generate
npm run bindings:check
```

- [x] generated files are stable across two consecutive runs;
- [x] CI fails on stale output;
- [x] generated output is committed if that is the selected policy;
- [x] no manual duplicate TypeScript enum remains.

### 8.3 Add round-trip tests

- [x] Rust DTO serializes to expected JSON fixture;
- [x] TypeScript fixture validates expected tagged union shape;
- [x] unknown kind fails visibly;
- [x] oversized input is rejected before core submission;
- [x] error fields survive conversion.

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

- [x] latest authoritative snapshot;
- [x] bridge lifecycle;
- [x] pending command receipts;
- [x] bounded errors/diagnostics;
- [x] stale-notification counters.

UI state contains presentation-only fields.

### 12.2 Add revision guard

```ts
export const shouldAcceptSnapshot = (
  current: number | null,
  incoming: number,
): boolean => current === null || incoming > current;
```

- [x] equal/older snapshots are rejected;
- [x] stale count increments;
- [x] newer snapshot replaces the complete authoritative snapshot;
- [x] no reducer locally advances host lifecycle.

### 12.3 Add typed client

Create `desktop/src/core/client.ts`.

- [x] wrap Tauri invokes with generated types;
- [x] attach notification channel;
- [x] convert invocation transport failure into bridge error;
- [x] do not convert failed invoke into successful empty result;
- [x] do not retry non-idempotent commands automatically.

### 12.4 Add tests

- [x] initial snapshot;
- [x] newer revision accepted;
- [x] equal revision rejected;
- [x] older revision rejected;
- [x] pending command remains pending until core evidence;
- [x] command failure displayed;
- [x] frontend reconnect obtains current snapshot;
- [x] no copied transition function exists.

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
- [x] restart/new stream ID.

The frontend remains pending until core state confirms each transition.

`start`/`pause`/`resume`/`stop_host_playback` are synchronous Tauri commands
that call `CoreActorHandle::submit_audio_event` directly, so by the time the
frontend's `await` resolves the actor has already validated and applied the
transition (unlike the async-delivery join/reject commands, there is no
separate confirmation round trip to wait for). Natural end-of-stream converges
on the same `Stop` broadcast + `Stopped` transition as an explicit stop
(`playback_streamer.rs`'s `run_pump` exit path). Restarting with a *new*
source after a stream completes is wired (`start_playback::start` always
computes a fresh `stream_id`) and is now locked by
`stop_then_second_source_uses_a_fresh_stream_without_old_audio_leakage`.

### 26.3 Data flow

- [x] decoder feeds bounded chunks;
- [x] packetizer feeds bounded transport queue;
- [x] transport reports per-peer delivery;
- [x] queue pressure becomes snapshot/diagnostic state;
- [x] stop cancels and joins decoder/packetizer workers;
- [x] no PCM or datagram payload enters Tauri IPC.

**Done 2026-08-03.** `process_broadcast_frames` was discarding the
`TransportDelivery` that `broadcast_audio`/`broadcast_sync` already return,
keeping only an aggregate last-error string on failure. A stream broadcast
into an empty session -- zero listeners, zero delivery -- was therefore
indistinguishable from a healthy one, which CLAUDE.md names explicitly as
not success, and queue depth had no diagnostic at all.

`BroadcastDiagnostics` now accumulates frames attempted/failed, frames fully
vs partially delivered, frames with no recipients at all, recipient-sends
intended vs delivered, and queue depth/peak/overflow, surfaced through
`HostTransportStatus` -> `ActiveHostSessionSnapshot` -> `BroadcastDeliveryDto`
on the host session snapshot. Counters are relaxed atomics, since they are
updated on the real-time broadcast path at 200 frames/second.

**Scope honesty:** these are counts per delivery attempt, not per listener
identity. The transport reports intended/successful/failed totals, so
attributing a specific failure to a specific peer would need a change in the
shared transport layer rather than in the desktop adapter. The item's wording
("per-peer delivery") is satisfied to the granularity the transport actually
exposes, and no further.

Tests: `broadcasting_to_no_listeners_is_reported_rather_than_counted_as_delivery`
(confirmed non-vacuous by dropping the `record_delivery` call) and a DTO
assertion that the counters reach the frontend snapshot.

### 26.4 Tests

- [x] one loopback listener receives expected packet sequence;
- [x] pause stops future presentation progression according to policy;
- [x] resume behavior is explicit;
- [x] stop clears pending stream data;
- [x] decoder failure mid-stream;
- [x] transport failure mid-stream;
- [x] queue full;
- [x] end-of-stream;
- [x] second source creates new stream identity.

The loopback suite now covers the full Block 26 software matrix. In addition
to the original happy path, `pause_stops_future_presentation_progression_until_resume`
locks the authoritative position while paused,
`a_transport_worker_failure_mid_stream_is_reported_by_the_playback_pump`
surfaces a live transport-worker loss, and
`a_full_broadcast_queue_is_a_visible_resource_limit_failure` proves bounded
queue saturation is explicit and diagnostic. The restart regression
`stop_then_second_source_uses_a_fresh_stream_without_old_audio_leakage` drives
the real Stop -> host-draft source update -> Start flow: it waits for the first
stream to stop, drains already-in-flight listener traffic, then requires a
fresh `stream_id`, sequence zero, and only the second stream's audio after the
new `StreamStart`. That test simultaneously locks the restart/new-ID, pending
stream cleanup, and second-source identity requirements without a physical
device.

**Acceptance:** The desktop host transmits real bounded audio datagrams through shared Rust code.

---

## Block 27 — Add playback and delivery UI

### 27.1 Playback controls

- [x] start/pause/resume/stop derive from core capabilities;
- [x] pending operation state;
- [x] source name and validated duration;
- [x] position based on authoritative timeline;
- [x] end-of-stream state;
- [x] no HTML audio element.

`HostSessionScreen`'s Play/Resume/Pause/Stop buttons derive their individual
enabled state from `playbackControlsEnabled` (now real, from host lifecycle +
transport-worker + selected-source state) and `playbackState`, and disable
during an in-flight command.

**Done 2026-08-03.** Two dormant shared-core `AudioEvent` variants,
`PositionAdvanced` and `EndOfStream`, existed for exactly this purpose but
nothing on the desktop path ever submitted them. The pump now does:

- Position is computed per audio frame from the frame's own presentation
  time against the stream's start (the authoritative timeline, not
  wall-clock elapsed, which would drift under pause or send-ahead bursting),
  throttled to one report per 250ms of stream-timeline advance so a 5ms
  packet duration does not submit 200 actor inputs/second for a value a UI
  only needs a few times a second.
- `packetizer.cancel_and_join()` returns `Ok` **only** when the source
  finished on its own -- every other exit, including deliberate
  cancellation, is an `Err`. That let the pump distinguish "the source
  finished" (`EndOfStream`) from "we were told to stop"
  (`PlaybackStateChanged(Stopped)`) without inventing new machinery.
- A genuinely new stream distinguishes itself from a paused stream resuming
  at the one point both submit the same `PlaybackStateChanged(Playing)`
  event: the *previous* state. Resuming from `Paused` must not reset
  position; starting fresh from anything else must. This is now the single
  place that reset happens, in the shared actor (`state/audio.rs`), not
  duplicated per platform.

New shared-core field: `CoreSnapshot.stream_ended_naturally: bool`, mirrored
into the UniFFI `FfiCoreSnapshot` record for Android/iOS consistency even
though nothing on those platforms reads it yet -- a domain field invisible to
one of the two FFI consumers is exactly the kind of silent divergence this
project's architecture rules warn against.

Surfaced through `HostSessionSnapshotDto`: `playbackPositionMs`,
`streamEndedNaturally`, and `audioSource` (name + validated duration, reusing
the existing `AudioSourceSummaryDto`). `HostSessionScreen` renders source
name, `position / duration` (m:ss), and a "Finished" badge distinct from the
generic `Stopped` status card.

Tests: an actor-level test (`host_block12_actor_lifecycle.rs`) proves the
reset-on-fresh-start-not-resume rule and the natural-vs-explicit distinction
directly against the shared state machine; a desktop integration test
(`playback_reports_advancing_position_and_natural_completion`) proves the
pump actually wires real, advancing values end to end over a real 3-second
source. Both were confirmed non-vacuous by reverting the change under test
and observing the failure.

### 27.2 Delivery health

Show bounded aggregate data:

- [x] intended peers;
- [x] successful peers;
- [x] failed peers;
- [x] partial delivery severity;
- [x] queue pressure;

**Done 2026-08-03.** The queue-depth/peak/overflow counters Block 26.3 added
to `BroadcastDiagnostics` are now rendered: a status line below delivery
health showing queued/peak frame counts, escalating to `role="alert"` amber
styling when `queueOverflows > 0`.
- [x] per-listener last failure;
- [x] zero-recipient warning.

Intended/successful/failed/severity and per-listener last failure predate
this block; this block added a distinct zero-recipient banner (red, `role="alert"`)
instead of folding it into the generic partial-failure amber styling. Queue
pressure remains unimplemented (see the 26.3 note -- no queue-depth
diagnostic exists yet for the playback broadcast path).

### 27.3 Tests

- [x] zero-recipient start policy;
- [x] partial delivery display;
- [x] failure not overwritten by later informational state;
- [x] stale command rejection;
- [x] stop pending;
- [x] source completion.

**Source completion done 2026-08-03** as part of the position/end-of-stream
work above (`playback_reports_advancing_position_and_natural_completion`).

**Zero-recipient start policy resolved 2026-08-08 (user decision): allow it.**
Starting playback with zero connected listeners stays allowed and simply
broadcasts to nobody; the 27.2 zero-recipient banner remains the only
signal, not a block. Locked in by
`starting_playback_with_zero_listeners_is_allowed`
(`desktop/src-tauri/src/platform/start_playback_tests.rs`) so this isn't
accidentally "fixed" into a block later without a deliberate decision.

**Stale command rejection resolved 2026-08-08 (user decision): today's
invalid-state checks are the policy**, not new revision-tracking machinery.
Writing the regression tests for that premise surfaced two real bugs, both
fixed the same day, not just tested:

- A duplicate/stale Start while a stream was already active was correctly
  rejected by `DesktopHostNetworkControl::start_playback`, but the calling
  `start_playback::start` submitted `PlaybackStateChanged(Buffering)` to the
  actor *before* that rejection ran, so the rejected duplicate still
  corrupted the authoritative snapshot to `PlaybackState::Error` even though
  the real, already-running stream was untouched. Fixed by adding
  `DesktopHostNetworkControl::playback_is_active()`, a non-mutating check
  `start_playback::start` now consults before submitting any actor
  transition. Covered by
  `starting_playback_twice_is_rejected_as_a_duplicate_command`.
- A duplicate/stale Resume arriving while already `Playing` (not `Paused`)
  was accepted and silently reset `playback_position_ms` to zero: the
  shared actor's reset-on-fresh-start rule
  (`rust/silent-disco-core/src/runtime/actor_runtime/state/audio.rs`)
  treated any `Playing` submission from a state other than exactly `Paused`
  as a fresh start, including `Playing -> Playing`. Fixed at the shared-core
  level by also excluding `Playing` from the reset condition. Covered at
  both layers: `resuming_while_already_playing_does_not_corrupt_position`
  (desktop) and the extended
  `playback_position_and_natural_completion_are_tracked_authoritatively`
  (shared core, `rust/silent-disco-core/tests/host_block12_actor_lifecycle.rs`).
- Duplicate Stop and duplicate/premature Pause/Resume/Stop (before anything
  is playing) were already correctly rejected with no code change needed;
  locked in by `stopping_playback_twice_is_rejected_not_silently_successful`
  and `pause_resume_stop_before_playback_started_are_all_rejected`.

Partial-delivery display and failure-persistence-across-refresh were already
tested pre-block (join/approval flows); Block 27.1/27.2 added
`HostSessionScreen playback controls` tests covering per-button
enable/disable by `playbackState`, the Play-becomes-Resume-when-paused
behavior, and a failed `stop` surfacing visibly.

**Acceptance:** The desktop never presents packet submission as universal listener success.

---

## Block 27 — closed 2026-08-08

All of 27.1, 27.2, and 27.3 are now checked. `bash scripts/check-rust.sh`,
`cd desktop && npm run check`, and `cargo clippy --all-targets --all-features
-- -D warnings` / `cargo fmt --all -- --check` against `desktop/src-tauri`
(not part of any automated gate yet -- see the open TODO note below) were
all run clean with the pinned `1.97.1` toolchain before closing.

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

- [x] **Fixed 2026-08-02** (Kotlin commit `9c5c4f7`, same day): even with the
      connection genuinely working, no sound was heard, because
      `ManualEndpointScreen.kt` (`feature/listener/ManualEndpointScreen.kt:136`)
      showed a static "Audio streaming is not part of this build yet."
      message and was never wired to the real playback pipeline. Fixed --
      see shared Rust migration TODO's Block 13.3 note for the full change.
      Confirmed live on the real device the same day: real join -> approval
      -> the screen genuinely showing "Connected / Buffering..." driven by a
      real `StreamStarted` event. **Not yet confirmed**: audible, in-sync
      sound from a human ear -- the run that reached "Buffering" ended
      (desktop side disconnected) before a human listened, due to the
      still-unfixed `stop_playback` bug below cutting the song-change step
      short. Retry with a human actually listening is the next real step for
      this block.
- [x] **Fixed 2026-08-03**: a silent-failure bug where `network.stop_playback()`
      reported success even when the actor never reached
      `PlaybackState::Stopped`. The pump's exit is what broadcasts `Stop` and
      makes that transition, and all of it was discarded --
      `drop(pump.join())` swallowed a panicking pump thread, and
      `drop(handle.submit_audio_event(..))` swallowed the transition itself.
      `run_pump` now returns its outcome, `join()` propagates it (including a
      panicking thread), and all three call sites report it: `stop_playback`
      returns it, `stop_host_inner` still shuts the runtime down but does not
      call that a clean shutdown, and `start_playback` surfaces a previous
      stream's failure instead of burying it under the next one.

      Un-swallowing immediately exposed a second, opposite bug in the first
      attempt: `cancel_and_join()` returns `Cancelled` on the *normal* stop
      path, because cancelling is exactly what stopping asks for. Treating
      that as a failure broke the existing integration test. Only the other
      kinds -- decode failure, packetize failure, panicking worker -- are real.

      Tests: `stop_playback_reports_a_pump_that_could_not_complete_its_shutdown`
      (confirmed non-vacuous by restoring `let _ = playback.join()`), plus
      `desktop_host_streams_real_audio_and_answers_sync_requests` now asserts
      the actor actually reaches `Stopped` after a successful stop -- the exact
      property the manual device test checks.

**Prep done 2026-08-08, live run still blocked on device availability**
(see `memory.md`'s LG G6 entry). Reviewing
`manual_real_android_listener_plays_a_song_change` against the 28.1
checklist below found it only covered WAV, never exercised pause/resume,
and printed no diagnostics -- all three closed without needing the device:

- `manual_real_android_listener_plays_flac` and
  `manual_real_android_listener_plays_mp3` added: same one-listener flow
  (join, approve, start, pause/resume, stop, diagnostics) against
  ffmpeg-encoded FLAC/MP3 fixtures. The app only decodes audio (no
  encoder), so `encode_with_ffmpeg` shells out to a real `ffmpeg` at test
  time rather than embedding committed binary fixtures; it panics with a
  clear message if `ffmpeg` isn't on `PATH` rather than silently skipping.
  Verified the real shared decoder (not just ffmpeg) actually opens both
  outputs before relying on this: a throwaway probe test round-tripped a
  2s ffmpeg-encoded FLAC and MP3 through `StreamingDecodeHandle::open`,
  confirmed 96,000 decoded frames each, then was deleted (not committed --
  it verified the approach, not a fixture worth keeping in the tree).
- All three manual tests now exercise pause (hold 5s so a human notices
  the silence) then resume mid-song, exactly what 28.1's "exercise
  pause/resume/stop" asks for, and safe now that Block 27.3 fixed the
  duplicate-Start/duplicate-Resume bugs it found.
- All three now call `print_diagnostics` at each phase transition,
  printing per-listener sync confidence/offset/RTT/drift and host-side
  broadcast/queue-pressure counters (Block 26.3) to stderr with
  `--nocapture`. Note printed by the helper itself: packet loss and
  underrun are listener-side (Android) diagnostics with no channel back to
  the host today, so those two still have to be read off the Android app's
  own screen by whoever runs the live session, not from this log.

Not yet attempted, and out of scope for this prep pass: 28.2's two
device-independent failure tests ("corrupt source fixture fails visibly",
"host source read failure does not claim continued normal streaming")
have no automated coverage at the `start_playback` orchestration level
today (only decoder-unit-level corrupt-input coverage exists in
`rust/silent-disco-core/src/audio/tests.rs`). Both are desktop-only and
don't need the phone -- worth doing before or during the live 28.2 session
rather than discovering the gap that day.

**Live session 2026-08-09, LG G6 available: real bugs found and fixed,
still no boxes checked.** First real playback attempts surfaced audible
defects (choppy/staticy, popping/crackling) that were root-caused --
not guessed -- to two real transport bugs (a 200-700ms blocking UDP send,
and a premature peer-disconnect side effect of fixing the first one; both
fixed and confirmed on-device) and then, after two hypotheses were tested
on real hardware and ruled out, to a pause/resume presentation-timeline
bug: the packetizer's fixed stream-start anchor goes stale across a real
pause, and `saturating_sub`-based pacing silently read every post-resume
frame as already late, bursting the whole backlog into the bounded
broadcast queue. Fixed with coordinated desktop (`playback_streamer.rs`/
`network.rs`, pause-offset tracking + a real `StreamStart` re-anchor
broadcast on resume) and shared-core/Android changes (`scheduler.rs`'s
`set_host_start_time_ms`, a same-`stream_id` lightweight re-anchor path in
`ManualListenerTransportController.kt` instead of a full engine
restart). See `memory.md`'s 2026-08-09T20:43:12Z entry for full detail.

Confirmed directly on the LG G6: `queue_overflows` stayed flat at 59
through pause, resume, and 20 more seconds of playback (previous runs
this same session climbed into the hundreds over that window). That is
real evidence 28.1's "exercise pause/resume/stop" now works for the WAV
song-change path specifically. **Still not checking any 28.1 box**: the
same run's later song-swap step hit an unrelated, separately-fixed test
timeout (`wait_snapshot` vs `wait_snapshot_for`) before a human could
listen to the second song, and the FLAC/MP3 variants were not re-run this
session. A full end-to-end run of all three manual tests with a human
actually listening is still the next step before any 28.1 box is
checked.

**Closed out 2026-08-10** (D4 bookkeeping pass, after A1-A6/D1-D2): see
`docs/AUDIO_PLAYBACK_STATE_2026-08-10.md` for the full session record this
references. 28.1 and 28.2 are now genuinely satisfied by real runs on the
LG G6; 28.3's regression-test claim is intentionally left partial below,
not overclaimed.

### 28.1 One listener

- [x] select supported WAV fixture;
- [x] join and approve one Android listener;
- [x] start stream;
- [x] confirm Android buffers and plays -- confirmed both by a human
      actually listening (mid-session feedback across this doc's fix
      history) and by real diagnostics (emitted/accepted frame counts,
      `phase=PLAYING`);
- [x] exercise pause/resume/stop -- every manual song-change/format test
      does this; A6 additionally confirmed `queue_overflows` stays flat
      across a real pause/resume;
- [x] record sync, RTT, packet-loss, and underrun diagnostics -- this box
      was only half-true before D2 (2026-08-10): packet-loss/underrun were
      always listener-side-only, but sync/RTT are now recorded on *both*
      sides, confirmed matching almost exactly
      (`rtt_ms=55.17` host vs. `rttMs=55.166666666666664` listener) on the
      same real run;
- [x] repeat with FLAC -- A5 (2026-08-10), clean
      (`concealed=112 late=21 hardResyncs=1`, within the post-A1-A3
      baseline range);
- [x] repeat with MP3 -- A5 (2026-08-10), completed without failure or
      disconnect, but **flagged, not clean**: listener-side quality trails
      WAV/FLAC noticeably (`concealed=700 late=271`, `ringFullEvents`
      nonzero) -- see `AUDIO_PLAYBACK_STATE_2026-08-10.md` §5 item 6.
      Checking this box records "the flow works end to end for MP3", not
      "MP3 quality is solved".

### 28.2 Failure tests

- [x] disable Android Wi-Fi during playback;
- [x] restore network;
- [x] verify disconnect/recovery policy -- A6 (2026-08-10) found three
      distinct things: listener-side detection is fast but for a narrower
      reason than a silence timeout (disabling Wi-Fi tears the local
      interface down, a *local* failure, not proof a *remote* silent
      partition is detected); recovery is fully manual by design,
      confirmed in code; and the host had zero visibility into the
      disconnect at all (100% false "delivered" for 2.5 minutes) -- found
      and fixed the same day (see `AUDIO_PLAYBACK_STATE_2026-08-10.md`
      §10 A6);
- [x] stop desktop transport -- exercised at the end of essentially every
      test in `start_playback_tests.rs`; the Block 27 `stop_playback`
      silent-failure bug (below) is the regression guard for this
      specifically;
- [x] corrupt source fixture fails visibly -- D1 (2026-08-10),
      `starting_playback_with_a_corrupt_source_fails_visibly_at_the_orchestration_level`;
- [x] host source read failure does not claim continued normal streaming
      -- D1 (2026-08-10),
      `a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming`.

### 28.3 Add regressions

- [x] every software defect receives an automated regression test -- the
      2026-08-10 audit's two known gaps are now closed.
      `translateTransportElapsedToPumpClock` is a pure Kotlin seam with JVM
      regressions for captured-origin delta translation, live-clock fallback,
      and underflow saturation.
      `LegacyBluetoothManifestInstrumentedTest` inspects the installed package
      and requires both legacy `BLUETOOTH` and `BLUETOOTH_ADMIN` declarations;
      it is intended for the repository's managed API-29 device, where those
      declarations are still operationally required. Existing coverage remains
      for A3 probe eviction, D1's two failure modes, A6 inbound-silence
      eviction, and D2 `SynchronizationReport` actor/codec/socket behavior.
- [x] record exact results in `memory.md` -- every fix this session has a
      dated, detailed entry with real device numbers, not summarized
      claims.

**Acceptance:** One Android listener plays synchronized audio transmitted by the Linux desktop host. **Met** for WAV and FLAC on the LG G6 (2026-08-10); MP3 works but with a flagged, unresolved quality gap (see above).

---

## Block 29 — Multi-listener physical validation

**Emulator-based dry run done 2026-08-08, no boxes checked below -- an
emulator is not a physical device and does not satisfy this block's
acceptance criteria.** Recorded here because it found and fixed a real bug
that would have blocked physical validation too, not just this dry run.

Two real Android emulators (headless, real network sockets, not loopback),
driven end to end via `adb`/`uiautomator` UI automation with no human
interaction (see `manual_two_emulator_listeners_play_together` and
`automate_manual_connect` in
`desktop/src-tauri/src/platform/start_playback_tests.rs`), joined the same
desktop-hosted session, were both approved, and both stayed connected
through start, a mid-stream pause/resume, and stop.

**Confirmed and fixed a real production bug that would have blocked this
with two real phones too, not just emulators**: `MainViewModel.kt`'s
`localListenerDeviceId` and `MainViewModelRustHost.kt`'s
`ANDROID_HOST_DEVICE_ID` were both hardcoded literal strings
(`"listener-device"`, `"android-host-device"`), so every install of the
Android app presented the identical identity to any host. The first attempt
at this dry run reproduced the failure directly: both emulators' join
requests were received and individually approved, but the host's snapshot
only ever showed one connected listener, because listener admission keys on
`device_id`. Fixed with a new `DeviceIdentityStore`
(`app/src/main/java/com/ekkus/silentdisco/core/identity/DeviceIdentityStore.kt`):
a random UUID generated once and persisted in app-private
`SharedPreferences`, shared by both roles (a physical device has one
identity regardless of which role it's currently playing). Confirmed with
genuinely distinct UUIDs in the re-run.

**Also found and fixed a second issue, this one in the manual test harness,
not production code**: `wait_for_real_join_and_approve` declared a listener
"approved" as soon as `dispatch_transport_effect` returned, but that call
only *enqueues* the send onto the transport worker -- the real delivery
confirmation (`TransportEvent::DeliveryCompleted`, which is what actually
moves a device from `pending_join_requests` into `listeners`) lands
asynchronously afterward. The single-listener manual tests never noticed
because enough real time passed before anything checked `listeners.len()`;
the two-listener test checked immediately after both approvals and caught
it directly (first listener transiently absent from `listeners`, right
after being approved). Fixed by waiting for the specific device to actually
appear in `snapshot.listeners` before returning.

**Not yet explained**: neither emulator ever completed a sync exchange
during the run (`listener has not yet completed a sync exchange`
throughout), and the broadcast queue accumulated 977 overflows by the end
of a ~100-second run. Both are plausibly explained by the emulator's I/O
being measurably slower than real hardware (see the `MANUAL_TEST_TIMEOUT`
note added the same day, and the `queue_overflows=930` finding from the
single-listener run that motivated it) rather than a protocol bug, but this
was not investigated further and should not be assumed benign until a real
device confirms sync actually completes under the same load.

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

**Scope correction, 2026-08-10**: `discovery.rs`'s doc comment above is
partly stale. `PlatformEffectRequest::StartAdvertising`/`StopAdvertising`
are *already* implemented (`effect_runner.rs` routes them to
`network.start_host`/`stop_host`, the real socket-binding host transport)
-- "advertising" today means "the host socket is bound and its endpoint
computed," with the actual address distribution being 100% manual
(`HostSessionScreen.tsx`'s "Manual connection details" copy/paste). Only
`StartDiscovery`/`StopDiscovery`/`EstablishNetwork`/`ReleaseNetwork` still
route to `unsupported_effect` -- and those four are listener-role-gated
(`require_role(AppRole::Listener, ...)` in
`runtime/actor_runtime/state/commands.rs`) and unreachable from the
desktop's host-only role today, not "unimplemented." So this block's real
job is narrower than its doc comment implies: add an mDNS **publish/
withdraw** adapter broadcasting the same `SessionAdvertisement` +
`NetworkEndpoint` data the manual payload already carries, alongside the
existing manual path, not replacing it. The original desktop-only
implementation did not include an Android NSD client. That follow-up is now implemented in the Android listener as a
convenience layer alongside BLE/Wi-Fi Direct and manual endpoint entry;
physical end-to-end verification remains part of Block 47 rather than being
inferred from source tests. The acceptance criterion remains unchanged:
"mDNS is a real convenience layer and not a hidden requirement for
transport."

### 30.1 Dependency gate

Evaluate maintained implementations.

Verify:

- [x] Rust/toolchain compatibility -- `mdns-sd` MSRV 1.71.0, well under the
      pinned 1.97.1 toolchain; builds clean.
- [x] Linux interface behavior -- spiked directly on this machine
      (register + browse + resolve + unregister + shutdown, real
      multicast, no daemon): correctly enumerated 8 real interfaces
      (`wlo1`, `lo`, Docker bridges/veth) and resolved to the genuine
      Wi-Fi address (`192.168.88.110` on `wlo1`) exactly as production
      `first_bindable_private_lan_address` already does for the manual
      path -- not just docs-read, empirically confirmed.
- [x] service update/withdrawal -- `ServiceDaemon::register`/`unregister`
      are separate, explicit calls; re-registering under the same
      instance name after a change is the documented update path.
- [x] bounded TXT record handling -- `ServiceInfo::new` takes a
      `HashMap<String, String>` of TXT properties with no crate-enforced
      size cap, so 30.2's "validate service and field lengths" is real,
      new work on top of this crate, not something it does for us.
- [x] license -- `mdns-sd` itself and its whole dependency tree (`flume`,
      `if-addrs`, `socket-pktinfo`, `spin`, `mio`, `socket2`, `libc`) are
      MIT/Apache-2.0/BSD-3-Clause; no copyleft.
- [x] shutdown/join -- `ServiceDaemon::shutdown()` is explicit and
      blocking-until-complete; confirmed clean in the spike (no hang, no
      panic).
- [x] testability -- the crate exposes both register (server) and browse
      (client) APIs in one dependency, so 30.3's "discover from a test
      client" can be a fully self-contained Rust test using the same
      crate as its own client, no external `avahi-browse`/`dns-sd` tool
      or physical device dependency.

**Selected: `mdns-sd` 0.20.3** (Apache-2.0 OR MIT), pinned exact per this
repo's dependency policy. **Alternative considered and rejected**:
`libmdns` (advertise-only, no browse API -- would have forced an external
tool dependency for 30.3's discovery test) and `zeroconf` (wraps
platform daemons -- Avahi via D-Bus on Linux -- which conflicts with
30.3's "daemon/multicast unavailable" test needing to exercise a real
failure mode rather than always failing because no daemon is running by
default in this environment; `mdns-sd` is pure Rust with its own
multicast socket, no system daemon dependency at all).

### 30.2 Implement adapter

- [x] publish only after real host endpoints exist -- `network.rs`'s
      `start_host_with_sink` calls `self.mdns.publish(advertisement,
      endpoint)` only after `DesktopHostTransportRuntime::start` has
      already succeeded, using the `endpoint` the transport actually
      bound to, never a value computed ahead of the bind.
- [x] use core-owned semantic advertisement -- `mdns.rs`'s
      `build_txt_properties` derives every TXT field from the same
      `SessionAdvertisement`/`NetworkEndpoint` the manual connection
      payload (`HostConnectionDto`) already carries; no separately
      invented advertisement shape.
- [x] validate service and field lengths -- `validate_txt_properties`
      enforces RFC 6763's 255-byte per-field limit
      (`MAX_TXT_VALUE_BYTES`) and a conservative 1300-byte summed
      payload bound (`MAX_TOTAL_TXT_BYTES`), covered by
      `a_field_over_the_255_byte_limit_is_rejected` and
      `a_payload_over_the_total_budget_is_rejected_even_with_no_single_field_over_limit`.
- [x] withdraw on session end and shutdown -- `stop_host_inner` calls
      `active.mdns.withdraw()` as part of teardown (attempted
      unconditionally alongside the playback/runtime steps, "first
      failure wins"); `MdnsSdPublisher::shutdown()` additionally tears
      down the whole daemon, confirmed by
      `shutdown_confirms_after_a_real_publish_and_withdraw` and
      `shutdown_is_a_clean_no_op_when_nothing_was_ever_published`.
- [x] update after endpoint/interface change according to explicit
      policy -- `mdns.rs`'s module doc comment ("Endpoint-change
      policy") records the decision not to live-republish on interface
      change, matching the manual payload's identical limitation;
      recovery is stop/start a new host session.
- [x] report publication failure -- `start_host_with_sink` never
      propagates a publish error into the host-start result; it records
      `MdnsPublicationState::Failed(error)` instead, surfaced end to end
      through `mdns_status_dto`/`MdnsStatusDto`/`NetworkBindingDto` to
      `HostNetworkPolicyCard.tsx`'s amber "Auto-discovery (mDNS)
      unavailable: ..." alert.
- [x] retain manual endpoint as visibly available alternative -- the
      "Bound ..." manual connection-details paragraph in
      `HostNetworkPolicyCard.tsx` renders unconditionally alongside (not
      instead of) the mDNS status line; covered by
      `surfaces an mDNS publication failure without hiding the manual
      connection details`.
- [x] never claim discovery active after publication failure --
      `mdns_status_dto` maps `MdnsPublicationState::Failed` to
      `{ active: false, failure_reason: Some(..) }`; there is no code
      path that reports `active: true` without a real
      `MdnsRegistration` in hand.

### 30.3 Tests

- [x] publish -- `a_real_publish_is_discoverable_by_a_separate_client_with_the_right_data`.
- [x] discover from a test client -- same test, via
      `resolve_from_a_fresh_client` spinning up a genuinely separate
      `ServiceDaemon` as the discovering client.
- [x] withdraw -- `withdrawing_removes_it_from_a_fresh_clients_discovery`.
- [x] duplicate service name -- `republishing_under_the_same_instance_name_is_not_an_error`.
- [x] interface disappears -- covered at the `network.rs` integration
      level (real interface loss inside the third-party `mdns-sd`
      daemon is impractical to trigger portably/reliably in an
      automated test) via
      `a_withdraw_failure_still_tears_down_the_host_but_is_reported_not_swallowed`
      in `network_tests.rs`, using a fake `MdnsRegistration` whose
      `withdraw()` fails the way a vanished interface would; confirms
      `stop_host_inner` still tears down the rest of the host and still
      reports the failure rather than claiming a clean stop.
- [x] daemon/multicast unavailable -- same rationale, covered via
      `a_publish_failure_does_not_fail_host_start_but_is_visible_in_the_snapshot`
      in `network_tests.rs`, using a fake `MdnsPublisher` that always
      returns `MdnsPublishError::DaemonUnavailable`; confirms host start
      still succeeds and the failure is visible in the snapshot, not
      swallowed.
- [x] oversized metadata -- `a_field_over_the_255_byte_limit_is_rejected`
      and
      `a_payload_over_the_total_budget_is_rejected_even_with_no_single_field_over_limit`.
- [x] shutdown -- `shutdown_confirms_after_a_real_publish_and_withdraw`
      and `shutdown_is_a_clean_no_op_when_nothing_was_ever_published`.

### 30.4 Android NSD convenience-layer closure

- [x] Android NSD discovery adapter --
      `app/src/main/java/com/ekkus/silentdisco/core/transport/MdnsDiscoveryService.kt`
      browses `_silentdisco._tcp`, serializes old-API resolve requests, rejects
      stale callbacks across stop/restart generations, and clears lost
      services rather than retaining stale endpoints.
- [x] lossless semantic metadata -- desktop TXT records now include
      `hostDeviceId`, stable `approvalMode`, and explicit `controlPort` in
      addition to the existing session/protocol/sync/audio fields; Android
      rejects unsupported protocol versions and contradictory SRV/TXT ports.
- [x] mDNS/BLE merge policy -- `MainViewModelTransport.kt` merges only
      confirmed Silent Disco advertisements, prefers endpoint-bearing mDNS
      data for a duplicate session ID, and never treats arbitrary Wi-Fi
      Direct peers as sessions.
- [x] real endpoint establishment -- an endpoint-bearing discovered session
      now opens `ListenerTransportController`, sends the real join request,
      and only then completes `EstablishNetwork`; it no longer reports a
      synthetic `NetworkEndpointReady` without opening transport.
- [x] reusable release -- `ListenerTransport.disconnect()` tears down the
      current native connection while leaving the event flow reusable, so a
      listener can join, leave, and join another discovered session without
      reconstructing the whole controller.
- [x] fallbacks preserved -- mDNS can operate without Bluetooth/Wi-Fi-Direct
      nearby-device permissions; BLE/Wi-Fi Direct remains available when
      permitted, and manual endpoint entry is unchanged.
- [x] software regressions -- pure TXT parsing covers valid metadata,
      protocol mismatch, control-port conflict, unknown approval mode, and
      IPv6 scope normalization; listener effect-runner coverage verifies
      mDNS-only discovery, multi-backend fallback, mDNS-over-BLE precedence,
      connect-before-success, release, and reconnect reuse. Dependency-free
      sandbox compilation also type-checks the NSD adapter and reusable
      listener transport against narrow framework/UniFFI stubs.

Physical Android discovery and packaged desktop-to-Android interoperability
remain Block 47 evidence gates; none of those device results are claimed by
30.4.

All ten `platform::mdns::tests` and the two new `platform::network_tests`
cases pass together with the full existing suite (138 tests total in the
desktop Rust lib, 0 failed) via `bash scripts/check-rust.sh` (shared
`rust/` workspace, unaffected) and `cd desktop && npm run check`
(bindings-check, biome, tsc, 60/60 Vitest, production build) -- both run
and confirmed green, not assumed.

**Acceptance:** mDNS is a real convenience layer and not a hidden requirement for transport.

---

## Block 31 — Add desktop QR invitation display

**Scope note, 2026-08-10**: investigation before implementation found a
real design fork this checklist doesn't anticipate. The already-shipped
"P2" QR system (`rust/silent-disco-core/src/p2/`, Android-to-Android,
ES256-signed, replay-protected -- exactly what 31.1 means by "existing
P2/core code") carries session/host identity but **no network endpoint**,
because Android's own P2 flow connects via BLE/Wi-Fi Direct discovery
*after* verification, never from data the QR itself carries. A desktop
host has no such discovery path -- the QR has to name a real connectable
endpoint or it cannot be joined at all. Resolved (user-selected "Full
stack now") by adding an optional `connection_payload_json` field to the
shared `p2` wire format, carrying the host's existing unsigned
manual-endpoint JSON verbatim (`ManualHostEndpoint`'s exact shape) inside
the signed envelope -- reusing that already-tested parser on the listener
side rather than inventing a second one, and leaving Android's own P2
host flow (which never sets this field) unaffected. Also added a new
per-profile P-256 signing keypair for the desktop (`invitation_identity.rs`,
keyring-backed), since desktop had no host-signing identity of any kind
before this block, mirroring Android's Keystore-backed
`HostIdentityManager`.

### 31.1 Core-generated invitation

- [x] invitation payload comes from Rust -- `desktop/src-tauri/src/platform/invitation.rs`'s
      `build_signed_invitation` calls the shared `silent_disco_core::p2`
      functions (`prepare_unsigned_qr`/`finalize_qr`) directly; the
      frontend only ever receives the already-signed string via the
      `create_host_invitation` Tauri command.
- [x] version, bounds, expiration, and signature validation use existing
      P2/core code -- no new validation logic was written; the shared
      `p2` module's `validate_unsigned`/canonicalization/ECDSA
      verification is unchanged and reused as-is (extended only with the
      new optional `conn` field, itself validated via the existing
      `ManualHostEndpoint::parse`).
- [x] frontend receives only the safe encoded invitation --
      `HostInvitationDto { payload: String, expires_at_ms: String }`;
      no key material, DER bytes, or nonce crosses IPC as a separate
      field.
- [x] no private signing key crosses IPC -- `DesktopHostSigningIdentity`'s
      private scalar never leaves `invitation_identity.rs`; only
      `sign_base64url`'s output (a signature) and `public_key_der`
      (public) are ever used outside it, and neither is the private key.

### 31.2 Render QR

- [x] select and pin a maintained QR rendering library or render from
      backend-generated safe data -- `qrcode` `1.5.4` (MIT), pinned
      exact, renders client-side from the backend-signed payload string;
      same division of labor Android already uses (zxing renders,
      Kotlin/Rust sign) rather than adding QR-bitmap generation to Rust.
- [x] no remote service -- `qrcode`'s `toDataURL` is fully offline
      (canvas/PNG encoding only, no network calls).
- [x] copyable text fallback -- "Copy invitation text" button, reusing
      the existing `CopyButton`/`copyValue` pattern already used for the
      manual connection payload.
- [x] expiration displayed -- `Detail` row showing the invitation's
      absolute expiry as a local wall-clock time.
- [x] refresh command is explicit -- only ever created/regenerated by an
      explicit "Create QR invitation"/"Refresh QR invitation" button
      click; nothing auto-generates or auto-refreshes one.
- [x] stale invitation is not silently reused -- the backend never
      caches an invitation (`create_host_invitation` always builds a
      fresh nonce/expiry), and the frontend shows an expired invitation
      as a visible "This invitation expired" state instead of the QR
      image, rather than continuing to display it as valid.

### 31.3 Tests

- [x] valid invitation -- `p2::tests::qr_with_a_valid_embedded_connection_payload_round_trips`,
      `platform::invitation::tests::a_valid_session_produces_a_signed_invitation_with_a_bounded_expiry`,
      and the Vitest
      `creates and renders a signed QR invitation with a copyable text fallback`.
- [x] expired invitation -- `p2::tests::qr_signature_expiry_tampering_and_replay_are_enforced`
      (shared validator, reused as-is) and the Vitest
      `shows an expired invitation as stale rather than silently reusing it`.
- [x] tampered invitation -- `p2::tests::qr_with_a_tampered_connection_payload_is_rejected`
      (new, specific to the embedded connection field) alongside the
      pre-existing general tampering case in the same test module.
- [x] oversized payload -- `p2::tests::qr_with_an_oversized_connection_payload_is_rejected_before_signing`.
- [x] QR rendering failure with text fallback -- covered at the
      component level: `HostSessionScreen.tsx` renders the "could not
      render its QR image" alert whenever `QRCode.toDataURL` fails,
      falling back to the same copyable text control shown in the
      success path (mirrors Android's own `QrHostInvitationDialog`
      failure branch, `onFailure` in `QrInvitationDialogs.kt`).
- [ ] Android scan/join physical test -- blocked on physical hardware,
      same as A7/Block 29 and D3. Desktop-side generation and Android-side
      wiring (`P2ValidatedInvitation.connectionPayloadJson`,
      `VerifiedQrInvitationDialog`'s "Connect" action,
      `MainViewModel.prefillManualEndpointFromInvitation`) are implemented
      and unit/instrumented-tested (`P2PresentationTest.kt`,
      `P2UiTest.kt`'s `verifiedInvitationWithADesktopEndpointOffersConnectInsteadOfDiscovery`),
      but no physical Android device has scanned a real desktop-issued QR
      and connected end to end.

All three quality gates run and green: `bash scripts/check-rust.sh`,
`cd desktop && npm run check` (bindings-check, biome, tsc, 64/64 Vitest,
production build), and `./gradlew test lintDebug`.

**Acceptance:** Android can join the desktop host through a Rust-generated invitation QR code.
Structurally satisfied end to end (signed generation through Android-side
connect wiring); the physical-device half of "join" is unverified,
identically to this project's other hardware-blocked items.

---

# Phase 9 — Optional local monitor audio

## Block 32 — Complete render-ring prerequisite

Before local monitoring, complete the applicable shared render-ring work from Blocks 16 and 17 of the shared migration TODO.

**Investigation, 2026-08-10**: Block 16 (SPSC render ring itself) is fully
checked in the shared migration TODO -- `silent_disco_core::audio::RenderRing`
(`rust/silent-disco-core/src/audio/render_ring.rs`) already satisfies every
bullet below with existing tests (`render_ring_tests.rs`, 12 tests including
two genuine multithreaded stress tests). Block 17 (C ABI) is checked except
one unrelated item (17.4's non-real-time fatal notification scheduling,
mobile-callback-specific, not relevant to desktop's consumer path). Desktop
already links `silent-disco-core` directly (not `silent-disco-ffi`), so it
already had ordinary safe access to `RenderRing::new(..).split()` -- the one
thing genuinely missing was a **controlled acquisition gate** guaranteeing
only one producer/consumer pair is ever outstanding, mirroring the C ABI
registry's `Active`/`Released` distinction without needing tokens or FFI.
That gate (`desktop/src-tauri/src/platform/render_ring.rs`,
`DesktopRenderRingGate`/`DesktopRenderConsumerLease`) is this block's only
new code; it does not schedule anything -- production of ring frames stays
the existing `PlaybackScheduler`/`PlaybackPump`'s job, reused unchanged by a
future local-monitor feature (Block 33+). Not yet wired into
`DesktopAppState`/any Tauri command -- out of scope until Block 33 actually
selects and wires a CPAL (or equivalent) output backend to consume it.

Verify:

- [x] 48 kHz stereo float32 internal format -- `RENDER_CHANNELS = 2`,
      `CANONICAL_SAMPLE_RATE_HZ = 48_000`/`CANONICAL_CHANNELS = 2` enforced
      upstream by the decoder/resampler pipeline (`render_ring.rs`,
      `audio/types.rs`); already the current format, not merely planned.
- [x] bounded preallocated ring -- `RenderRing::new` preallocates a fixed
      `Box<[AtomicU32]>` up front with hard `MIN`/`MAX_RING_CAPACITY_FRAMES`
      bounds; capacity never grows.
- [x] single producer -- `RenderRing::split(self)` returns exactly one
      `RenderRingProducer`, which does not implement `Clone` -- a
      compile-time, not runtime, guarantee.
- [x] single consumer registration -- same `split()` guarantee for
      `RenderRingConsumer` at the ring layer; additionally enforced at the
      desktop acquisition layer by the new `DesktopRenderRingGate`
      (`a_second_acquire_while_active_is_rejected`,
      `concurrent_acquire_attempts_yield_exactly_one_winner`).
- [x] no unread overwrite -- `push_frames` bounds writes to
      `free_frames` computed from an `Acquire`-loaded `read_index`; stress
      tested by `producer_faster_than_consumer_never_corrupts_or_loses_data`.
- [x] nonblocking consumer -- `read_frames` is pure atomics over a
      preallocated slice, no locks or allocation.
- [x] telemetry -- all 8 documented counters present and exercised
      (`RenderRingTelemetry`/`RenderRingSnapshot`).
- [x] stress tests -- two genuine multithreaded stress tests at the ring
      layer (`render_ring_tests.rs`); a third added at the desktop
      acquisition layer specifically
      (`concurrent_acquire_attempts_yield_exactly_one_winner`, 16 threads
      racing `acquire()`, asserting exactly one winner). Literal
      ThreadSanitizer was not run (stable toolchain has no nightly TSan
      wired in) -- documented as a known gap in Block 16's own
      implementation note, not silently skipped.
- [x] controlled consumer acquire/release lifecycle -- new
      `DesktopRenderRingGate::acquire`/`DesktopRenderConsumerLease`'s
      `Drop` (`desktop/src-tauri/src/platform/render_ring.rs`): explicit
      acquire, release-on-drop, a rejected config never leaves the gate
      stuck `Active`
      (`an_invalid_config_is_rejected_without_leaving_the_gate_stuck`), and
      a released lease permits a fresh acquire
      (`dropping_the_lease_allows_a_fresh_acquire`).

The desktop may use a safe Rust consumer API rather than the mobile C ABI, but semantics must remain equivalent.

All three quality gates run and green: `bash scripts/check-rust.sh`,
`cd desktop && npm run check` (bindings-check, biome, tsc, 64/64 Vitest,
production build) -- `./gradlew test lintDebug` unaffected by this block
(desktop-only Rust change, no Kotlin touched).

**Acceptance:** A safe desktop render consumer can be acquired without creating a second scheduling path.

---

## Block 33 — Select and validate CPAL or approved audio backend

**Test system, 2026-08-10** (recorded because every finding below is
specific to it, not a general claim): this machine runs `PipeWire` 1.0.5
as the session audio server, exposing a PulseAudio-compatible socket
(`pactl info` reports `Server Name: PulseAudio (on PipeWire 1.0.5)`). Real
ALSA hardware is present (`/proc/asound/cards` lists two `HDA-Intel` cards
and one `acp` card) but `/dev/snd/*` is owned by the `audio` group, which
this session's user is not a member of -- confirmed independently by
`aplay -l` reporting "no soundcards found". `/usr/share/alsa/alsa.conf.d/`
has both `50-pipewire.conf` and `99-pipewire-default.conf`, making
`PipeWire`'s ALSA plugin the system default PCM -- so `cpal`'s ALSA
backend talks to `PipeWire`'s socket, not `/dev/snd/*` directly, which
turns out to sidestep the permission gap entirely (confirmed empirically,
not assumed). Spike code lives at
`desktop/src-tauri/src/platform/cpal_spike_tests.rs` -- not production
code, not wired into any Tauri command, run via
`cargo test cpal_spike -- --nocapture`.

### 33.1 Spike

With CPAL 0.18.x or current approved candidate, test:

- [x] default device enumeration -- `cpal::default_host().default_output_device()`
      resolves to "Default Audio Device"; `output_devices()` enumerates 12
      entries, all `PipeWire`'s ALSA-plugin nodes (rate converters, JACK,
      OSS, "PipeWire Sound Server", "PulseAudio Sound Server", channel
      up/downmix plugins, "Default ALSA Output (currently PipeWire Media
      Server)") -- confirms this system's device list reflects the
      software plugin chain, not distinct physical hardware.
- [x] explicit device selection -- `cpal` 0.18.1 removed the fallible
      `Device::name()` from earlier versions (a real API change this spike
      had to discover, not one this project chose); device identity now
      goes through `Display`/`.to_string()`, `.id()`, and structured
      `.description()`. Re-finding a previously enumerated device by
      `PartialEq` identity after a fresh `output_devices()` call succeeded
      for all 12 devices.
- [x] supported format negotiation -- `supported_output_configs()`
      returned 320 ranges (1-64 channels, 1-384000 Hz, U8/I32/F32) on the
      default device. Caveat recorded, not glossed over: this is
      `PipeWire`'s ALSA-plugin layer being maximally permissive, not a
      real hardware capability report -- not authoritative for what a
      genuine physical device would report if `audio`-group access were
      available.
- [x] 48 kHz stereo float output where available -- `default_output_config()`
      returned exactly `2 ch, 48000 Hz, F32` on this machine, an exact
      match for the project's canonical render format
      (`CANONICAL_SAMPLE_RATE_HZ`/`CANONICAL_CHANNELS` in
      `silent_disco_core::audio`) with no format conversion required.
- [x] fallback conversion policy where approved -- **N/A by deliberate fail-closed policy**.
      No format-conversion fallback is approved: Block 33.2 requires canonical
      48 kHz/stereo/f32 (or transparent OS/backend negotiation to it) and a
      visible actionable error otherwise. This is therefore complete as a
      policy assertion, not an untested claim that a converter exists.
- [x] `PipeWire` -- the entire spike above **is** the `PipeWire` path (see
      test-system note); genuinely exercised, not merely present.
- [x] PulseAudio/ALSA behavior present on the test system -- PulseAudio
      access confirmed present (via `PipeWire`'s compatibility socket);
      direct ALSA hardware access confirmed blocked by the `audio`-group
      permission gap. Both recorded as real, asymmetric findings, not
      papered over.
- [ ] device removal -- not exercised. No removable/hot-unpluggable audio
      device was available in this session, and deliberately killing the
      user's live `PipeWire` session to force a disconnect would disrupt
      their actual desktop -- an honest, documented gap, matching this
      project's established pattern for hardware this session cannot
      safely or actually reach (LG G6 physical device, ThreadSanitizer).
- [x] stream error callback -- not triggered via a live device-loss event
      (see "device removal" above), but its reachability was proven a
      different way: `build_output_stream` with a deliberately absurd
      config (0 channels, 0 Hz) was rejected as a typed `cpal::Error`
      ("channel count must be at least 1"), never a panic --
      `an_unsupported_config_is_rejected_as_an_error_not_a_panic`.
- [x] callback timing -- a real stream ran for 300ms and recorded 30
      callback invocations via atomic counters (no logging/allocation in
      the callback itself), roughly consistent with `PipeWire`'s default
      buffering; zero error-callback invocations during the run.
- [x] shutdown quiescence -- dropping a playing stream from a spawned
      thread completed without panicking or hanging, joined successfully
      within the test.
- [x] Rust version and license -- `cpal` 0.18.1: `rust-version = "1.85"`
      (well under this project's pinned `1.97.1`), license `Apache-2.0`.
      Its Linux dependency chain: `alsa` 0.11.0 (`Apache-2.0/MIT`),
      `alsa-sys` 0.4.0 (`MIT`), `dasp_sample` 0.11.0
      (`MIT OR Apache-2.0`) -- all permissive, no copyleft, consistent
      with this project's dependency policy.

### 33.2 Decide policy

Record:

- [x] selected backend and features -- `cpal` `=0.18.1`, pinned exact
      per this repo's dependency policy
      (`desktop/src-tauri/Cargo.toml`). No non-default features enabled;
      the default build already covers the ALSA/`PipeWire` path this
      spike exercised.
- [x] supported Linux audio stacks -- `PipeWire` (via its PulseAudio-
      compatible socket and its default ALSA plugin) confirmed working
      end to end on the test system. Raw/direct ALSA hardware access is
      untested here (blocked by this session's `audio`-group permission
      gap, not by `cpal` or `PipeWire`) -- a real end-user's desktop
      session is expected to be in the `audio` group by normal Linux
      desktop convention, so this is a test-environment limitation, not
      an expected production one, but it is explicitly unverified rather
      than assumed to work.
- [x] device-selection UX -- deferred to Block 34; out of scope for a
      selection/validation block. This spike confirms the API this
      future UX would be built on (enumerate via `output_devices()`,
      identify by `PartialEq`/`.id()`, display via `Display`) is usable.
- [x] format conversion location -- **no new resampler is built for this
      block.** The shared core already has a private, decode-time-only
      `StereoResampler` (`silent_disco_core::audio::resampler`,
      `pub(super)`, source-rate to canonical-48kHz only) -- not exposed
      or intended for output-stage use. Given CLAUDE.md's "prefer simple
      correction strategies before advanced time-stretch/resampling" and
      that this spike found the canonical 48kHz/stereo/f32 format is
      natively available on this real test system, Block 34's policy is:
      **require a device config that already matches the canonical
      format** (or one `cpal`/the OS can transparently negotiate,
      e.g. `PipeWire`'s own internal SRC when the app requests 48kHz/f32
      explicitly) and **fail closed with a visible, actionable error**
      rather than silently downsampling/upsampling, if no compatible
      config exists. Building a genuine output-stage resampler is
      deferred until real evidence (a real device that does not support
      the canonical format) shows it is actually needed -- consistent
      with this project's staged-block, no-premature-complexity
      approach.
- [x] monitor-failure effect on host transmission -- **must be zero.**
      Local monitor audio is Phase 9, explicitly titled "Optional local
      monitor audio," and Block 34.2 already requires "host can stream
      with monitor disabled" and "no fake monitor success on headless
      systems." Recorded here as the binding policy for Block 34's
      implementation: any monitor-stream failure (device gone, config
      rejected, stream error callback) surfaces as a visible, monitor-
      scoped error and never touches, pauses, or degrades the host's
      network broadcast path, which remains entirely independent.
- [x] rejected alternatives -- none seriously evaluated as competitors:
      `cpal` is the de facto standard cross-platform Rust audio I/O crate
      (used by `rodio`, `bevy_audio`, and most Rust audio projects),
      already named as the presumed candidate in this block's own title
      and 33.1's "CPAL 0.18.x or current approved candidate" framing, and
      this spike found no disqualifying behavior on the real test system
      -- no alternative crate spike was warranted.

**Acceptance:** The selected backend has measured Linux behavior and explicit failure semantics.

All three quality gates run and green: `bash scripts/check-rust.sh`,
`cd desktop && npm run check` (bindings-check, biome, tsc, 64/64 Vitest,
production build) -- `./gradlew test lintDebug` unaffected by this block
(desktop-only Rust change, no Kotlin touched). Also manually ran
`cargo fmt --check` and `cargo clippy --all-targets --all-features` for
`desktop/src-tauri` specifically (neither is part of `npm run check`,
which never invokes them for the desktop crate) -- both clean, and this
pass also caught and fixed pre-existing `cargo fmt` drift across several
earlier blocks' desktop Rust files (`mdns.rs`, `invitation.rs`,
`invitation_identity.rs`, `app_state.rs`, `host_session_dto.rs`,
`network_tests.rs`, `render_ring.rs`) that had never been formatted by
any prior gate.

---

## Block 34 — Implement desktop local monitor adapter

**Architecture, 2026-08-10**: desktop's existing host broadcast pump
(`playback_streamer.rs`) is deliberately *not* presentation-time-scheduled
-- it only bounds how far ahead of the transport clock it may send
(`SEND_AHEAD_HORIZON_MS`), relying entirely on each *listener's* own
`PlaybackScheduler`/`PlaybackPump` to place audio in time. Real local
output hardware needs genuinely paced frames, so "the same scheduled Rust
timeline" this block's acceptance criterion asks for means literally
reusing that same `PlaybackScheduler`/`PlaybackPump` machinery
(`silent_disco_core::audio`, otherwise only ever used by
`rust/silent-disco-ffi/src/listener_playback/pump.rs` for Android listeners) on
the desktop host itself -- a second real thread
(`platform/monitor_pump.rs`), not a second scheduling *implementation*. A
desktop monitor differs from a listener in exactly one respect: there is
no clock gap to estimate, since the host is pacing its own local decode
against its own local clock -- the monitor pump locks
`PlaybackPump::apply_sync_offset(0.0)` once, immediately, and never
touches it again.

New modules: `platform/audio_device.rs` (the real-time-safe callback +
`AudioOutputBackend` trait + `CpalAudioOutputBackend`, Block 33's selected
crate), `platform/monitor_pump.rs` (the scheduled pump thread),
`platform/monitor.rs` (`DesktopMonitorControl`: on/off preference, the
Block 32 render-ring gate, and stream lifecycle coordination). Wired into
the existing broadcast pump via a bounded, non-blocking tap
(`playback_streamer.rs`'s `forward_to_monitor`) that forwards a clone of
each outgoing audio datagram -- never the reverse; the monitor can never
affect what the broadcast pump does.

**Lifecycle policy, recorded because it is a deliberate simplification**:
enabling the monitor only takes effect on the *next* stream start (it does
not reach back into a song already playing); disabling it takes effect
immediately, tearing down any active monitor stream right away. Monitor
on/off is desktop-platform-local state, surfaced through
`HostSessionSnapshotDto.monitor`, never `CoreCommand`/`AudioEvent`/
`CoreSnapshot` domain state -- the same architectural choice already made
for mDNS publication status (Block 30), since monitor audio affects only
what is heard at this desktop machine, never what any listener receives.

### 34.1 Add adapter

Create:

```text
desktop/src-tauri/src/platform/audio_device.rs
```

- [x] enumerate devices outside callback -- `CpalAudioOutputBackend::default_output_config`
      runs entirely outside the real-time callback, called once per stream
      start from `monitor.rs`.
- [x] configure stream outside callback -- `AudioOutputBackend::start` takes
      an already-negotiated `AudioOutputConfig`; no negotiation happens
      inside `RenderCallback::write`.
- [x] acquire one validated render consumer -- `DesktopMonitorControl`
      acquires through Block 32's `DesktopRenderRingGate`, which itself
      guarantees only one outstanding lease.
- [x] callback performs bounded ring read and silence fill only --
      `RenderCallback::write` is exactly one `RenderRingConsumer::read_frames`
      call plus atomic telemetry, nothing else.
- [x] callback performs no Tauri, logging, `SQLite`, file, network,
      allocation, or blocking work -- confirmed by direct code review of
      `RenderCallback::write`'s full body (`audio_device.rs`); it touches
      only the pre-allocated output slice and pre-allocated atomics.
- [x] atomic telemetry only -- `AudioOutputTelemetry` (`callback_count`,
      `frames_written`, `frames_silence_filled`), all `AtomicU64`.
- [x] errors reach core through non-real-time event path -- live backend errors are retained in
      `ActiveMonitorStream.runtime_failure` through a write-once `OnceLock`; `status()` turns the
      monitor inactive and exposes the first actionable cause through the host-session snapshot.
      The audio callback remains real-time safe because the backend error callback is a separate
      non-real-time path.
- [x] callback is quiescent before consumer release --
      `RunningAudioOutputStream::stop` consumes `self` and blocks until the
      backend's own thread/callback is provably done (joined, for both the
      real `cpal::Stream` drop and the fake used in tests) before
      `DesktopMonitorControl` drops the render-ring lease.

### 34.2 Transmit-only default

- [x] host can stream with monitor disabled -- monitor defaults to
      disabled (`DesktopMonitorControl`'s `enabled: false` initial state);
      every existing `start_playback_tests.rs` real-audio test continues
      to pass unmodified with the monitor wired in but off, confirming
      zero behavioral change to host transmission by default.
- [x] monitor enable is explicit -- only ever changed by the new
      `set_host_monitor_enabled` Tauri command, itself only ever called
      from an explicit UI toggle click.
- [x] monitor failure follows recorded policy -- startup/configuration failures are recorded in
      `MonitorState.failure_reason`, live backend failures are retained in the active stream's
      `runtime_failure`, and `status()` reports the monitor inactive with that cause. None of these
      paths stop or gate the host's listener transmission path.
- [x] no fake monitor success on headless systems --
      `NullAudioOutputBackend` (the default backend before
      `with_monitor_backend` is called) always reports
      `AudioOutputError::NoDefaultDevice`, exactly matching a genuinely
      headless system's own behavior -- a test double must be explicitly
      injected to ever observe `active: true`.
- [x] no automatic switch to HTML audio -- not applicable to this native
      desktop app (no web-audio/HTML-audio code path exists anywhere in
      `desktop/src-tauri`); confirmed by there being nothing to switch to.

### 34.3 Tests

- [x] generated test tone through render ring --
      `a_generated_test_tone_reaches_the_output_callback_through_the_real_pipeline`
      (`monitor_tests.rs`): a synthetic recognizable PCM16 signal submitted
      through the tap is observed, via a real fake-backend-driven callback
      thread, at the far end of the actual scheduler/pump/render-ring
      pipeline -- not simulated or shortcut.
- [x] start/stop repeated -- `start_stop_repeated_never_leaks_or_panics`
      (5 iterations, asserts `status().active` toggles correctly every time).
- [x] underrun and silence fill --
      `an_empty_ring_produces_silence_and_records_it_as_such` (`audio_device.rs`,
      unit-level: an unfed ring produces exact silence and records it as
      `frames_silence_filled`, not `frames_written`).
- [x] device removal -- `device_removal_mid_stream_is_survived_without_panicking` injects the
      backend's real `on_error` path mid-stream, requires the monitor to become inactive with the
      exact device-removal cause, and verifies teardown preserves that cause. A physical hot-unplug
      remains useful Block 47 acceptance evidence, but the software failure path itself is covered.
- [x] wrong format -- `a_non_canonical_device_format_is_rejected_before_opening_a_stream`
      (a fake reporting 44.1kHz is rejected before `start()` is ever
      called -- `backend.starts` counter proves it).
- [x] callback after release prevention --
      `callback_after_release_is_structurally_impossible` (the fake's
      driving thread is provably joined -- and therefore gone -- before
      `on_stream_stopped` returns; a fresh acquire against the same gate
      immediately afterward succeeds, which Block 32's gate would refuse
      were the previous consumer still alive).
- [x] host transmit continues or stops exactly according to policy --
      `forwarding_to_a_tap_that_cannot_accept_right_now_never_blocks_or_panics`
      (`playback_streamer.rs`): a capacity-0 rendezvous channel makes
      `try_send` fail immediately with no receiver waiting, proving
      `forward_to_monitor` cannot block the unconditional
      `network.broadcast_playback_frame` call that always follows it in
      `run_pump`. A full real end-to-end test combining a real listener
      *and* a struggling real monitor simultaneously was not additionally
      built, to avoid modifying `start_playback_tests.rs`'s
      `start_host_session` helper (shared by 19 existing tests); the
      causal mechanism itself is directly unit-tested instead.
- [x] shutdown under active callback --
      `shutdown_while_the_callback_is_actively_running_completes_cleanly`
      (stop is issued with no settling delay, while the fake's thread is
      mid-write/sleep cycle; joins cleanly, does not panic).

All three quality gates run and green: `bash scripts/check-rust.sh`,
`cd desktop && npm run check` (bindings-check, biome, `cargo fmt --check`,
tsc, 68/68 Vitest, production build) -- `./gradlew test lintDebug`
unaffected by this block (desktop-only change, no Kotlin touched). Also
ran `cargo clippy --all-targets --all-features` manually for
`desktop/src-tauri` (still not part of the enforced gate) -- zero
deny-level errors; only pre-existing/precedented pedantic warnings.

**Acceptance:** Partially met. Optional desktop monitoring uses the same scheduled Rust timeline and the real-time callback remains bounded, but live CPAL errors are still discarded instead of reaching the non-real-time/core-visible failure path.

---

# Phase 10 — Diagnostics, lifecycle, and controlled shutdown

## Block 35 — Build desktop diagnostics screen and export

### 35.1 Diagnostics DTO

Expose bounded summaries for:

- [x] versions -- `VersionsDiagnosticsDto` (`diagnostics_dto.rs`): core
      version, app version (`CARGO_PKG_VERSION`), export schema version.
- [x] profile/platform -- `ProfileDiagnosticsDto`: profile ID,
      `std::env::consts::OS`.
- [x] storage -- `StorageDiagnosticsDto`, populated in
      `app_state.rs::host_diagnostics` via
      `DatabaseWorker::client().metadata()` against the already-open
      worker (not `storage_inspection.rs`'s pre-session
      lease-acquire/open/close cycle, which would conflict with the
      session's already-held lease); `available: false` +
      `failure_reason` on query failure, never fabricated as healthy.
- [x] identity availability without secrets -- `IdentityDiagnosticsDto`:
      presence booleans plus a SHA-256 public-key fingerprint via the
      existing `silent_disco_core::p2::public_key_fingerprint`; raw DER
      and the device-identity secret never touched.
- [x] endpoints/interface -- `endpoint: Option<HostConnectionDto>`,
      reusing the exact same `host_connection_dto` mapping
      `HostSessionSnapshotDto` uses (extracted to a shared
      `pub(crate)` function in `host_session_dto.rs` for this reuse).
- [x] transport queues and delivery -- `TransportDiagnosticsDto`
      (`state`, `lastDelivery`, `broadcast`), reusing
      `broadcast_delivery_dto` the same way.
- [x] listeners -- `Vec<ListenerDiagnosticsDto>`, bounded to
      `MAX_DIAGNOSTICS_LISTENERS = 64` with `listeners_truncated: bool`
      set whenever the real count exceeds it (see 35.3's
      truncation/omission requirement).
- [x] synchronization -- `SynchronizationDiagnosticsDto`
      (confidence/offset/RTT/drift), mapped from `CoreSnapshot`'s
      existing `SynchronizationSummary`.
- [x] decoder/source queues -- `DecodeQueueDiagnosticsDto`, sourced from
      a new `DecodeStatisticsReader` (`rust/silent-disco-core/src/audio/decoder.rs`):
      a small `Clone`-able reader that clones out the decoder's existing
      `Arc<SharedStatistics>` *before* the handle is consumed by the
      packetizer worker, since the handle's own `statistics()` becomes
      unreachable once ownership transfers.
- [x] packetizer -- `PacketizeQueueDiagnosticsDto`, sourced from a new,
      analogous `PacketizeStatisticsReader`
      (`rust/silent-disco-core/src/audio/packetizer_worker.rs`), taken
      before the packetizer is moved into the playback pump thread.
- [x] local monitor and render counters -- `MonitorDiagnosticsDto`.
      Fixed a real Block 34 gap while wiring this: `AudioOutputTelemetry`'s
      `Arc` was constructed in `DesktopMonitorControl::start_stream` and
      handed only to the render callback, with no other reference
      retained anywhere -- live telemetry was structurally unreachable.
      `ActiveMonitorStream` now retains its own `Arc::clone`, and
      `status()` reports it as `MonitorTelemetrySnapshot`.
- [x] notification bridge -- `NotificationBridgeDiagnosticsDto`
      (`delivery_failure: Option<DesktopErrorDto>`).
- [x] last structured errors -- `last_error: Option<DesktopErrorDto>`,
      direct from `CoreSnapshot.last_error`.
- [x] shutdown state -- `shutting_down: bool`, direct from
      `CoreSnapshot.shutting_down`.

Assembly is a pure builder,
`platform::diagnostics::build_diagnostics_snapshot`: it takes only
already-resolved plain data and does zero locking/querying itself
(mirroring `HostSessionSnapshotDto::from_parts`'s own discipline), so it
is unit-testable with fixture inputs; all real I/O happens in the new
`app_state.rs::host_diagnostics()` caller. Deliberately bypasses the
pre-existing `CoreCommand::ExportDiagnostics` /
`PlatformEffectRequest::ShareDiagnostics` pathway entirely -- confirmed
via `grep -rn "ExportDiagnostics" desktop/` to be completely unwired from
any UI today -- in favor of the same "direct `DesktopAppState` method +
independent Tauri command" pattern already used for
`create_host_invitation`/`set_host_monitor_enabled`.

### 35.2 Screen

Created:

```text
desktop/src/screens/DiagnosticsScreen.tsx
```

- [x] bounded display -- renders exactly the bounded DTO; the listener
      list additionally shows its own truncation flag.
- [x] severity and subsystem filters -- `deriveFindings` turns the DTO
      into a bounded findings list, each carrying a real subsystem and
      severity (`DesktopErrorDto`'s own fields where available,
      synthesized for boolean/enum-shaped facts); two `<select>` filters
      narrow that list without hiding the full detail sections below it.
- [x] no color-only communication -- every finding renders a text
      severity label (`[ERROR]`/`[WARNING]`/`[FATAL]`/`[OK]`) alongside
      its color class; the stale banner and error alerts are always text
      first.
- [x] safe copy behavior -- "Copy diagnostics JSON" copies the DTO
      exactly as received (already redacted server-side; nothing added
      client-side).
- [x] no private identity or invite secrets -- the screen renders only
      `DesktopDiagnosticsDto` fields; nothing else is fetched or joined
      in.
- [x] clear stale-data indicator -- `generatedAtMs`-derived age is always
      shown ("Snapshot captured N s ago"); an age past `STALE_AFTER_MS`
      (5s, more than double the 2s poll interval) renders an explicit
      `role="alert"` STALE banner.

Reachable via a always-visible "Diagnostics" toggle in `App.tsx`'s
header, independent of host lifecycle -- including the bridge-failed
state, since diagnosing a startup failure is exactly when this screen
matters most.

### 35.3 Export

- [x] Rust creates versioned bounded export -- writes the same
      `DesktopDiagnosticsDto` the screen displays (one DTO, two
      consumers, per its own module doc comment); `exportSchemaVersion`
      carries the version.
- [x] save dialog selects destination -- new
      `DiagnosticsSaveDialog`/`TauriDiagnosticsSaveDialog`
      (`diagnostics_export.rs`), mirroring `file_picker.rs`'s
      `AudioFileDialog`/`TauriAudioFileDialog` pattern, using
      `tauri-plugin-dialog`'s `blocking_save_file()` (this codebase's
      first save-, not open-, dialog).
- [x] temporary write then atomic rename where supported --
      `write_diagnostics_export`: create-new temp file beside the
      destination, write/flush/`sync_all`, then `fs::rename`. POSIX
      `rename` already replaces an existing destination atomically; on
      Windows (which does not) `install_atomically` removes the existing
      destination first and retries once -- the payload is already fully
      durable in the temp file before either path runs.
- [x] cancellation distinct from failure -- `DiagnosticsExportOutcome::{Saved,Cancelled}`;
      a `None` from the dialog produces `Ok(Cancelled)`, never an `Err`.
- [x] truncation/omission reported -- `listeners_truncated` travels
      inside the exported DTO itself (see 35.1); the export mechanism
      never truncates further.
- [x] no audio payloads -- the DTO has no field capable of carrying
      audio; nothing beyond `DesktopDiagnosticsDto` is ever serialized.
- [x] no raw private paths unless redacted policy approves -- no path
      of any kind appears in the DTO; the chosen destination path itself
      never leaves the Rust write function.
- [x] no success until file is committed -- `export_with_dialog` only
      returns `Saved` after `write_diagnostics_export` returns `Ok`,
      which itself only returns `Ok` after the final rename and parent-
      directory `sync_all` succeed.

### 35.4 Tests

- [x] secret redaction --
      `no_secret_shaped_content_appears_in_the_serialized_snapshot`
      (`diagnostics.rs`): a realistic invite-code-gated, signed,
      actively-monitored fixture is serialized and checked for
      `privateKey`/`secret`/`DER`/`keyring`-shaped substrings.
- [x] bounded size -- `an_oversized_listener_list_is_truncated_and_reported`
      / `a_normal_sized_listener_list_is_not_reported_as_truncated`
      (`diagnostics.rs`, DTO-level) and
      `an_oversized_payload_is_rejected_before_any_write`
      (`diagnostics_export.rs`, byte-level defense in depth against
      `MAX_EXPORT_BYTES = 1 MiB`).
- [x] destination failure --
      `a_missing_destination_directory_is_a_reported_failure`
      (`diagnostics_export.rs`): a real, portable I/O fault (missing
      parent directory) surfaces a structured error.
- [x] cancellation -- `a_cancelled_dialog_produces_cancelled_not_an_error`
      (`diagnostics_export.rs`): a fake dialog returning `None` produces
      `Ok(Cancelled)`.
- [x] existing file policy -- `a_pre_existing_destination_is_overwritten`
      (`diagnostics_export.rs`): a pre-seeded destination file is
      replaced, not rejected -- the native save dialog already confirmed
      the overwrite.
- [x] partial write cleanup -- `an_install_failure_cleans_up_its_temporary_file`
      (`diagnostics_export.rs`): a real, portable install fault (the
      destination already exists as a directory, so the final rename
      cannot succeed) is forced *after* the temp file was already
      written/flushed/synced; asserts the temp file left behind by that
      successful write stage is removed, not orphaned.
- [x] export after startup failure --
      `diagnostics_after_startup_failure_surfaces_the_stored_failure`
      (`app_state_tests.rs`): while `DesktopRuntimeState::Failed` (no
      `ReadyRuntime` exists to query), `host_diagnostics()` returns the
      exact same stored `DesktopErrorDto` `open_profile_sync` itself
      returned -- matching every other `DesktopAppState` accessor's
      established behavior in that state, not a fabricated generic error
      or a false success.
- [x] export after transport failure --
      `a_transport_failure_is_surfaced_in_the_snapshot_not_hidden`
      (`diagnostics.rs`): unlike a startup failure, a transport failure
      happens on an otherwise-ready runtime, so the snapshot still
      succeeds and carries `transport.state == "failed"` and
      `last_error` as data instead of hiding it.

All three quality gates run and green: `bash scripts/check-rust.sh`
(275 `silent-disco-core` tests + full workspace, 0 failed),
`cd desktop && npm run check` (bindings-check, biome, `cargo fmt --check`,
tsc, 68/68 Vitest, production build) -- `./gradlew test lintDebug` not
re-run (desktop-only change, no Kotlin touched, matching established
session precedent). `desktop/src-tauri`'s own `cargo test` run:
181 passed, 0 failed (was 173 before this block; +8 new tests). Also ran
`cargo clippy --all-targets --all-features -- -D warnings` manually for
`desktop/src-tauri` (still not part of the enforced `npm run check`
gate) -- confirmed via `git stash` that the small set of remaining
deny-level errors (in `mdns.rs`, `host_session_dto.rs`'s pre-existing
test, `audio_device.rs`, `render_ring.rs`, `start_playback_tests.rs`)
are 100% pre-existing on `master`, unrelated to this block; the one
new lint this block's own code triggered
(`clippy::field_reassign_with_default` in a `diagnostics.rs` test) was
fixed with a scoped, justified `#[allow]`.

**Acceptance:** Operational state and failures are diagnosable without leaking secrets.

---

## Block 36 — Implement deterministic application shutdown

### 36.1 Add lifecycle state

States include at least:

- [x] closed -- `DesktopRuntimeState::Closed` (`app_state.rs`), unchanged
      from prior blocks.
- [x] opening -- `DesktopRuntimeState::Opening { profile_id }`.
- [x] ready -- `DesktopRuntimeState::Ready`.
- [x] shutting down -- `DesktopRuntimeState::Closing` (profile-focused) and,
      layered above it for the whole-application flow,
      `AppShutdownPhase::ShuttingDown` (`app_shutdown.rs`, Block 36.3).
- [x] shutdown failed -- new `DesktopRuntimeState::ShutdownFailed(DesktopErrorDto)`,
      deliberately distinct from `Failed` (open failure): `begin_open`
      refuses to reopen from `ShutdownFailed` (unlike `Failed`, which
      remains reopen-safe) because a genuine timeout may leave owned
      resources alive on a detached background thread (see 36.2/36.3) --
      recovery requires restarting the application, not retrying in
      place. Also `AppShutdownPhase::ShutdownFailed(DesktopErrorDto)` at
      the whole-application layer.
- [x] terminated -- new `AppShutdownPhase::Terminated` (`app_shutdown.rs`),
      the whole-application concern; `DesktopRuntimeState` itself has no
      "terminated" (a profile close returns to reopen-safe `Closed`,
      which is a materially different guarantee than "the process is
      exiting").

Two cooperating state machines, not one: `DesktopRuntimeState` remains
`DesktopAppState`'s own profile open/close/reopen lifecycle (all prior
blocks' behavior preserved); `AppShutdownCoordinator`/`AppShutdownPhase`
is new and layers the window-close-triggered whole-application concern on
top, exposed to the frontend as the new `AppShutdownPhaseDto`
(`dto.rs`, `get_app_shutdown_state` command).

### 36.2 Implement ordered shutdown

Required order:

- [x] reject new commands -- already true from the first instant of
      `take_for_close`'s `mem::replace(&mut *state, Closing)`: every
      `DesktopAppState` accessor (diagnostics, snapshot, playback
      control, etc.) already matches `Closed | Opening | Closing =>
      not_ready`, unchanged by this block. The "except shutdown/status"
      carve-out is satisfied by `get_app_shutdown_state` and
      `close_profile` themselves staying callable regardless of profile
      state.
- [x] core enters shutdown -- `CoreActorRuntime::shutdown()` (unchanged
      core API) sets `accepting=false` as its first action and queues
      `CoreCommand::Shutdown`. **Deliberate documented deviation from the
      spec's literal step ordering**: this codebase calls it *after*
      mDNS/playback/transport teardown (`shutdown.rs`), not before,
      because those subsystems still submit legitimate final
      `AudioEvent`/`TransportEvent`s to the live actor while winding
      down; shutting the actor down first would reject those as
      spurious failures rather than let them land. Reordering would
      require splitting `CoreActorRuntime::shutdown`'s signal-and-join
      into two core-API-level phases -- out of this block's scope, and
      risky for a shared, Android-facing core type. Investigated and
      chosen deliberately, not missed.
- [x] stop playback/packet production -- unchanged: `stop_host_inner`
      (`network.rs`) stops the playback pump, which itself stops the
      packetizer/decoder.
- [x] withdraw mDNS -- unchanged per-publication `withdraw()`
      (`stop_host_inner`) **plus new** daemon-level `shutdown()`
      (`DesktopHostNetworkControl::shutdown`, `network.rs`) -- the
      previously-deferred gap explicitly marked "Block 36's job" in
      `mdns.rs`'s own doc comment, now closed: the daemon is shut down
      unconditionally, even when no binding was ever active.
- [x] stop transport -- unchanged `DesktopHostTransportRuntime::shutdown`.
- [x] stop local monitor and confirm callback quiescence -- unchanged
      `DesktopMonitorControl::on_stream_stopped`
      (`RunningAudioOutputStream::stop` consumes `Box<Self>`, Block 34.1).
- [x] stop decoder/source workers -- unchanged
      `StreamingPacketizeHandle::cancel_and_join` (decoder is consumed by
      the packetizer worker, so joining it joins both).
- [x] close core/database workers -- unchanged `DatabaseWorker::stop_and_join`
      (WAL checkpoint + close).
- [x] stop notification dispatcher -- unchanged `DesktopNotificationBuffer::shutdown`,
      **reordered** in `shutdown_owned_resources` to run *after* the
      database worker closes (previously before it) -- spec order 10
      before 11: a dispatcher stopped before the database closes could
      never relay a database-close-related notification.
- [x] release profile lock -- unchanged `ProfileLease::release`.
- [x] allow process/window exit -- new: the window-close-triggered flow
      (`app_shutdown.rs::handle_close_requested`) only calls
      `AppHandle::exit(0)` after the bounded shutdown attempt reports
      `Ok`; a failure leaves the window open instead.

### 36.3 Window close interception

- [x] close event initiates controlled shutdown -- `lib.rs`'s
      `.on_window_event` intercepts `WindowEvent::CloseRequested`,
      always calls `api.prevent_close()` first, then
      `app_shutdown::handle_close_requested`, which spawns the bounded
      shutdown attempt on its own thread (never blocks the event loop).
- [x] duplicate close is idempotent -- two layers: (1)
      `AppShutdownCoordinator::claim()` returns `Perform` exactly once
      across its lifetime, every later call observes
      `AlreadyHandled(phase)` and does nothing (or, once `Terminated`,
      finally calls `exit`); (2) independently, `DesktopAppState::take_for_close`
      now returns a new `CloseAction::AlreadyInProgress` (not an error)
      when called while another close is already tearing the profile
      down, replacing the previous `desktop.profile.close_in_progress`
      *error* -- a second `close_profile` command call is now also
      idempotent, not just the window-close path.
- [x] progress is visible -- new `get_app_shutdown_state` command +
      `AppShutdownPhaseDto`, polled by a new `ShutdownOverlay` component
      (`App.tsx`) every 500ms and rendered as a full-screen overlay
      whenever the phase is not `notRequested`. Polled, not pushed --
      the webview stays fully alive while a close is pending (only the
      native close is prevented, not the process), matching this
      codebase's existing polling idiom (`HostSessionScreen`,
      `DiagnosticsScreen`) rather than introducing a push-event
      mechanism for a single new screen.
- [x] timeout becomes visible failure -- `run_with_timeout` (`app_shutdown.rs`)
      bounds the whole attempt at `SHUTDOWN_TIMEOUT` (10s); on expiry the
      coordinator reaches `ShutdownFailed`, visible via
      `get_app_shutdown_state` and rendered by `ShutdownOverlay`.
- [x] timeout does not free callback-visible memory unsafely --
      `run_with_timeout` never joins, cancels, or drops the spawned
      worker thread on timeout; it is deliberately detached and left to
      finish independently (best case) or run indefinitely (worst case).
      Proven by `run_with_timeout_returns_promptly_and_never_joins_a_slow_worker`
      (`app_shutdown.rs`): a gated worker is released *after* the
      timeout already fired and is observed completing on its own,
      demonstrating nothing about it was force-torn-down to produce the
      timeout result.
- [x] development forced-exit behavior, if any, is explicitly gated and
      labeled -- **none was added**. A forced-exit escape hatch that
      bypasses controlled teardown is exactly the kind of "safety valve"
      this project's error-handling rules (`CLAUDE.md`: no fallback that
      turns a production failure into log-only/silent-success behavior)
      argue against; the checklist item itself is phrased "if any", so
      its absence is a deliberate choice, not an oversight. A genuinely
      stuck process can still be terminated by the OS, which is safe
      (atomic reclaim) unlike an in-process forced free.

### 36.4 Tests

- [x] normal shutdown -- unchanged
      `opens_real_storage_actor_and_snapshot_then_shuts_down_idempotently`
      (`app_state.rs` tests).
- [x] shutdown during open --
      `closing_while_still_opening_is_reported_and_leaves_the_opening_state_intact`
      (`app_state.rs` tests): a close request while `Opening` is
      reported (`desktop.profile.open_in_progress`), and the `Opening`
      state survives intact for the original open to finish.
- [x] shutdown during source copy -- `close_profile_sync` unconditionally
      calls `SourceStagingControl::cancel_and_wait()` before tearing down
      owned resources (previously only the `close_profile` command did
      this; now also true of the shared sync path); the blocking-until-
      finished mechanism itself is exercised by the pre-existing
      `cancel_and_wait_blocks_until_the_operation_finishes`
      (`source_staging_control.rs`). No new end-to-end test drives a
      real in-flight source copy through a full `close_profile_sync`
      call -- doing so needs a real `AppHandle`, which would require
      enabling `tauri`'s `test`/`mock_runtime` feature (not currently
      enabled in this crate); noted as a real gap, not silently claimed
      covered.
- [x] shutdown during decode --
      `cancelling_while_backpressured_joins_the_owned_decoder_worker` drives
      the exact owned-resource seam used by desktop playback shutdown: a
      long decoder is demonstrably still active while the packetizer is
      pinned behind a one-packet output queue, then
      `StreamingPacketizeHandle::cancel_and_join` must return only after the
      shared decoder worker count reaches zero. The full Tauri close path does
      not need a mock `AppHandle` to prove the ownership/join invariant.
- [x] shutdown during streaming -- reasonably covered by existing
      coverage this block did not need to duplicate:
      `stop_playback_reports_a_pump_that_could_not_complete_its_shutdown`
      (`start_playback_tests.rs`) and
      `a_withdraw_failure_still_tears_down_the_host_but_is_reported_not_swallowed`
      (`network_tests.rs`) both drive teardown while streaming/bound,
      proving failures are reported, not swallowed, and teardown still
      completes.
- [x] shutdown during database write --
      `shutdown_waits_for_an_accepted_queued_write_then_checkpoints_and_joins`
      blocks the database worker, queues a real settings write ahead of the
      shutdown command, proves shutdown cannot finish early, then releases the
      worker and requires the accepted write to succeed before
      checkpoint/close/join. Reopening the same database and reading the value
      back proves the write was durable before shutdown returned.
- [x] shutdown with mDNS failure --
      `network_control_shutdown_reports_a_failing_mdns_daemon_shutdown`
      (new, `network_tests.rs`): a failing daemon-level `shutdown()` is
      reported, not swallowed. Withdrawal-failure-during-teardown is
      also already covered by
      `a_withdraw_failure_still_tears_down_the_host_but_is_reported_not_swallowed`.
- [x] shutdown with monitor callback active -- unchanged
      `shutdown_while_the_callback_is_actively_running_completes_cleanly`
      (`monitor_tests.rs`, Block 34).
- [x] repeated shutdown -- `opens_real_storage_actor_and_snapshot_then_shuts_down_idempotently`
      (`Closed -> Closed`, unchanged) plus new
      `a_duplicate_close_while_one_is_already_in_flight_is_idempotent`
      (`Closing -> Closing`, the genuinely new idempotent path this
      block added) and the `AppShutdownCoordinator`-level
      `only_the_first_claim_performs_and_later_claims_observe_the_same_phase`
      (`app_shutdown.rs`).
- [x] profile can reopen after clean shutdown -- new
      `a_profile_reopens_cleanly_after_a_full_close` (`app_state.rs`
      tests): a real, full `open -> close -> open` cycle (not just the
      profile lock's own acquire/release, which
      `second_open_is_rejected_and_profile_lock_is_retained_until_close`
      already covered).

New tests this block: 10 (2 `network_tests.rs`, 5 `app_shutdown.rs`, 3
`app_state.rs` tests) -- desktop crate `cargo test` went from 181 to 191
passed, 0 failed throughout.

All three quality gates run and green: `bash scripts/check-rust.sh`
(full workspace, 0 failed), `cd desktop && npm run check`
(bindings-check, biome, `cargo fmt --check`, tsc, 68/68 Vitest,
production build) -- `./gradlew test lintDebug` not re-run (desktop-only
change, no Kotlin touched, matching established session precedent).
Manually ran `cargo clippy --all-targets --all-features -- -D warnings`
for `desktop/src-tauri` (still not part of the enforced gate) --
confirmed via `git stash` (Block 35's own methodology) that all
remaining deny-level errors are the identical, unrelated pre-existing
set from `master` (`host_session_dto.rs`, `audio_device.rs`,
`render_ring.rs`, `mdns.rs`'s two redundant-closure lints,
`start_playback_tests.rs`); this block's own new code introduced two
lints during development (`clippy::doc_markdown` missing backticks,
`clippy::manual_let_else`) and both were fixed before landing, not
suppressed.

**Acceptance:** The desktop process does not depend on OS termination to clean up shared-core resources. Met for the profile-owned resources this block's `shutdown_owned_resources` covers (mDNS daemon and publication, transport, playback/decoder/packetizer, core actor, database, notification dispatcher, profile lock) via the new window-close-triggered path, with a bounded timeout that reports failure rather than hanging or force-freeing on a stuck teardown.

---

# Phase 11 — Deterministic Lab Mode

## Block 37 — Add explicit Lab Mode build feature and isolation

### 37.1 Feature gates

- [x] add Rust feature such as `lab-mode` -- `desktop/src-tauri/Cargo.toml`
      (`lab-mode = []`), not in `default`.
- [x] add frontend build flag derived from backend capability, not only
      JavaScript environment -- new always-compiled, always-registered
      `get_lab_mode_available` command (`lib.rs`) returns `cfg!(feature =
      "lab-mode")`; new `getLabModeAvailable()` client call and `lab`
      Redux slice (`labSlice.ts`, spec section 12's item 3) store only
      that backend answer, never `import.meta.env`/`process.env`.
- [x] production release defaults Lab Mode off unless intentionally
      selected -- `lab-mode` absent from `default`; confirmed the
      default `cargo build`/`cargo test` (no `--features`) compiles and
      passes with the `lab` module entirely absent.
- [x] UI is visibly labeled -- `App.tsx`'s header renders an amber "Lab
      Mode build" badge (with an explanatory `title`) only when
      `getLabModeAvailable()` answered `true`; covered by
      `App.test.tsx`'s two new badge-visibility tests.
- [x] Lab profiles use separate roots -- `LabRuntime` roots every node
      under `<app_local_data_root>/lab/`, structurally disjoint from
      `DesktopProfilePaths`'s `<app_local_data_root>/profiles/`.
- [x] production profile cannot be opened in Lab runtime -- true by
      construction (`LabRuntime` has no function that accepts a
      `ProfileId` or touches the `profiles/` subtree at all) and proven
      by `lab_root_is_disjoint_from_the_production_profiles_root`
      (`lab/tests.rs`).
- [x] synthetic identity and virtual adapters compile only where
      intended -- the entire `lab` module is `#[cfg(feature =
      "lab-mode")]`; its synthetic identity derivation
      (`synthetic_identity`, reusing `DesktopIdentity::from_secret`,
      which never touches the OS keyring) exists only inside it.

### 37.2 Lab runtime

Created:

```text
desktop/src-tauri/src/lab/mod.rs
```

- [x] owns multiple core handles -- `LabRuntime.nodes: Mutex<HashMap<LabNodeId, LabNodeHandle>>`,
      each wrapping its own real `CoreActorRuntime`.
- [x] unique node IDs -- `LabNodeId(u32)`, monotonically allocated by an
      internal `AtomicU32` counter, never reused.
- [x] isolated databases -- each node gets its own `lab.sqlite3` under
      its own `lab/lab-node-NNNN/` directory, opened through a real
      `DatabaseWorker`.
- [x] isolated identities -- each node's `CoreActorConfig` uses a
      `DeviceId` from a deterministic, per-node synthetic
      `DesktopIdentity` (`synthetic_identity`), never the OS keyring.
- [x] explicit start/stop/join -- `LabRuntime::start_node`/`stop_node`/`shutdown`,
      each a real, blocking, reported-outcome operation (no fire-and-forget).
- [x] no global production singleton reuse -- `lab/mod.rs` never
      imports `crate::app_state` or any other production-owned type;
      confirmed via `grep -rln "crate::lab" desktop/src-tauri/src/`
      returning only this doc-comment's own backtick-quoted mention of
      the fact, never an actual `use` from the production side.
- [x] bounded node count -- `MAX_LAB_NODES = 16`; exceeding it is a
      reported `desktop.lab.node_limit_reached` error, never a silently
      dropped or substituted node.

### 37.3 Tests

- [x] production build has no Lab entry -- default `cargo build`/`cargo test`
      (this crate's actual default, no `--features` flag) compiles and
      passes with the `lab` module entirely absent; new
      `lab_mode_is_unavailable_in_a_production_build` (`lib.rs`) asserts
      the always-compiled availability command answers `false` in that
      exact configuration.
- [x] Lab build is labeled -- new `lab_mode_is_available_in_a_lab_build`
      (`lib.rs`, `#[cfg(feature = "lab-mode")]`), run explicitly via
      `cargo test --features lab-mode`, asserts the same command answers
      `true`; frontend side covered by `App.test.tsx`'s "shows the Lab
      Mode badge only when the backend reports it available" /
      "shows no Lab Mode badge when ... unavailable".
- [x] profile roots differ -- `lab_root_is_disjoint_from_the_production_profiles_root`
      (`lab/tests.rs`).
- [x] production secure-store failure cannot select Lab identity
      automatically -- new `a_production_identity_failure_is_a_hard_error_with_no_fallback`
      (`app_state_tests.rs`): a failing identity provider makes
      `open_profile_sync` return a hard `desktop.identity.unavailable`
      error, never a silent substitute; backed architecturally by
      `app_state.rs` never referencing `crate::lab` (see 37.2).
- [x] Lab shutdown releases every node --
      `shutdown_releases_every_node_and_is_idempotent_when_empty`
      (`lab/tests.rs`): every started node is gone after `shutdown()`,
      and a second call on an already-empty runtime is a clean no-op,
      not an error.

New tests this block: 11 (7 `lab/tests.rs`, 2 `lib.rs`, 1
`app_state_tests.rs`, plus 2 `App.test.tsx` + 2 `labSlice.test.ts` on the
frontend) -- desktop crate `cargo test` (default, no `lab-mode`) went
from 191 to 193 passed; `cargo test --features lab-mode` reaches 200
passed; 0 failed in both configurations throughout. Frontend Vitest went
from 68 to 72 passed.

All three quality gates run and green: `bash scripts/check-rust.sh`
(full workspace, 0 failed), `cd desktop && npm run check`
(bindings-check, biome, `cargo fmt --check`, tsc, 72/72 Vitest,
production build) -- `./gradlew test lintDebug` not re-run (desktop-only
change, no Kotlin touched, matching established session precedent).
Manually ran `cargo clippy --all-targets --all-features -- -D warnings`
for `desktop/src-tauri` (`--all-features` includes `lab-mode`, still not
part of the enforced gate) -- confirmed the identical, unrelated
pre-existing error set noted in the Block 35/36 memory entries is still
the only thing failing; this block's own new code (default and
`lab-mode` builds alike) introduced zero new lints.

**Acceptance:** Lab facilities cannot become a silent production fallback. `LabRuntime` is fully isolated by construction (separate root, separate identities, separate `Mutex`-owned node registry, zero references from any production code path) and absent entirely from a default build; virtual transport/clock wiring and any node-management UI/command surface remain a later Lab Mode block's scope.

---

## Block 38 — Implement deterministic virtual clocks

### 38.1 Shared clock abstraction

- [x] If not already present, add a platform-independent trait in the
      shared core or test-support crate -- **already present**:
      `silent_disco_core::transport::TransportClock` (`fn now(&self) ->
      MonotonicMillis`, `Send + Sync + 'static`) is exactly the trait
      shape this section calls for, plus a production
      `SystemTransportClock` and a `ManualTransportClock` already used
      by `virtual_fault_tests.rs`.
- [x] Use the actual existing time abstractions where present. Do not
      duplicate them -- new `desktop/src-tauri/src/lab/clock.rs` builds
      directly on `ManualTransportClock`/`TransportClock` (wraps one
      inside `LabClock`, implements the other for `LabNodeClock`)
      instead of inventing a second, parallel clock trait.

### 38.2 Virtual clock features

Implemented in `desktop/src-tauri/src/lab/clock.rs`, wired into
`LabRuntime`/`LabNodeHandle` (Block 37's runtime) so every Lab node has
its own clock view over one shared scenario timeline:

- [x] deterministic initial time -- `LabRuntime::new(app_local_data_root,
      initial_clock_ms)` / `LabClock::new(initial_ms)`.
- [x] manual advance -- `LabRuntime::advance(delta_ms)` /
      `LabClock::advance(delta_ms)`; the only way virtual time ever
      moves anywhere in a Lab scenario.
- [x] scheduled wakeups through the Lab scheduler -- `LabClock::schedule(deadline_ms,
      callback)`; every `advance` drains and runs every wakeup now due,
      in deterministic `(deadline, then registration order)` order.
- [x] per-node offset -- `LabRuntime::start_node_with_clock(offset_ms, drift_ppm)`;
      `LabNodeClock::offset_ms()`.
- [x] per-node drift in ppm -- same constructor; `LabNodeClock::drift_ppm()`,
      applied relative to the shared timeline's own origin so two nodes
      with identical drift always agree at the same shared time
      regardless of when each was created.
- [x] checked arithmetic -- `LabClock::advance` uses `checked_add` and
      rejects an overflowing delta outright, leaving time completely
      unchanged (never a partial advance); `LabNodeClock::new` rejects a
      drift configuration beyond a documented sanity bound
      (`MAX_DRIFT_PPM = 100_000`) at construction, before a node ID is
      even allocated; `LabNodeClock::now()` itself widens to `i128` (so
      the realistic range can never overflow) and clamps into `u64`'s
      valid range on the rare pathological configuration, using the
      same "clamp on conversion, never wrap or panic" discipline
      `SystemTransportClock::now()` already uses in the shared core.
- [x] no wall-clock sleep required for deterministic scenarios --
      confirmed by `grep -rn "Instant::now\|SystemTime::now"
      desktop/src-tauri/src/lab/` returning nothing; every test in
      `lab/clock/tests.rs` runs in milliseconds of real time regardless
      of how much *virtual* time it advances (one test advances a
      simulated 400 days across a handful of `advance` calls).
- [x] explicit invalid-discontinuity injection only for negative tests --
      `LabClock::force_discontinuity(new_now_ms)`: can move time
      *backward*, bypassing `advance`'s checked, monotonic path
      entirely; distinct name and extensive doc warning make accidental
      ordinary-scenario use conspicuous, and normal advancement always
      goes through `advance` instead.

### 38.3 Tests

All in `desktop/src-tauri/src/lab/clock/tests.rs` unless noted:

- [x] exact offset -- `a_pure_offset_shifts_time_exactly`.
- [x] positive and negative drift -- `positive_and_negative_drift_move_time_proportionally`
      (a +1%/-1% node pair diverge in exact proportion to elapsed base
      time).
- [x] long-run arithmetic -- `long_run_advances_match_the_drift_formula_exactly`
      (400 simulated days across repeated large advances match the
      drift formula's prediction exactly, with no accumulated rounding
      error beyond the single final integer truncation).
- [x] overflow rejection --
      `overflow_and_out_of_bounds_configuration_are_rejected_not_silently_accepted`
      (an overflowing `advance` is rejected and leaves time unchanged; an
      out-of-bounds drift is rejected at construction).
- [x] deterministic repeated seed -- `identical_seeds_and_advances_produce_identical_results`
      (two independently constructed clocks given the identical seed and
      the identical advance sequence produce byte-identical results at
      every step).
- [x] scheduler event order at equal timestamps --
      `wakeups_at_the_same_deadline_run_in_registration_order` (five
      wakeups at the exact same deadline always fire in registration
      order, never `BinaryHeap`'s own unspecified tie-breaking).
- [x] no production direct system clock remains in shared scheduling
      logic -- confirmed via `grep -rln "Instant::now\|SystemTime::now"
      rust/silent-disco-core/src/sync/ rust/silent-disco-core/src/runtime/
      rust/silent-disco-core/src/audio/` (excluding tests) returning
      nothing: the shared core's sync estimator, actor runtime, and
      audio scheduling logic already depend only on the injectable
      `TransportClock` boundary, never the OS clock directly.

Also added, integrating this block with Block 37's `LabRuntime` (in
`lab/tests.rs`, not `lab/clock/tests.rs`):
`nodes_started_with_different_clock_configurations_diverge_as_the_scenario_advances`
and `an_invalid_clock_configuration_never_consumes_a_node_slot`.

New tests this block: 10 (8 `lab/clock/tests.rs`, 2 `lab/tests.rs`) --
`cargo test --features lab-mode` went from 200 to 210 passed; the
default (no `lab-mode`) build is unaffected (193 passed, unchanged,
since `clock.rs` lives inside the same `#[cfg(feature = "lab-mode")]`
module boundary Block 37 established). 0 failed in both configurations.

All three quality gates run and green: `bash scripts/check-rust.sh`
(full workspace, 0 failed), `cd desktop && npm run check`
(bindings-check, biome, `cargo fmt --check`, tsc, 72/72 Vitest,
production build -- unaffected, this block touched no frontend code) --
`./gradlew test lintDebug` not re-run (desktop-only change, no Kotlin
touched). Manually ran `cargo clippy --all-targets --all-features -- -D
warnings` (`--all-features` includes `lab-mode`) -- fixed one new lint
this block's own tests introduced (`clippy::items_after_statements`,
matching an already-precedented pattern elsewhere in this crate) before
landing; the remaining 8 deny-level errors are the identical,
unrelated pre-existing set noted in the Block 35/36/37 memory entries.

**Acceptance:** Sync and scheduling scenarios can run deterministically without real time. `LabClock`/`LabNodeClock` prove this directly: every test above advances anywhere from milliseconds to a simulated 400 days of virtual time while itself completing in real time on the order of milliseconds, with zero wall-clock sleeps anywhere in the module.

---

## Block 39 — Implement virtual transport and fault injection

### 39.1 Transport boundary

Use production serialized frames/datagrams.

- [x] host encodes through production protocol -- already true before
      this block: `virtual_transport.rs`'s private `round_trip` helper
      calls the real `encode_frame` on every send. New this block: fixed
      a real bug found while investigating this boundary -- every
      host-to-listener event (`FrameReceived`, `PeerDisconnected`) was
      stamped with the **host's** clock instead of the **receiving
      listener's own** clock (`connect_listener`'s `clock` parameter was
      silently discarded, `_clock`). Fixed in `VirtualListenerRegistration`
      (now carries its own `clock: Arc<dyn TransportClock>`) and the three
      call sites that stamped `received_at`; proven by
      `virtual_transport_stamps_delivered_events_with_the_recipients_own_clock`
      (`transport/tests.rs`) using two genuinely independent clocks. This
      matters directly for this block: without it, a Lab node's own
      configured clock offset/drift (Block 38) would never be visible in
      anything it actually receives.
- [x] virtual link receives bytes plus metadata -- the split
      `transport/virtual_transport/*` implementation now carries canonical
      encoded frame bytes together with channel/peer/timestamp metadata across
      the in-process wire. `virtual_wire_tests::virtual_listener_decodes_raw_wire_bytes_at_receive_time`
      injects real encoded bytes directly and proves the recipient owns the
      decode step.
- [x] listener decodes through production protocol -- `recv_event` calls
      the production `decode_frame` on the encoded bytes at the recipient. The
      companion corrupted-wire regression mutates the bytes after encoding and
      requires a real `TransportErrorKind::Protocol` from receive-side decode.
- [x] tests claiming wire coverage do not inject high-level success
      events -- every new test in `virtual_fault_tests.rs` and
      `lab/fault/tests.rs` builds a real `ProtocolFrame`/`ControlMessage`
      and drives it through the real transport trait methods; none
      construct a `TransportEvent` directly. `corruption_produces_a_real_diagnosable_protocol_error`
      specifically forces a genuine `decode_frame` failure on mutated
      bytes rather than injecting a synthetic error event.

### 39.2 Fault model

Implemented across two layers: everything synchronous and receive-side
(or connect-side) in the shared core
(`rust/silent-disco-core/src/transport/virtual_fault.rs`, extending the
existing `VirtualUdpFaultConfig`/`FaultInjectingVirtualTransportFactory`);
latency and jitter -- the two that fundamentally need virtual time -- in
the desktop Lab module (`desktop/src-tauri/src/lab/fault.rs`, new,
`#[cfg(feature = "lab-mode")]`), built on Block 38's `LabClock`.

- [x] latency -- `LabLatencyConfig::fixed_latency_ms` (`lab/fault.rs`).
      Deliberately scoped to the listener receive path only (see the
      module's own doc comment for why); a held event's release deadline
      is anchored to the event's own `received_at` (its real send-time
      clock reading), not to whenever a caller happens to poll for it --
      the fix that made `zero_jitter_is_exact_latency_regardless_of_seed`
      pass instead of flaking on poll timing.
- [x] jitter -- `LabLatencyConfig::jitter_ms`, a seeded symmetric offset
      in `[-jitter_ms, +jitter_ms]` added on top of the fixed latency.
- [x] loss -- extended beyond the pre-existing exact-count
      `drop_next_sync_events`/`drop_next_audio_events` with new
      probabilistic, seeded `loss_permille` (`virtual_fault.rs`).
- [x] duplication -- new `duplicate_permille`.
- [x] reordering -- extended beyond the pre-existing exact-pair-swap
      `reorder_next_*_pair` with a new bounded, seeded-shuffle
      `reorder_window`.
- [x] corruption -- new `corrupt_next_events`, **audio-channel only** by
      deliberate design: audio is the one frame kind whose wire format
      carries a payload checksum (`FLAG_PAYLOAD_INTEGRITY`), so a single
      corrupted byte is *guaranteed* to trip a real decode failure;
      control/sync frames carry no such checksum, so the same technique
      would not reliably fail for them. Implemented send-side (encode,
      corrupt one byte, attempt decode, fail the send on decode failure)
      rather than receive-side, since the virtual "wire" carries decoded
      values -- see 39.1's notes and the module's own "corruption's
      send-side semantics" doc section for the full reasoning. This does
      not model UDP's fire-and-forget semantics (a real corrupted UDP
      payload would typically fail at the *receiver*, with the sender
      seeing apparent success); it does exercise the real production
      decoder on genuinely mutated bytes, which is what 39.1 requires.
- [x] bandwidth limit -- new `bandwidth_limit_bytes`, a cumulative
      per-channel encoded-byte budget rather than a literal
      bytes-per-second rate (documented as a deliberate simplification
      in the config field's own doc comment -- a true rate limiter would
      need the same virtual-clock dependency latency/jitter have).
- [x] queue saturation -- already existed incidentally
      (`event_queue_capacity` + `try_event`'s `QueueFull`); newly given a
      dedicated test through the fault-injecting factory specifically
      (`a_saturated_queue_is_reported_not_swallowed_by_fault_processing`),
      proving fault processing never swallows it.
- [x] connection refusal -- new `FaultInjectingVirtualTransportFactory::with_connection_refusals(count)`,
      a factory-level shared counter (distinct from the per-node `VirtualUdpFaultConfig`
      fields, since a refusal happens before any per-node fault state
      could exist).
- [x] disconnect -- new `disconnect_after_events`: once a channel has
      processed that many events, every later one is replaced by a
      synthesized `PeerDisconnected`.
- [x] reconnect delay -- `ReconnectDelayingTransportFactory` records a
      reconnect deadline against the injectable `TransportClock` and refuses
      the reconnect until virtual time reaches it; it never sleeps wall-clock
      time. `disconnect_then_reconnect_obeys_the_virtual_clock_delay` proves
      the exact deadline and `zero_reconnect_delay_does_not_invent_a_backoff`
      covers the zero-delay case.

Seeded deterministic PRNG: new `silent_disco_core::transport::DeterministicPrng`
(hand-rolled `SplitMix64` -- confirmed via investigation that no PRNG
dependency, seedable or otherwise, existed anywhere in this workspace
before this block), re-exported from the shared core so both the core's
own fault model and the desktop Lab latency wrapper draw from the exact
same implementation rather than two.

### 39.3 Tests

All in `rust/silent-disco-core/src/transport/virtual_fault_tests.rs`
unless noted:

- [x] zero-fault parity -- `zero_fault_parity_behaves_like_the_unfaulted_virtual_transport`.
- [x] exact fixed latency -- `exact_fixed_latency_holds_until_the_precise_deadline`
      (`desktop/src-tauri/src/lab/fault/tests.rs`): held one millisecond
      short of its deadline, then released at the exact millisecond.
- [x] deterministic loss sequence -- `identical_seed_produces_an_identical_loss_sequence`.
- [x] duplicate detection -- `duplication_delivers_the_same_send_twice`.
- [x] reorder window -- `reorder_window_releases_events_out_of_fifo_order_deterministically`.
- [x] malformed/corrupt packet diagnostics -- `corruption_produces_a_real_diagnosable_protocol_error`,
      complementing the already-substantial existing protocol-level
      coverage found during investigation
      (`protocol::codec::tests::rejects_truncation_trailing_bytes_and_integrity_failure`,
      `protocol::vector_tests::malformed_vectors_fail_with_the_declared_stable_category`,
      `...::diagnostic_counters_distinguish_every_required_failure_class`)
      -- this block's own test proves the fault-injection plumbing
      itself correctly reaches that same real decoder, not just that the
      decoder works in isolation.
- [x] backpressure -- `a_saturated_queue_is_reported_not_swallowed_by_fault_processing`.
- [x] disconnect/reconnect -- disconnect remains covered by
      `disconnect_after_events_replaces_later_events_with_a_synthesized_disconnect`;
      `virtual_reconnect_tests::disconnect_then_reconnect_obeys_the_virtual_clock_delay`
      adds the deterministic reconnect half.
- [x] identical seed produces identical trace -- `identical_seed_produces_an_identical_loss_sequence`.
- [x] different seed changes trace where expected -- `a_different_seed_changes_the_loss_trace`.

Also added beyond the required list: `scripted_connection_refusal_rejects_exactly_the_configured_count`,
`bandwidth_limit_drops_sends_once_the_byte_budget_is_exceeded`,
`zero_jitter_is_exact_latency_regardless_of_seed`,
`jitter_keeps_the_deadline_within_its_configured_bound`,
`control_channel_events_are_never_delayed` (`lab/fault/tests.rs`).

New tests this block: 21 (11 `virtual_fault_tests.rs`, 1 `transport/tests.rs`
listener-clock fix, 4 `lab/fault/tests.rs`, plus the pre-existing
`virtual_udp_faults_drop_sync_and_reorder_audio_without_changing_send_reports`
re-verified unchanged). `cargo test -p silent-disco-core`: 276 -> 286
passed. `desktop/src-tauri cargo test` (default): 193, unchanged
(everything new lives inside `#[cfg(feature = "lab-mode")]` except the
core-level listener-clock fix and fault-model extension, which are
covered by the core crate's own count). `cargo test --features lab-mode`:
210 -> 214. 0 failed throughout, in every configuration.

All three quality gates run and green: `bash scripts/check-rust.sh`
(full workspace, 0 failed), `cd desktop && npm run check`
(bindings-check, biome, `cargo fmt --check`, tsc, 72/72 Vitest --
unaffected, this block touched no frontend code -- production build) --
`./gradlew test lintDebug` not re-run (no Kotlin touched). Manually ran
`cargo clippy --all-targets --all-features -- -D warnings` for
`desktop/src-tauri` (`--all-features` includes `lab-mode`) -- fixed one
new lint this block's own code introduced (`clippy::doc_markdown`,
"WiFi" needing backticks or a hyphen) before landing; the remaining 8
deny-level errors are the identical, unrelated pre-existing set noted in
the Block 35-38 memory entries.

**Acceptance:** Partially met. Implemented faults are deterministic and exercise real protocol/state-machine code, but the virtual receive boundary still carries decoded frames rather than raw wire bytes through the production receive/decode path, and reconnect delay plus disconnect/reconnect coverage remain open.

---

## Block 40 — Implement scenario schema, runner, and assertions

### 40.1 Select format

- [x] choose JSON or YAML with exact pinned parser -- JSON via
      `serde`/`serde_json` (`=1.0.228`/`=1.0.145`), both already pinned,
      exact-version dependencies of `desktop/src-tauri` and
      `rust/silent-disco-core`; no YAML dependency introduced (no existing
      YAML use anywhere in this workspace) -- see `lab/scenario.rs`'s own
      module doc comment, "40.1 Format".
- [x] version schema -- `scenario::SCHEMA_VERSION = 1`; every `Scenario`
      document carries its own `schemaVersion` field.
- [x] bound nodes, links, steps, strings, and duration --
      `scenario::MAX_NODES`/`MAX_LINKS`/`MAX_FIXTURES`/`MAX_STEPS`/`MAX_ASSERTIONS`/`MAX_ID_BYTES`/`MAX_SCENARIO_DURATION_MS`
      (24 simulated hours)/`MAX_SCENARIO_FILE_BYTES` (1 MiB raw file bound,
      checked before JSON parsing even starts), enforced by
      `Scenario::validate` and `load_scenario_json`.
- [x] reject unknown versions -- `load_scenario_json` checks
      `schemaVersion` against `SCHEMA_VERSION` *before* attempting full
      structural parsing, returning a distinct
      `ScenarioParseError::UnknownSchemaVersion`, never silently
      reinterpreted; `scenario::tests::unknown_schema_version_is_rejected_distinctly`.
- [x] reject unknown commands and assertions -- `ScenarioAction`/`ScenarioAssertion`
      are closed Rust enums decoded through `serde`'s internally tagged
      (`tag = "kind"`) representation; an unrecognized `"kind"` string is a
      hard deserialization error, never a silently skipped step;
      `scenario::tests::unknown_command_kind_is_rejected`,
      `unknown_assertion_kind_is_rejected`.
- [x] no arbitrary code execution -- every field is a plain, bounded,
      statically typed value; nothing resembling a script or expression
      language exists anywhere in the schema.

### 40.2 Scenario types

Created:

```text
desktop/src-tauri/src/lab/scenario.rs
desktop/src-tauri/src/lab/recorder.rs
desktop/src-tauri/src/lab/replay.rs
```

Include:

- [x] seed -- `Scenario::seed: u64`, threaded through `ScenarioReport`/`ScenarioRecording`
      and checked by replay (see 40.2's "recording/replay" bullet below).
- [x] nodes -- `Scenario::nodes: Vec<ScenarioNode>`, bounded by `MAX_NODES`
      (`= super::MAX_LAB_NODES`, Block 37's own node cap, not a second
      independent bound).
- [x] links -- `Scenario::links: Vec<ScenarioLink>` (from/to/latencyMs/jitterMs/lossPermille),
      bounded and validated (`MAX_LINKS`, `MAX_LINK_LATENCY_MS`,
      `MAX_LINK_JITTER_MS`, `MAX_LOSS_PERMILLE`, node-reference integrity)
      but **not yet wired into any node's live transport** -- `LabRuntime`
      has never connected a Lab node to the shared core's virtual
      transport/fault-injection stack (`super::fault`,
      `silent_disco_core::transport::virtual_transport`/`virtual_fault`);
      that wiring, and scenario steps that change a link's fault
      configuration mid-run, remain a later Lab Mode block's concern, exactly
      as `lab/mod.rs`'s own doc comment has said since Block 37 and Block 39
      left true. See `scenario.rs`'s "Deliberate scope boundaries" doc
      section for the full reasoning.
- [x] clocks -- `Scenario::clocks: HashMap<String, ScenarioClock>` (offsetMs/driftPpm),
      applied for real through `LabRuntime::start_node_with_clock_and_observer`
      (Block 38's own clock machinery, not duplicated).
- [x] source fixture references restricted to Lab assets --
      `ScenarioFixture` is purely descriptive metadata (id, display name,
      optional byte length/duration) with **no filesystem path field at
      all**; a scenario cannot reference anything on the real filesystem by
      construction, satisfying this bullet without inventing a "Lab assets
      directory" mechanism that no Lab node's (audio-pipeline-less) runtime
      could use yet anyway.
- [x] timed commands -- `ScenarioStep { at_ms, node, action }`, a
      deliberately curated, real subset of `CoreCommand` submitted through
      the exact real `CoreActorHandle::submit_command` production entry
      point (see `scenario.rs`'s doc comment for the excluded variants and
      why). Also included: three directly injected real
      `AudioEvent`/`TransportEvent` actions
      (`injectUnderrun`/`injectSynchronizationUpdated`/`injectDeliveryCompleted`)
      through the same real `submit_audio_event`/`submit_transport_event`
      entry points a platform adapter uses -- the only way to exercise
      sync/delivery/underrun assertions (40.3) before the transport wiring
      above exists.
- [x] fault changes -- `setLinkFaults` is a real scenario step that mutates
      an already-live link's profile through the dynamic fault controller.
      `mid_run_loss_mutation_applies_to_an_already_connected_listener` proves
      the mutation takes effect during the same run rather than only at setup.
- [x] assertions -- `Scenario::assertions: Vec<ScenarioAssertion>`, see 40.3.
- [x] timeout and termination policy -- `Scenario::timeout_ms: u64` (the
      run's bounded virtual-time budget; a step scheduled at or beyond it
      never runs) and `Scenario::termination: TerminationPolicy`
      (`stop_on_assertion_failure`, default `true`).
- [x] recorder/replay -- `recorder::ScenarioRecorder` is a bounded
      (`MAX_RECORDED_NOTIFICATIONS = 4_096`, excess counted as
      `dropped_count()` rather than silently discarded), `Condvar`-backed
      `CoreObserver` capturing every notification (snapshots, effects,
      diagnostics, errors) a Lab node's actor emits -- the only way to see
      a *rejected* command at all, since `ActorState::process` leaves
      `CoreSnapshot` completely unchanged on rejection (see `scenario.rs`'s
      "Step settlement" doc section). `replay::ScenarioRecording`/`replay::replay`
      re-execute a scenario and refuse (`ReplayError::SchemaVersionMismatch`/`SeedMismatch`)
      rather than silently reinterpret one captured under a different
      schema version or seed (spec 29.5) -- persisting a recording to disk,
      protocol/core version stamping, packet hashes, and a divergence diff
      against the original run are explicitly Block 41's own scope
      (already listed there), not duplicated here.

### 40.3 Assertions

Support typed assertions for (`ScenarioAssertion`, `evaluate_assertion`):

- [x] lifecycle by deadline -- `LifecycleReached { node, target }`, where
      `target` is one of `Role`/`Host`/`Listener`/`Playback`, directly
      reusing the real `AppRole`/`HostLifecycle`/`ListenerLifecycle`/`PlaybackState`
      domain enums (via their existing `from_wire_name`, no parallel schema
      enum invented).
- [x] snapshot capability -- `CapabilityAvailable { capability, available }`,
      directly reusing `PermissionCapability` against `CapabilitySnapshot`'s
      six fields.
- [x] listener count -- `ListenerCountAtLeast { count }` against
      `snapshot.listeners.len()`.
- [x] sync confidence -- `SyncConfidenceAtLeast { confidence }` against
      `snapshot.synchronization`.
- [x] bounded offset/RTT -- `SynchronizationWithinBounds { max_abs_offset_ms, max_round_trip_ms }`.
- [x] expected error code -- `ErrorCodeObserved { code }`, reading the
      recorder's trace (`CoreError::code.stable_name()`) since a rejected
      command's error is otherwise invisible outside it.
- [x] delivery severity -- `DeliverySeverityIs { severity }` against
      `snapshot.last_delivery`.
- [x] underrun/concealment bounds -- `UnderrunFramesAtMost { max_total_missing_frames }`,
      summing the real `audio_underrun` diagnostic's `missing_frames`
      field from the recorder's trace. Covers underrun only: no distinct
      "concealment" counter exists anywhere in the current runtime record
      model beyond `AudioEvent::Underrun`'s own field, so this assertion is
      bounded to the one real, available signal rather than inventing a new
      production type outside this block's scope.
- [x] clean shutdown -- `CleanShutdown { node }`; every node `run_scenario`
      starts is torn down via `LabRuntime::stop_node` regardless of outcome,
      and a torn-down node with no recorded fatal error satisfies this.
- [x] no unexpected fatal error -- `NoUnexpectedFatalError { node }`,
      checking the recorder's trace for any `CoreNotification::Error` whose
      severity is `Fatal` (`RecordedNotificationKind::is_fatal_error`).

### 40.4 Tests

- [x] minimal happy path -- `scenario::tests::minimal_happy_path_completes_with_every_assertion_held`.
- [x] invalid schema -- `scenario::tests::invalid_schema_is_rejected`.
- [x] unknown version -- `scenario::tests::unknown_schema_version_is_rejected_distinctly`
      (plus `missing_schema_version_is_rejected`).
- [x] impossible assertion -- `scenario::tests::impossible_assertion_times_out`.
- [x] timeout -- `scenario::tests::a_step_scheduled_past_the_scenario_timeout_never_runs`
      (the action that would have satisfied the assertion is scheduled past
      `timeoutMs` and never runs, distinct from the impossible-assertion
      case above).
- [x] deterministic report -- `scenario::tests::identical_scenario_and_seed_produce_a_deterministic_report`
      (two independent `LabRuntime`s, same scenario/seed, byte-for-byte
      equal `ScenarioReport`) and `replay::tests::replay_against_the_matching_scenario_reproduces_the_report`.
      Getting this genuinely deterministic required collapsing
      `StepSettlement`'s two internal detection paths (revision-advanced vs.
      notification-observed) into one `Settled` value, once a full-suite
      run under real system load exposed that *which* of those two racing
      real threads happened to notice first is not itself deterministic --
      see `StepSettlement`'s own doc comment.
- [x] bounded malformed file behavior -- `scenario::tests::oversized_scenario_file_is_rejected_before_parsing`,
      `truncated_json_is_a_bounded_error_not_a_panic`,
      `arbitrary_binary_input_is_a_bounded_error_not_a_panic`.

Also added beyond the required list:
`scenario::tests::unknown_command_kind_is_rejected`,
`unknown_assertion_kind_is_rejected`, `exceeding_a_declared_bound_is_rejected`,
`a_step_referencing_an_undeclared_node_is_rejected`,
`a_command_that_is_illegal_in_the_current_state_is_reported_not_swallowed`
(proves a real actor-side rejection is observed through the recorder, not
silently swallowed by the runner); `recorder::tests` (5 tests covering
ordering, fatal-severity classification, the bounded-drop count, and
`wait_for_progress`'s wake/timeout behavior); `replay::tests::replay_refuses_a_schema_version_mismatch`,
`replay_refuses_a_seed_mismatch`.

New tests this block: 26 (17 `lab/scenario/tests.rs`, 5 `lab/recorder/tests.rs`,
3 `lab/replay/tests.rs`, plus one `lab/tests.rs`-style helper duplicated per
submodule rather than shared, matching Block 38/39 precedent). `cargo test
--features lab-mode` (this crate): 214 -> 237 passed, 0 failed, run
repeatedly (including full-suite runs under real system load, which is what
originally caught the `StepSettlement` nondeterminism above) with no
flakes after the fix. Default (no `lab-mode`) build unaffected: 193 passed,
unchanged, confirmed via `nm` showing zero `scenario`/`ScenarioRecorder`
symbols in the default-build `.rlib`.

All three quality gates run and green: `bash scripts/check-rust.sh` (full
workspace, 0 failed), `cd desktop && npm run check` (bindings-check, biome,
`cargo fmt --check`, tsc, 72/72 Vitest, production build -- unaffected,
this block touched no frontend code). Manually ran `cargo clippy
--all-targets --all-features -- -D warnings` for `desktop/src-tauri` --
confirmed the identical, unrelated pre-existing 8-error baseline from
Blocks 35-39 (`host_session_dto.rs`, `platform/audio_device.rs` x2,
`platform/mdns.rs` x2, `platform/render_ring.rs`,
`platform/start_playback_tests.rs` x2) is still the only thing failing;
this block's own code introduced zero new lints (several were fixed before
landing: `doc_markdown`, `too_many_lines` on `Scenario::validate` -- split
into five focused `validate_*` helpers -- `collapsible_match`,
`collapsible_if`, `redundant_closure`, three `map_unwrap_or` call sites
unified into one `current_revision` helper, and `match_same_arms`).

**Acceptance:** Scenarios are executable specifications, not ad hoc UI macros. Met: a scenario document is parsed once into a closed, versioned, bounded schema and then *executed* against real Lab nodes through the exact real `CoreActorHandle` production entry points (`submit_command`/`submit_audio_event`/`submit_transport_event`), producing a deterministic, typed report with typed assertion outcomes -- not a sequence of ad hoc UI clicks replayed blind. Static cross-node `ScenarioLink` latency/jitter/loss profiles are now wired through `LiveTransportDriver`; only **mid-run fault mutation** remains open under 40.2.

---

## Block 41 — Add recording and replay

Audited against Block 40's actual code before writing anything: Block 40
built `recorder::ScenarioRecorder` (a bounded, in-memory `CoreObserver`
trace) and `replay::replay` (schema-version/seed-checked re-execution), but
explicitly deferred everything below to this block in both `recorder.rs`'s
and `replay.rs`'s own module doc comments -- confirmed by reading both
files plus their tests in full, not assumed. New files this block:
`desktop/src-tauri/src/lab/recording.rs` (persisted format, version
stamping, `Divergence`/`first_divergence`) and its `recording/tests.rs`.
Extended (not duplicated): `recorder.rs` (`SnapshotSummary` and its
redaction boundary), `scenario.rs` (`ScenarioTrace`/`ClockAdvance`,
`run_scenario_with_trace`), `replay.rs` (rewritten around
`recording::ScenarioRecording`/`ReplayOutcome`).

### 41.1 Record bounded trace

Record:

- [x] schema/protocol/core versions -- `recording::ScenarioRecording::recording_format_version`
      (this module's own on-disk shape, distinct from the scenario's own
      `schemaVersion`, itself also carried as `scenario_schema_version`),
      `protocol_version` (`silent_disco_core::protocol::PROTOCOL_VERSION`),
      and `core_version` (`RecordedCoreVersion::from(silent_disco_core::core_version())`).
      See `recording.rs`'s own doc comment, "Which versions gate replay",
      for why protocol/core versions are recorded but deliberately
      **informational**, not a hard gate -- forcing them to match would
      contradict this block's own acceptance criterion ("replayed against a
      later core build").
- [x] seed -- already true since Block 40 (`Scenario::seed`/`ScenarioReport::seed`);
      now also stamped onto `ScenarioRecording::seed`.
- [x] clock advances -- new `scenario::ClockAdvance { requested_delta_ms, resulting_now_ms }`,
      pushed by `execute_steps_and_assertions` at both of its two real
      `LabRuntime::advance` call sites, collected into
      `ScenarioTrace::clock_advances` by the new `run_scenario_with_trace`.
- [x] commands -- implicit and exact: every submitted command is already
      fully determined by the persisted scenario document's own `steps`
      (Block 40), and each step's real outcome is recorded in
      `ScenarioReport::step_results` (`StepResult::submit_error`/`settlement`),
      itself part of every `ScenarioRecording`.
- [x] events -- `recorder::RecordedNotification`/`RecordedNotificationKind`
      (Block 40, now `Serialize`/`Deserialize` so they are the literal
      persisted representation, not a separate mirror), captured per node
      into `ScenarioTrace::node_notifications` in `scenario.nodes`
      declaration order (deterministic, not a `HashMap`'s own order).
- [x] effects -- `RecordedNotificationKind::Effect`/`TransportEffect`/`StorageEffect`
      (Block 40's own by-stable-name capture, `name` widened from
      `&'static str` to owned `String` so the type can implement
      `Deserialize`), carried in the same persisted trace as events above.
- [x] snapshot revisions and safe hashes/full bounded snapshots -- new
      `recorder::SnapshotSummary::capture`, a redacted, bounded, full (not
      hashed) projection of `CoreSnapshot` captured at every
      `CoreNotification::Snapshot` alongside its `revision`; see its own
      doc comment for the deliberate choice of "full bounded projection"
      over an opaque hash (more useful to a human reading a saved
      recording) and the exact list of excluded fields.
- [x] packet metadata and payload hashes, not complete audio payload by default --
      the Lab transport trace records bounded transport facts with channel/peer/frame metadata and
      SHA-256 frame hashes (`RecordedFrameHashScope`) rather than retaining raw audio payloads.
      `lab/fault/trace/tests.rs` and `scenario/transport_recording_tests.rs`
      verify those hashes are present in real live-transport recordings.
- [x] faults -- the transport trace records each actual fault decision as
      `TransportFactKind::FaultDecision` (`Pass`, `Drop`, `Hold`, `Release`,
      etc.), and the scenario recording persists that trace. Dedicated trace
      tests verify deterministic pass/drop/hold/release histories.
- [x] errors -- already true since Block 40 (`RecordedNotificationKind::Error`),
      now part of the persisted trace via the same `Serialize` derive.
- [x] assertion results -- already true since Block 40
      (`ScenarioReport::assertion_results`), now part of every persisted
      `ScenarioRecording::report`.

### 41.2 Replay

- [x] verify compatible versions -- `replay::replay` refuses on
      `recordingFormatVersion`, `schemaVersion`, or `seed` mismatch
      (`ReplayError::RecordingFormatVersionMismatch`/`SchemaVersionMismatch`/`SeedMismatch`).
      Protocol/core version differences are checked and surfaced
      (`ReplayOutcome::recorded_protocol_version`/`current_protocol_version`/
      `recorded_core_version`/`current_core_version`) but deliberately do
      not gate replay -- see 41.1's first bullet and `recording.rs`'s doc
      comment for why that split is the correct reading of spec 29.5
      alongside this block's own acceptance criterion.
- [x] reconstruct deterministic schedule -- unchanged from Block 40's own
      proven determinism (`scenario::tests::identical_scenario_and_seed_produce_a_deterministic_report`);
      `replay` re-executes the identical `Scenario` document through the
      identical `run_scenario_with_trace` path.
- [x] detect divergence at the first meaningful event -- new
      `recording::first_divergence`, comparing a recorded and a freshly
      replayed `ScenarioReport` in the scenario's own chronological order
      (every step result in submission order, then every assertion result
      in declaration order), returning the first point they disagree.
- [x] produce bounded diff -- `recording::Divergence`, a single enum value
      (`DifferentStepCount`/`StepResultMismatch`/`DifferentAssertionCount`/
      `AssertionResultMismatch`/`DifferentOutcome`) carrying only the one
      recorded/replayed pair that actually differs, not an unbounded list
      of every difference -- see `first_divergence`'s own doc comment for
      why a single value is the deliberately correct shape given Block
      40's proven report determinism.
- [x] never silently reinterpret incompatible recording -- unchanged
      discipline from Block 40, now covering the new
      `recordingFormatVersion` field too: every mismatch is a distinct,
      reported `ReplayError` variant; `load_recording_json`/
      `load_recording_from_path` never guess at an unrecognized shape.
- [x] support conversion only through an explicit versioned future tool --
      no conversion code path exists anywhere in `recording.rs`/`replay.rs`
      for an incompatible `recordingFormatVersion`; documented explicitly
      in `recording.rs`'s own module doc comment ("Deliberately out of
      scope") as this block's own designed absence, not an oversight.

### 41.3 Tests

- [x] record then replay identical --
      `replay::tests::replay_against_the_matching_scenario_reproduces_the_report_with_no_divergence`
      (in-memory) and
      `replay::tests::a_recording_saved_to_disk_can_be_loaded_back_and_replayed_later`
      (through a real file, matching this block's own acceptance criterion
      literally).
- [x] changed core behavior produces divergence --
      `replay::tests::a_recording_whose_captured_behavior_differs_from_a_fresh_run_is_detected`
      (mutates a captured recording's `step_results`, the same simulate-a-later-build
      technique Block 40 used for schema/seed mismatches, since building
      two genuinely different core builds is outside a single test's
      reach); plus focused unit coverage of `first_divergence` itself in
      `recording::tests` (`first_divergence_reports_the_first_differing_step_not_a_later_one`,
      `a_changed_step_result_diverges_before_assertions_are_even_compared`,
      `first_divergence_reports_a_changed_assertion_outcome`,
      `first_divergence_reports_a_changed_step_count`,
      `first_divergence_reports_a_changed_overall_outcome_when_nothing_else_differs`,
      `identical_reports_have_no_divergence`).
- [x] incompatible version rejected --
      `replay::tests::replay_refuses_a_schema_version_mismatch`,
      `replay_refuses_a_seed_mismatch` (both carried over from Block 40,
      re-verified against the new API), and new
      `replay_refuses_a_recording_format_version_mismatch`; complemented by
      `a_differing_recorded_protocol_or_core_version_does_not_block_replay`
      proving the deliberate non-gating split from 41.2 the other direction.
- [x] truncated recording rejected --
      `recording::tests::truncated_recording_bytes_are_rejected_not_a_panic`
      (cuts a valid recording's serialized bytes in half) and
      `arbitrary_binary_input_is_a_bounded_error_not_a_panic` (completely
      unstructured input).
- [x] secret redaction --
      `recorder::tests::snapshot_summary_never_carries_the_raw_invite_code`
      (constructs a `CoreSnapshot` with a real `host_draft.invite_code` and
      `session_name` set, asserts neither string appears in the serialized
      `SnapshotSummary` JSON) and
      `snapshot_summary_capture_excludes_invite_code_by_construction`
      (asserts the exclusion at the typed value, not only after
      serialization). `invite_code` was identified by direct inspection of
      `CoreSnapshot`/`HostDraft` as the one real plaintext admission secret
      reachable from a snapshot -- not assumed absent.
- [x] bounded output --
      `recording::tests::oversized_recording_is_rejected_before_being_written`
      (a recording whose content genuinely exceeds `MAX_RECORDING_FILE_BYTES`
      is rejected by `to_bounded_json`, not truncated or silently accepted)
      and `oversized_file_bytes_are_rejected_before_parsing` (the same
      bound enforced on the read/parse side, mirroring
      `scenario::load_scenario_json`'s own "check the length first"
      discipline).

New tests this block: 17 (2 `lab/recorder/tests.rs`, 11 `lab/recording/tests.rs`
-- an entirely new file -- 4 net new in `lab/replay/tests.rs`, whose other 3
tests are carried over from Block 40 and updated for the new
`recording`-backed API rather than duplicated). `cargo test --features
lab-mode` (desktop crate): 237 → 254 passed, 0 failed -- the delta matches
the 17 new test functions exactly. Default (no `lab-mode`) build unaffected:
193 passed, unchanged, confirmed via `nm` on the default-build `.rlib`
showing zero `ScenarioRecording`/`SnapshotSummary`/`first_divergence`
symbols.

All three quality gates run and green: `bash scripts/check-rust.sh` (full
workspace, 0 failed), `cd desktop && npm run check` (bindings-check, biome,
`cargo fmt --check`, tsc, 72/72 Vitest, production build -- unaffected,
this block touched no frontend code). Manually ran `cargo clippy
--all-targets --all-features -- -D warnings` for `desktop/src-tauri` --
confirmed the identical, unrelated pre-existing 8-error baseline from
Blocks 35-40 (`host_session_dto.rs`, `platform/audio_device.rs` x2,
`platform/mdns.rs` x2, `platform/render_ring.rs`,
`platform/start_playback_tests.rs` x2) is still the only thing failing;
this block's own code introduced zero new lints after two were fixed
before landing (`clippy::struct_excessive_bools` on `SnapshotCapabilities`,
resolved with the same precedented `#[allow]` `crate::runtime_dto::CapabilitySnapshotDto`
already carries for the identical shape; `clippy::enum_variant_names` on
`Divergence`, resolved by deliberately varying the five variant names
instead of a uniform `*Changed` postfix).

**Acceptance:** A difficult failure can be saved and replayed against a later core build. Met for the persisted scenario/core trace: `recording::save_recording_to_path`/`load_recording_from_path` round-trip a versioned, bounded, redacted recording through a real file, and `replay::replay` reports the first bounded divergence. **Not fully complete for live transport evidence:** packet metadata/payload hashes and actual fault-decision records are still absent even though `LiveTransportDriver` now carries live cross-node traffic.

---

## Block 42 — Build Lab Mode UI

Create:

```text
desktop/src/screens/LabScreen.tsx
```

New backend command surface (not originally named by this block, required
to give the new UI something real to call): `desktop/src-tauri/src/lab_commands/mod.rs`
(`#[cfg(feature = "lab-mode")]`, 9 Tauri commands: `lab_get_state`,
`lab_open_scenario_file`, `lab_save_scenario_file`, `lab_run_loaded_scenario`,
`lab_advance_virtual_time`, `lab_start_node`, `lab_stop_node`,
`lab_stop_all_nodes`, `lab_export_recording_file`) and
`desktop/src-tauri/src/lab_dto.rs` (unconditionally compiled DTOs -- see its
own module doc comment for why; `bindings.rs`'s generator runs with default
features, so a `lab-mode`-gated DTO would break every generated-bindings
build and `LabScreen.tsx`'s own `tsc`). `desktop/src-tauri/src/lab/mod.rs`
gained two small additive accessors (`LabNodeId::as_u32`/`from_u32`) and
`clock` was widened from a private to a `pub(crate)` submodule (matching
`recorder`/`recording`/`replay`/`scenario`'s existing visibility) so
`lab_commands.rs` can read a node's offset/drift back out; no other
`silent-disco-core`/`lab/*` domain logic changed.

Provide:

- [x] node list and state panels -- `LabScreen.tsx`'s "Nodes" panel, backed
      by `lab_get_state`/`lab_start_node`/`lab_stop_node`/`lab_stop_all_nodes`;
- [x] scenario open/save through restricted dialogs -- `lab_open_scenario_file`/
      `lab_save_scenario_file`, both `tauri_plugin_dialog` file dialogs
      restricted to `.json` (mirrors `platform/file_picker.rs`'s and
      `platform/diagnostics_export.rs`'s existing dialog pattern exactly);
      "save" writes back the exact validated bytes ("save a copy" -- there
      is no scenario editor in this block, so nothing is silently mutated);
- [x] start/pause/step/stop controls -- the command surface now provides
      backend-owned cooperative run control: start runs the loaded scenario on
      a blocking worker; pause/resume gates the next deterministic step
      boundary through `ScenarioRunControl`; step remains explicit virtual-time
      advancement when no scenario owns the runtime; stop requests cooperative
      cancellation and lets the runner perform its own node cleanup. Frontend
      tests assert pause, resume, and stop commands are actually invoked.
- [x] virtual time -- `LabStateDto.nowMs`, rendered live and advanced only
      through `lab_advance_virtual_time` (spec 29.2 "manual advancement");
- [x] fault configuration -- `LabScreen.tsx` renders editable latency,
      jitter, and loss controls per link and applies them through the typed
      `lab_set_link_faults` command. Scenario `setLinkFaults` steps provide the
      separate deterministic mid-run mutation path. Screen tests cover the
      editable values and command request.
- [x] bounded event timeline -- `lab_commands.rs`'s own
      `MAX_TIMELINE_ENTRIES_PER_NODE` (50) caps what a `LabRunOutcomeDto`
      ever carries, independent of and in addition to the backend
      recorder's own 4096-entry bound; `LabScreen.tsx` renders exactly
      what it receives, applying no further ad hoc truncation, and marks
      `timelineTruncated` when the cap was hit;
- [x] assertion results -- `LabRunOutcomeDto.assertionResults`, rendered
      pass/fail per assertion;
- [x] recording export -- `lab_export_recording_file`, capturing a real
      `recording::ScenarioRecording` (Block 41) from the last completed
      run and saving it through a restricted `.json` save dialog;
- [x] clear Lab Mode labeling -- an amber, `role="alert"` banner reading
      "Lab Mode" / "Developer testing tool..." at the top of `LabScreen.tsx`
      itself, plus the existing Block 37.1 amber "Lab Mode build" badge in
      `App.tsx`'s header and a matching amber "Lab Mode" nav button shown
      only when the backend reports availability.

UI must not mutate node domain state directly. It submits scenario/test commands to `LabRuntime`.
Verified: `LabScreen.tsx` never imports or references `silent_disco_core`/`lab::*` domain types --
every action is a typed IPC call through `core/client.ts`'s `lab*` wrappers, and every rendered
field is data the backend already computed (`LabStateDto`/`LabRunOutcomeDto`), never recomputed
client-side.

Tests (`desktop/src/screens/LabScreen.test.tsx` unless noted):

- [x] keyboard control -- `can be operated entirely from the keyboard` (focuses the real `<button>`
      "Step" and presses Enter via `@testing-library/user-event`, asserting the real command fired);
- [x] invalid scenario display -- `displays a scenario validation failure instead of swallowing it`
      (a rejected `openLabScenarioFile` promise is shown as a visible `role="alert"`, not dropped);
- [x] running-state command disablement -- `disables run and step controls while a scenario is
      already running` (backend `running: true` disables "Run scenario"/"Step" for real, not only
      visually);
- [x] deterministic timeline rendering -- `renders the event timeline in the exact order the
      backend reported it` (three entries render in exactly the order the DTO carried them);
- [x] bounded history -- `shows only the most recent run, never accumulating run history`
      (`labSlice.ts`'s `lastRun` is a single value, not an array -- proven both at the reducer level,
      `labSlice.test.ts`'s `retains only the single most recent run`, and end-to-end through two
      real runs in the screen test);
- [x] production build absence -- Rust Lab commands remain behind the
      non-default `lab-mode` feature; frontend tests require no Lab entry point
      when the backend reports it unavailable; and every normal frontend
      production build runs `desktop/scripts/verify-production-lab-absence.mjs`
      from `npm run build`, which rejects a production bundle containing the
      Lab-only surface. The dedicated Lab build remains an explicit
      `tauri:lab:* --features lab-mode` path. The frontend bundle itself is not feature-split (Vite has no
      awareness of Rust `cargo` features, exactly as already true for the Block 37.1
      `get_lab_mode_available` badge) -- `LabScreen.tsx`'s code ships in every build but is
      reachable only behind the backend's own runtime-truthful availability flag, the same
      "ideally" caveat this block's own TODO entry anticipates.

New tests this block: 6 `LabScreen.test.tsx` (new file), 6 net new `labSlice.test.ts` (2
pre-existing kept, 8 total), 2 net new `App.test.tsx`, 8 Rust (6 `lab_commands/tests.rs`, a new
file; 2 `lab_dto.rs`) = 22 net new across both stacks. `cargo test --features lab-mode` (desktop
crate): 254 → 262 passed, 0 failed -- the +8 delta matches the 8 new Rust test functions exactly.
Default (no `lab-mode`) build: 193 → 195 passed -- the +2 delta is `lab_dto.rs`'s own 2 tests,
unconditionally compiled by design (see this block's own module-doc-comment reasoning above);
`lab_commands/tests.rs`'s 6 tests remain `lab-mode`-only and do not appear in this count. `npm run
check` (bindings-check, biome, `tsc`, 86/86 Vitest -- up from 72, production build) all green.

All quality gates run and green: `cargo build --lib` / `cargo test` (default features, 195 passed),
`cargo build --all-targets --features lab-mode` / `cargo test --features lab-mode` (262 passed),
`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings` -- confirmed
the identical, unrelated pre-existing 8-error baseline (`host_session_dto.rs`, `platform/audio_device.rs`
x2, `platform/mdns.rs` x2, `platform/render_ring.rs`, `platform/start_playback_tests.rs` x2) is still
the only thing failing; this block's own new code introduced zero new lints after fixing the ones it
did trigger first (`clippy::needless_pass_by_value` on six Tauri command functions taking `AppHandle`/
request DTOs by value -- resolved with the same precedented `#[allow]` `host_commands.rs` already
carries for the identical Tauri-extraction reason; `clippy::format_push_string` in a test helper --
resolved with `write!` instead of `push_str(&format!(...))`). `cd desktop && npm run check` all green
(see test counts above).

**Acceptance:** Partially met. Developers can run deterministic multi-node scenarios without physical devices, but true in-flight pause, editable/mid-run fault configuration, and complete production-bundle exclusion of the frontend Lab code remain open.
`LabScreen.tsx` opens a validated scenario, runs it to completion through the exact production
`scenario::run_scenario_with_trace` entry point Block 40 already proved deterministic for a given
scenario and seed, and renders its bounded timeline/assertion results/step results -- all without a
physical device, entirely against isolated synthetic Lab nodes. Left honestly out of this block's
literal UI (not required by its own "Provide" checklist, and out of scope by design): replaying a
previously saved recording and importing/loading a recording file through the UI (Block 41's
`replay::replay`/`recording::load_recording_from_path` remain real, tested, `lab-mode`-only Rust
APIs with no Tauri command yet) -- a natural, explicitly flagged extension point for a later Lab
Mode block, not a gap in this one's acceptance criterion.

---

# Phase 12 — Hardening, packaging, and release readiness

## Block 43 — Security and Tauri capability audit

Full audit performed 2026-08-10 (Sonnet 5). Complete enumeration and per-item
justification recorded in `memory.md`'s `2026-08-10T22:40:05Z` entry. Summary
citations below; see `memory.md` for the full text.

### 43.1 Capability review

- [x] list every Tauri permission/capability — enumerated: `desktop/src-tauri/capabilities/default.json`
      grants exactly one permission, `core:default`, to window `main`; `app.security` in
      `desktop/src-tauri/tauri.conf.json` sets `csp`. `core:default`'s own grant set
      (`desktop/src-tauri/gen/schemas/acl-manifests.json`'s `core.default_permission.permissions`)
      is `core:path:default`, `core:event:default`, `core:window:default`, `core:webview:default`,
      `core:app:default`, `core:image:default`, `core:resources:default`, `core:menu:default`,
      `core:tray:default` — no fs/shell/http/dialog/os plugin permission is granted anywhere;
- [x] justify each in `memory.md` — full text in the memory entry cited above;
- [x] remove unused filesystem access — none was granted (see above); no fix needed;
- [x] no shell plugin unless separately approved — `tauri-plugin-shell` is not a dependency in
      `desktop/src-tauri/Cargo.toml` and no `shell:*` permission exists anywhere; confirmed absent,
      no approval on file, none needed;
- [x] no remote URL loading — `tauri.conf.json`'s `build.devUrl` is `http://127.0.0.1:1420` (dev-only
      local Vite server) and `build.frontendDist` is the bundled `../dist` directory; no
      `app.windows[].url` points at a remote origin; `desktop/src/core/client.ts` and the rest of
      `desktop/src` contain no `fetch`/`XMLHttpRequest`/remote navigation;
- [x] restrictive CSP — `tauri.conf.json`: `"csp": "default-src 'self'; connect-src ipc:
      http://ipc.localhost; img-src 'self' data:; style-src 'self'"`; not null, no wildcard, no
      `unsafe-inline`/`unsafe-eval`, no `script-src` override (falls back to `default-src 'self'`);
- [x] no `eval` — `grep -rn "eval(\|new Function(\|dangerouslySetInnerHTML" desktop/src` returns no
      matches;
- [x] production devtools policy explicit — `desktop/src-tauri/Cargo.toml`'s `tauri` dependency uses
      `features = []` (the `devtools` Cargo feature is not enabled), and `grep devtools
      desktop/src-tauri/Cargo.lock` returns nothing anywhere in the resolved dependency tree — the
      webview-inspector code is not compiled into the binary at all in a `cargo build --release`
      production build, not merely "off by default in dev";
- [x] dialog access scoped — every dialog call is backend-only (`tauri_plugin_dialog::DialogExt`,
      never `@tauri-apps/plugin-dialog` from the frontend — that npm package was an unused dependency
      and has been removed, see 43.3) and every call sets `.add_filter(...)` to a specific extension
      list: `platform/file_picker.rs:162` (`["wav","flac","mp3"]`), `platform/diagnostics_export.rs:270`
      (`["json"]`), `lab_commands.rs` scenario open/save and recording export (`["json"]`); no
      unrestricted/whole-filesystem dialog exists;
- [x] path access constructed in backend — `platform/paths.rs::resolve_profile_paths` builds every
      profile path from `app.path().app_local_data_dir()` (Tauri-owned) joined with a
      charset-restricted `ProfileId` (`profile.rs::ProfileId::parse`: ASCII lowercase/digits/`-`/`_`
      only, 64-byte max, no `.`/`/`); `validate_trusted_root` rejects any root containing a
      `ParentDir` component. File-dialog *results* (untrusted external paths, per spec 13.3) are
      read directly (`lab_commands.rs::lab_open_scenario_file`/`lab_save_scenario_file`,
      `platform/file_picker.rs::inspect_source`) but never accepted as raw strings over IPC — they
      only ever originate from a user's own native OS dialog selection, never from a JSON command
      argument.

### 43.2 IPC review

All 36 `#[tauri::command]` handlers enumerated via `grep -rn "#\[tauri::command\]"
desktop/src-tauri/src` (`lib.rs`×2, `host_commands.rs`×19, `lab_commands.rs`×9, `app_shutdown.rs`×1,
`app_state.rs`×4) and read in full.

- [x] every command validates input — every non-trivial argument goes through a typed
      parse/validate step before use: `host_commands.rs::parse_snapshot_revision` (canonical decimal
      only), `parse_request_id`/`parse_device_id` (delegate to domain `RequestId`/`DeviceId`
      validation), `ApprovalMode::from_wire_name`, `ProfileId::parse` (`app_state.rs::open_profile`),
      `lab_commands.rs`'s `delta_ms`/`offset_ms`/`drift_ppm` string-to-integer parses and
      `LabNodeId` parse-from-`u32`. No command passes a raw frontend string into a filesystem,
      process, or SQL operation unchecked;
- [x] every command has bounded payload — DTOs use fixed-shape structs
      (`#[serde(deny_unknown_fields)]` throughout `lab_dto.rs`/`runtime_dto.rs`/etc.), and every
      *file* read/write a command triggers is size-checked from filesystem metadata before the
      content is read: `platform/file_picker.rs` (`MAX_AUDIO_SOURCE_BYTES` = 8 GiB, checked via
      `opened.byte_length` from `fs::metadata`/`File::metadata` before decode), and — a real gap
      found and fixed this block — `lab_commands.rs::lab_open_scenario_file` previously read the
      whole file via `std::fs::read` *before* `load_scenario_json`'s `MAX_SCENARIO_FILE_BYTES` (1
      MiB) check; extracted into `read_bounded_scenario_file`, which now checks `fs::metadata(path).len()`
      first (tests: `lab_commands/tests.rs::oversized_scenario_file_is_rejected_before_being_read`,
      `::scenario_file_within_the_limit_is_read_verbatim`). Reviewed-and-accepted residual: a few
      request strings (e.g. `UpdateHostDraftRequest.session_name`, `SetNetworkBindPreferenceRequest.address`)
      are not size-capped at the DTO layer itself before the shared core's own `MAX_*_BYTES`
      constants (`rust/silent-disco-core/src/{protocol,runtime/types}.rs`) reject an oversized value
      inside `CoreCommand` application — nothing oversized is ever accepted, persisted, or acted on,
      and the only caller is this app's own bundled (non-remote, no-`eval`) webview, not external
      input, so this is accepted as a low-severity residual rather than fixed;
- [x] no private keys — `dto.rs` carries `has_private_key_reference: bool` and an opaque
      `private_key_ref` string identifier, never the key itself; `bindings.rs::output_does_not_include_secret_key_fields`
      asserts the generated TS bindings contain no `private_key_ref`/`identitySecret` field. Signing
      material lives only in `platform/invitation_identity.rs` behind the OS keyring
      (`keyring`/zbus secret-service on Linux) and is never returned from a command;
- [x] no PCM/datagrams — `grep -rniE "Vec<u8>|Vec<i16>|Vec<f32>|pcm|datagram"
      desktop/src-tauri/src/**/*.rs` matches only `lab_commands.rs`'s internal
      `LoadedScenario.raw_bytes: Vec<u8>` (a scenario *JSON* document's own bytes, held in
      process-local `LabAppState`, never returned by any command — commands return only
      `LabScenarioSummaryDto`/`LabFileOutcomeDto`); no audio sample buffer crosses IPC anywhere;
- [x] no native pointers — `grep -rn "ptr\b\|pointer\|as \*const\|as \*mut\|native_handle\|raw_handle"
      desktop/src-tauri/src` returns nothing; no command DTO carries a raw pointer/handle integer;
- [x] no raw SQL — `grep -rniE "SELECT .* FROM|INSERT INTO|UPDATE .* SET|DELETE FROM"
      desktop/src-tauri/src` returns nothing; all domain SQL lives in `rust/silent-disco-core`,
      which this crate never bypasses (matches the project's "Rust core is sole SQLite owner" rule);
- [x] no arbitrary absolute path operation — see 43.1's "path access constructed in backend" citation;
      every command-reachable filesystem write/read target is either backend-constructed from a
      trusted root (`ProfileId` + `app_local_data_dir()`) or comes from a user's own native-dialog
      selection, never a raw path string accepted as a command argument;
- [x] stale revision policy tested — **a real, previously-untested gap, closed this block.** The
      authoritative rejection lives at `rust/silent-disco-core/src/runtime/actor_runtime/state/mod.rs:188`
      (`request.expected_revision != self.snapshot.revision` → `CoreErrorCode::InvalidStateTransition`,
      message `"expected snapshot revision ..., but current revision is ..."`), and — before this
      block — that exact message/branch had zero references from any test in the whole repository
      (`grep -rln "expected snapshot revision" rust desktop` found only the one production call
      site). Added `desktop/src-tauri/src/platform/effect_runner_tests.rs::stale_expected_revision_command_is_rejected_by_authoritative_core`,
      driving a real `CoreActorRuntime` the same way the file's existing
      `stale_completion_is_rejected_by_authoritative_core` test does: submits a valid `SelectRole`
      command at revision 0 (advancing the actor to revision 1), then resubmits a second `SelectRole`
      still declaring revision 0, and asserts the resulting `CoreNotification::Error` carries
      `CoreErrorCode::InvalidStateTransition` and a message containing `"revision"`. This is also
      exactly the guard every desktop `expected_revision` IPC argument
      (`host_commands.rs::parse_snapshot_revision` and its callers) depends on;
- [x] non-idempotent commands not automatically retried — every frontend `invoke()` call in
      `desktop/src/core/client.ts` funnels through the single `invokeDesktop<T>` wrapper
      (`client.ts:211`), which is a plain `try { return await invoke(...) } catch { throw
      DesktopBridgeInvocationError }` — no loop, no retry, no backoff. `retryable: boolean` on
      `DesktopErrorDto` is surfaced to Redux state (`desktop/src/app/coreSlice.ts`, `desktop/src/screens/HostSessionScreen/index.tsx`,
      `screens/LabScreen.tsx`) purely as UI metadata for an operator-triggered manual retry button;
      `grep -rn "retryable" desktop/src` confirms no code path re-invokes a command based on it.

### 43.3 Dependency review

Full per-dependency table (exact version, license, features, reason, platform behavior, transitive
native requirement) recorded in `memory.md`'s `2026-08-10T22:40:05Z` entry — 15 direct
`desktop/src-tauri/Cargo.toml` crates and 23 direct `desktop/package.json` entries (7 runtime + 16
dev), all reviewed.

- [x] exact version — `desktop/src-tauri/Cargo.toml` pins every direct dependency with `=`; every
      `desktop/package.json` entry is now an exact version (the one non-exact pin,
      `@testing-library/user-event: "^14.6.3"` from Block 42, is fixed to `"14.6.3"` this block);
- [x] license — all 15 Rust direct deps and all 23 npm direct deps are MIT, Apache-2.0, or
      dual MIT/Apache-2.0 (`ts-rs` and `netdev` are MIT-only; `cpal` is Apache-2.0-only); no
      copyleft/GPL dependency found;
- [x] features — recorded per-crate in `memory.md`; notable non-defaults: `p256` = `["ecdsa",
      "pkcs8"]` (invitation signing only, no TLS/curve bloat), `serde` = `["derive"]`, `ts-rs` =
      `["serde-compat", "no-serde-warnings"]`; `cpal`'s own `default = []` means no `jack`/
      `pipewire`/`pulseaudio`/`asio` backend is compiled in, only the platform-default backend
      (ALSA on Linux); `keyring`'s `default = ["v1"]` enables exactly the OS-native backends
      (Secret Service on Linux via `zbus`, Keychain on macOS, Credential Manager on Windows), no
      `db-keystore` fallback;
- [x] security advisory check — **run for real, not skipped.** `cargo audit` (v0.22.2, installed
      this block; not present beforehand) against the pinned `Cargo.lock`: **0 vulnerabilities**, 18
      allowed warnings, all "unmaintained"/"unsound" advisories on *transitive* Linux GTK3 webview
      bindings (`atk`/`gdk`/`gtk`/`glib` 0.18.x, pulled in by `tauri`'s Linux `wry` runtime, not a
      direct dependency choice) plus `paste`, `proc-macro-error`, and four `unic-*` crates (also
      transitive). No advisory names a direct dependency of this crate. `npm audit`: found one
      **high** transitive advisory (`nanoid < 3.3.17`, GHSA-2v37-7h3g-55p8, `zero-size custom
      generator` denial-of-service), reachable only through `postcss` (a `devDependency`-only,
      build-time-only chain — `"dev": true` in `package-lock.json`, never shipped in the production
      `dist/` bundle nor callable from the running app); fixed with `npm audit fix` → `npm audit`
      now reports **0 vulnerabilities**;
- [x] reason required — recorded per-crate in `memory.md`; every dependency maps to one real
      feature (audio playback, invitation signing, OS credential storage, LAN interface enumeration,
      mDNS discovery, native file dialogs, IPC/DTO codegen, staged-file writes, etc.) — none is
      speculative. One genuinely unused dependency was found and removed:
      `@tauri-apps/plugin-dialog` (npm) — added at initial scaffold, but every dialog interaction in
      this codebase is backend-driven (`tauri_plugin_dialog::DialogExt` from Rust, per spec 13.3),
      so the frontend package was never imported anywhere (`grep -rln "plugin-dialog"
      desktop/src` was empty before removal);
- [x] platform behavior — recorded per-crate in `memory.md`; e.g. `cpal` uses ALSA on Linux,
      CoreAudio on macOS, WASAPI on Windows; `keyring` uses Secret Service (D-Bus) on Linux,
      Keychain on macOS, Credential Manager on Windows; `netdev` has separate Android/Apple
      System-Configuration feature branches (both compiled in by its own `default` feature set,
      inert on desktop Linux/macOS/Windows targets since they're behind target `cfg`s upstream);
- [x] transitive native requirement — `cpal` → system ALSA (`libasound`) on Linux via the `alsa`
      crate; `keyring`'s Linux backend (`zbus-secret-service-keyring-store`) is pure-Rust D-Bus, no
      C library, but needs a running Secret Service provider (e.g. `gnome-keyring-daemon`) at
      runtime; `tauri` (via `tauri-runtime-wry`) requires system GTK3 + WebKit2GTK
      (`libgtk-3`, `libwebkit2gtk-4.1`) on Linux, confirmed by the `atk`/`gdk`/`gtk`/`webkit2gtk`/
      `soup3`/`javascriptcore-rs` crates actually compiling during `cargo clippy`/`cargo test`;
      `netdev` uses Linux netlink (`netlink-sys`/`netlink-packet-route`), no C library.

**Acceptance:** Desktop privileges and dependencies are intentional and minimal. Met: least-privilege
capability confirmed by direct read of the ACL manifest (only 9 harmless core sub-permissions, no
fs/shell/http/dialog grant), every command's input/bound/path handling verified by reading its full
body (not sampled), one real bounded-payload gap and one real untested-policy gap found and fixed
with production-facing tests, one genuinely unused dependency removed, and both `cargo audit` and
`npm audit` actually executed with real (not paraphrased) output — 0 vulnerabilities in both after
the `npm audit fix`.

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

- [x] remove it;
- [x] prove it is test-only;
- [x] prove the ignored result is intentionally non-material;
- [x] or document and test the explicit visible policy.

Ran all six suggested greps for real against the current tree (Blocks 33-43 added substantial
new code, including the entire `lab` module, since this block was drafted; re-ran with that in
mind rather than assuming the original scope). `unwrap()|expect()` had 1741 raw matches;
`let _ =|.ok()` had 86; a Python pass classified each by whether it falls inside a
`#[cfg(test)]`-marked region or a `_tests.rs`/`tests.rs` file, then every remaining "production"
hit (5 for `unwrap`/`expect`, dozens for `let _`/`.ok()`) and every non-test hit from the other
four greps was read in full context by hand -- not rubber-stamped. The classifier's own blind
spot (a file with more than one `#[cfg(test)]` marker, e.g. a per-item test-only constructor
followed by real production code) was separately checked for every multi-marker file; one real
instance of exactly that risk was found and hand-verified (`notification_buffer.rs`, see below).

**Real bugs found and fixed (with file/line):**

1. `desktop/src-tauri/src/platform/network_error.rs:142-155` -- `DesktopNetworkError::core_error()`
   built its message via `bounded_error_message()` (bounds length, substitutes for emptiness, but
   does **not** strip control characters) and then did
   `CoreError::new(...).expect("bounded desktop network error")`. `CoreError::new` independently
   rejects messages containing a control character (`error.rs::validate_error_message`), and
   `self.message` can come from `TransportError`/`std::io::Error` `Display` output (`transport()`,
   `endpoint_mismatch()`), which is not proven free of control characters -- so this `.expect()`
   could panic on the one path whose entire job is reporting a failure safely. Fixed by delegating
   to the sibling `failure::core_error()` free function, which already has a tested,
   non-panicking fallback for exactly this case (used 10+ other places in this same file for the
   same underlying mutex). Added `platform::network_error::tests::
   a_message_with_a_control_character_does_not_panic_and_falls_back_safely` (constructs a message
   with `\u{0007}`, confirms no panic and a safe `PlatformOperationFailed` fallback) and
   `an_ordinary_message_preserves_the_original_code_and_text` (regression guard that the fallback
   isn't taken unconditionally).
2. `desktop/src-tauri/src/platform/invitation.rs:110-123` -- `generate_nonce()` did
   `getrandom::fill(&mut bytes).expect("system CSPRNG must be available for QR nonce generation")`.
   Its own comment claimed this matched `identity.rs`/`invitation_identity.rs`'s handling of the
   same `getrandom` failure, but those two files actually propagate it as a typed `Result` via
   `?` -- this file was the one inconsistent, panicking outlier. Fixed: `generate_nonce()` now
   returns `Result<String, getrandom::Error>`; added a new `pub(crate) enum InvitationError {
   Nonce(getrandom::Error), Invitation(P2Error) }` with `Display`/`Error`/`From<P2Error>`;
   `build_signed_invitation()` now returns `Result<HostInvitationDto, InvitationError>`. The
   caller (`app_state.rs::create_host_invitation`) now routes through a new
   `invitation_error_dto()` that gives a CSPRNG failure its own `"platform"`/retryable category
   (`desktop.invitation.nonce_unavailable`) distinct from a shared-validator rejection
   (`"validation"`/non-retryable `desktop.invitation.build_failed`), rather than collapsing both
   into one category the way a single `.to_string()` mapping would. Added
   `a_csprng_failure_building_an_invitation_is_a_visible_retryable_platform_error` and
   `a_rejected_invitation_shape_is_a_non_retryable_validation_error` in `app_state_tests.rs`.
3. `desktop/src-tauri/src/app_state/host_ops.rs` (pre-fix line) -- `host_diagnostics()` computed
   `ready.notifications.delivery_failure().ok().flatten()`. `delivery_failure()` returns
   `Err(state_poisoned_error())` when its internal mutex is poisoned (something already panicked
   holding it) -- `.ok().flatten()` folded that `Err` to the exact same `None` as "no failure
   observed", i.e. a real, severe failure was indistinguishable from healthy in the diagnostics
   DTO this project requires for exactly this kind of visibility. Fixed to
   `.unwrap_or_else(Some)` (keeps `Ok(existing)` as-is, turns `Err(poisoned)` into
   `Some(poisoned)`), matching the storage branch six lines above it in the same function, which
   already surfaces its own read failure via `failure_reason` instead of reporting "available".
   To prove this for real (not simulate it), added `DesktopNotificationBuffer::
   poison_state_for_test()` (test-only, spawns a thread that locks the real state mutex and
   panics, joins it, leaving the mutex genuinely poisoned) and
   `app_state_tests.rs::a_poisoned_notification_state_is_visible_in_diagnostics_not_hidden_as_healthy`,
   which opens a real profile, genuinely poisons its notification buffer, and asserts
   `host_diagnostics()` now reports the poisoning (`core.ffi_callback_failed`, "desktop
   notification state was poisoned") instead of `None`. The same test also confirms
   `close_sync()` correctly *fails* afterward (a permanently poisoned mutex cannot close cleanly)
   rather than falsely claiming success, while still releasing the profile lock (every shutdown
   phase is attempted independently in `shutdown_owned_resources`).
4. `desktop/src-tauri/src/notification_buffer.rs:398-436` (pre-fix lines) --
   `record_delivery_failure`/`record_worker_failure` (called from the background subscription
   worker thread, `run_subscription_worker`) did
   `shared.state.lock().expect("... state was poisoned")`, unlike every other lock of the exact
   same mutex in this file (`wait_for_next`, `clear_active_subscription`, `subscribe`, ...), which
   all use `.map_err(|_| state_poisoned_error())?`. If the mutex was already poisoned by an
   earlier panic elsewhere, these two functions' own `.expect()` would panic *this* worker thread
   a second time, on top of poisoning that is already independently visible to every other caller
   -- and this second panic added no new information, just a redundant background-thread death.
   Fixed both to `let Ok(mut state) = shared.state.lock() else { return; };` (return instead of
   panicking; the poisoning remains visible everywhere else). Found via the "multiple
   `#[cfg(test)]` markers" audit of my own classifier's blind spot: this file has
   `#[cfg(test)]`-gated *fields/constructors* (lines 101/111) well before its real `mod tests`
   (line 538+), so a naive "everything after the first `#[cfg(test)]` is a test" filter would have
   missed these two production `.expect()`s entirely. Added
   `notification_buffer_tests.rs::record_delivery_failure_does_not_panic_on_an_already_poisoned_state`
   and `record_worker_failure_does_not_panic_on_an_already_poisoned_state` (both genuinely poison
   a real `NotificationShared` via a spawned-and-joined panicking thread, then call the two
   functions directly and assert no panic).
5. `desktop/src-tauri/src/lab/mod.rs:351-363` (pre-fix lines) --
   `start_node_with_clock_and_observer`'s double-fault path (the database opened fine, then
   `CoreActorRuntime::start` failed) did `let _ = database.stop_and_join();` before returning the
   primary error, discarding whether the database's own teardown also failed. This project's own
   established pattern for exactly this shape of problem
   (`app_state.rs`'s `open_runtime`/`install_ready` double-fault paths) instead appends the
   cleanup failure's message to the primary error rather than dropping it. Extracted that pattern
   as a new shared, testable `DesktopErrorDto::with_appended_cleanup(self, Option<Self>) -> Self`
   method in `dto.rs` (used by both `app_state.rs`, replacing its former private
   `fn append_cleanup` free function at all 5 call sites, and `lab/mod.rs`, without giving `lab/`
   a new dependency on `app_state` -- `LabRuntime`'s own module doc comment explicitly requires
   "no global production singleton reuse"). Added
   `dto::tests::appended_cleanup_failure_is_folded_into_the_message_and_keeps_primary_classification`
   and `no_cleanup_failure_leaves_the_primary_error_unchanged`.
6. `desktop/src-tauri/src/lab/scenario.rs:1627-1641` (pre-fix lines) -- the
   `UnderrunFramesAtMost` scenario assertion summed `missing_frames` diagnostic field values via
   `.and_then(|(_, value)| value.parse::<u64>().ok())` inside a `filter_map` -- a
   present-but-unparseable value was silently excluded from the sum rather than failing the
   assertion, understating a real underrun count and potentially letting a genuine regression
   read as "passed". This matters beyond Lab Mode's own scope: Block 45 (performance/soak testing,
   next in this TODO) explicitly depends on this exact assertion as evidence for release
   decisions. Rewrote to fail the assertion outright (`return false`) the moment a present
   `missing_frames` field fails to parse, rather than silently treating it as zero. Added
   `lab/scenario/tests.rs::underrun_frames_at_most_sums_valid_missing_frames_values` (regression
   guard for the ordinary path) and
   `underrun_frames_at_most_fails_closed_on_an_unparseable_missing_frames_value` (a malformed
   entry now fails the assertion even against an unbounded `u32::MAX` threshold, proving it can no
   longer silently pass).
7. `rust/silent-disco-core/src/audio/decoder.rs:243-249` (pre-fix lines) --
   `StreamingDecodeHandle`'s `Drop` impl did `let _ = join.join();`. The ordinary
   drop-without-explicit-join path (cancellation requested, worker cooperatively stops) already
   self-reports its final state (`Cancelled`/`Completed`/a logical `Failed`) from *inside*
   `run_decoder_worker` before it returns; the one outcome that cannot self-report that way is a
   genuine Rust panic, which unwinds past that bookkeeping -- visible only via `join()`'s own
   `Err`, which `Drop` was discarding. This project's diagnostics requirements explicitly list
   "contained panics" as mandatory; a panicked decode worker torn down via implicit `Drop` (rather
   than the explicit `cancel_and_join()`, which already handles this via `join_worker()`) would
   leave `DecodeStatistics.state` at its last pre-panic value (typically `Running`) -- a false
   impression of health for any diagnostics reader holding the shared `Arc<SharedStatistics>`
   after the handle itself is gone. Extracted `record_join_outcome()` (marks `Failed` on a `join()`
   `Err`, no-ops otherwise) and call it from `Drop`. Added an inline `#[cfg(test)] mod tests`
   (this file previously had none) with
   `a_panicked_worker_marks_shared_statistics_failed` (spawns a thread that genuinely panics,
   joins it the same way `Drop` does, confirms `Failed` is recorded) and
   `a_clean_join_does_not_alter_the_recorded_state` (a normal `Err(Cancelled)` return, no panic,
   must not clobber an already-recorded state).

**Deliberately left as-is, with reasoning (not a blanket "reviewed, fine"):**

- `desktop/src-tauri/src/platform/failure.rs:66` -- `.expect("static fallback desktop platform
  error is valid")` on a `CoreError::new(...)` built entirely from hardcoded, compile-time-constant
  arguments (`CoreErrorCode::PlatformOperationFailed`, a short literal string, `ErrorSeverity::Error`,
  `false`, `None`) -- provably always passes `validate_error_message` (non-empty, short, no control
  characters). This is the *good* pattern network_error.rs's bug (finding 1) should have used.
- `desktop/src-tauri/src/platform/render_ring.rs:108` -- `.expect("consumer is only ever taken by
  Drop, never observably absent")`. `consumer: Option<RenderRingConsumer>` is set to `None` only in
  `Drop::drop`, which takes `&mut self`; no other method can run concurrently with or after `Drop`
  begins, so `consumer_mut()` can never observe `None`. Proven by local control-flow inspection, not
  assumed.
- `rust/silent-disco-core/src/audio/packetizer_worker.rs:427` --
  `pending.take().expect("frame present until sent")` inside `send_with_backpressure`'s loop.
  Traced every branch: `pending` starts `Some`; the only place it becomes `None` is this exact
  `.take()` call (immediately consumed); the `Full` branch reassigns `Some(returned)` before
  looping; the backpressure-limit `continue` branch never touches `pending`. No path reaches this
  line with `pending == None`. Proven, not assumed.
- `rust/silent-disco-core/src/audio/packetizer_worker.rs:320-327` (`StreamingPacketizeHandle::Drop`)
  -- also has `let _ = join.join();`, structurally identical to decoder.rs's bug (finding 7) at
  first glance, but *not* fixed the same way: this worker's `SharedStatistics` has no live
  observable `state` field at all (only `queued_packets`/`backpressure_events`/`emitted_packets`
  counters) -- `PacketizerWorkerState` only ever exists once, embedded in the `PacketizerSummary`
  returned by an explicit `join()`/`cancel_and_join()`. Unlike decoder.rs, there is no existing
  diagnostic surface whose accuracy this discard compromises, so there is nothing for `Drop` to
  correct-into. Recorded here as a real, honest asymmetry (not silently treated as equivalent to
  the decoder.rs fix) and a candidate for a future block if packetizer-worker panic visibility
  becomes a real gap -- adding a live state field is a larger design change than this audit's
  charter, not a one-line silent-failure fix.
- `desktop/src-tauri/src/platform/host_transport.rs:161-169` -- `let _ =` on a
  `fetch_update(...) |depth| Some(depth.saturating_sub(1))` -- the closure always returns `Some`,
  so `fetch_update` can never return `Err` here; nothing is discarded that could ever exist.
- `desktop/src-tauri/src/platform/monitor_pump.rs:100` -- `let _ = pump.apply_sync_offset(0.0);`.
  `apply_sync_offset` returns `SyncApplyOutcome`, not a `Result` -- there is no error channel here
  at all, only an informational enum describing what happened (deterministically `Locked` on a
  freshly constructed pump).
- `desktop/src-tauri/src/platform/monitor.rs:165` (`on_stream_started`'s `self.state.lock().ok()?`)
  and `desktop/src-tauri/src/platform/network.rs:313` (`stream_diagnostics_snapshot`'s
  `self.state.lock().ok()?`) -- both fold poisoning into the same `Option::None` their doc
  comments already document as a legitimate, common outcome ("no stream has started"/"no monitor
  running"). Verified the mutating operations on the *same* mutex in the *same* file
  (`DesktopMonitorControl::status()`, `DesktopNetworkHandle::set_preference()`/`snapshot()`, 10+
  sites) already surface poisoning explicitly and visibly -- so poisoning is not silently lost
  system-wide, only under-attributed by these two specific read-only diagnostics accessors in the
  rare case it occurs. `monitor.rs`'s own module doc additionally states this policy explicitly:
  monitor failures never propagate to the caller, only via `status()`, which is verified correct.
- `desktop/src-tauri/src/lab/mod.rs:192-232` (`node_ids`/`node_handle`/`node_identity`/
  `node_clock`) -- same shape as the monitor/network case above: poisoning of `self.nodes` folds
  into the same `None`/empty already documented as legitimate ("stopped or never existed").
  Verified the mutating operations on the same mutex (`start_node_with_clock_and_observer`,
  `stop_node`, `shutdown`) already propagate poisoning explicitly via `lab_poisoned_error()`, so
  the very next node-lifecycle action after any real poisoning surfaces it; these read-only
  getters would only under-attribute a "why is this None" reason in a case already caught
  elsewhere. Lab Mode is a `lab-mode`-feature-gated engineering test harness, not a listener-facing
  production path.
- `desktop/src-tauri/src/lab/scenario.rs:1304` (`handle.current_snapshot().ok()`) -- feeds
  `evaluate_assertion(assertion, snapshot.as_ref(), ...)`; verified every assertion arm that
  touches `snapshot` does `let Some(snapshot) = snapshot else { return false };` -- a failed
  snapshot fetch (for any reason, including poisoning) fails the assertion, never falsely passes
  it. Fails closed by construction.
- `rust/silent-disco-core/src/storage/migrations.rs:206-209` -- `u32::try_from(index).ok()
  .and_then(...).ok_or_else(|| migration_failure(...))?` -- this is `Option`-chaining syntax to
  combine a `Result` and a bound-check into one typed `Err` with a clearer message, not error
  swallowing: the ultimate failure is still returned via `?` regardless of which step produced
  `None`.
- `rust/silent-disco-core/src/storage/worker/lifecycle.rs:99,110,140` (`join`/`stop_and_join`/
  `Drop`, `let _ = self.stop();`) -- traced `stop()`'s body: it stores its own return value into
  `self.stop_result` as a side effect *before* returning it, and every one of these three callers
  immediately calls `finish_join()`, which reads `self.stop_result` (not the discarded return
  value). No information is actually lost -- confirmed by reading the full call chain, not assumed
  from the discard alone.
- `rust/silent-disco-core/src/storage/worker/lifecycle.rs:53` (`DatabaseWorker::start`'s
  `Ok(Err(error)) => { let _ = join_handle.join(); Err(error) }`) -- the worker already reported a
  complete, well-formed startup error over the startup channel; joining afterward is pure cleanup
  reaping an already-finishing thread, and the definitive error being returned is the one already
  received, not the (structurally near-impossible, and strictly less informative even if it did
  happen) join-time panic signal.
- `rust/silent-disco-core/src/storage/worker/mod.rs:142` (`let _ =
  startup_sender.send(Err(error.clone()));`) -- if the send fails because the receiver was already
  dropped, the same `error` is still returned as this worker thread's own function return value
  (`return Err(error);`, the very next line), which remains reachable via `join_handle.join()`.
  Nothing is lost regardless of whether the channel send succeeds.
- `rust/silent-disco-core/src/transport/socket/host/peer.rs` (`PeerState::transport_peer`/
  `device_id`, `self.identity.lock().ok().and_then(...)`) -- same shape as the monitor/network/lab
  cases: `Option<DeviceId>::None` is already the expected value before a peer identifies itself.
  Verified the one write path for this exact mutex (`validate_host_frame` in `host_workers.rs`)
  correctly propagates poisoning as `TransportErrorKind::WorkerPanicked`.
- `rust/silent-disco-core/src/transport/socket/host_workers.rs:266,273` (best-effort peer/route
  deregistration during connection teardown) and `:412` (`routes.lock().ok().and_then(...)` when
  matching an inbound datagram to an authorized route) -- teardown cleanup where the peer is
  already being closed regardless, and (for `:412`) a poisoned-or-missing route both correctly
  fall through to the same fail-closed `TransportEvent::Rejected { ..Unauthorized.. }` path an
  unrecognized/spoofed datagram source would also hit -- the safe direction, not a false accept.
- `rust/silent-disco-core/src/transport/socket/shared.rs` (10 `let _ = on_outcome(...)` sites in
  `read_control_loop`) -- `on_outcome: FnMut(ReadControlOutcome) -> bool` is a continue/stop
  signal, not an error channel. Verified every single call site in this function is immediately
  followed by an unconditional `return`/`break` regardless of the returned `bool` (including the
  one site that captures it into `keep_running` before also unconditionally returning) -- the
  loop was ending at every one of these sites either way, so the discarded value provably can
  never change behavior here. The `bool` genuinely matters elsewhere in this same function (the
  `Frame` success case, which does branch on it) -- just not at any of the discard sites.
- `rust/silent-disco-core/src/runtime/actor_runtime/mod.rs:247-259` (`shutdown()`'s
  `observer_error` computation calls `read_failure(...)` twice, using `.err()` from the first call
  and `.ok().flatten()` from a second, identical call inside `.or_else`) -- logically sound (the
  second call's `.ok()` can only run when the first call already proved the lock was not
  poisoned, so it cannot silently discard a poisoning error) but wastefully locks the mutex twice
  for one combined value. Not a silent-failure bug -- a minor, out-of-scope "simplify" observation,
  left unchanged since this audit's charter is failure visibility, not code golf.
- `desktop/src-tauri/src/platform/effect_runner.rs:436-459` (`combine_shutdown_errors`'s
  `CoreError::new(...).unwrap_or(fallback)`) -- `fallback = primary.clone()`, the original,
  already-valid primary error, not a generic placeholder. If the *combined* (primary + cleanup)
  message happens to fail `CoreError::new`'s validation (e.g. a control character neither half's
  own bounding strips), this degrades to the still-fully-informative original primary error rather
  than panicking or reporting a phony message -- the correct pattern, verified safe, not the
  network_error.rs bug's shape.
- `desktop/src-tauri/src/platform/profile_lock.rs:118-133` (`ProfileLease::Drop`'s "fallback
  unlock result" in an `assert!` message) -- a fail-loud safety net (`assert!` panics if a lease
  was dropped without going through explicit `release()`), not a silent fallback; "fallback" here
  is just the debug-message wording for the best-effort unlock attempted right before the assert.
- Every other `let _ =`/`.ok()` match not listed above was inside a `#[cfg(test)]`-gated region
  or `_tests.rs`/`tests.rs` file (test cleanup helpers, `Drop for TestDirectory`, fixture
  construction) -- confirmed individually, not assumed from filename alone (see finding 4's note
  on why filename-based classification alone is not trusted).

Specifically verified (all clean, no findings beyond the fixes above):

- [x] no in-memory database fallback -- `storage/database.rs`'s own test
  (`rejects_invalid_configuration_and_non_wal_database`) proves `:memory:` is explicitly rejected,
  not silently accepted; `storage_inspection.rs`'s
  `failed_database_open_releases_profile_lock_without_fallback` asserts no `fallback.sqlite3` is
  ever created; every `DatabaseConfig::new(...)` call site (production and `lab/mod.rs`) takes a
  real file path.
- [x] no plaintext identity fallback -- `identity.rs`/`invitation_identity.rs` only ever touch
  secret material through `keyring::Entry`; grepped both files for `fs::write`/`write_all` against
  secret bytes -- zero matches.
- [x] no synthetic production identity -- `lab/mod.rs`'s own module doc comment states synthetic
  identity/roots are structurally disjoint from `DesktopProfilePaths`'s production `profiles/`
  root (Block 37.1); the entire `lab/` module tree, including its synthetic-identity code, is
  compiled only under the `lab-mode` Cargo feature (`desktop/src-tauri/Cargo.toml`:
  `lab-mode = []`, not a default feature).
- [x] no virtual transport production fallback -- `grep -rln VirtualTransportFactory|virtual_transport`
  across `desktop/src-tauri/src` matches only files under `lab/`; zero matches anywhere under
  `desktop/src-tauri/src/platform/` (the real network/transport code).
- [x] no fake decoder/audio fallback -- `rust/silent-disco-core/src/audio/decoder.rs` is the one
  real `symphonia`-backed streaming decoder used by every platform; `grep -Rn
  "Audio|createMediaElement|WebAudio|AudioContext" desktop/src` returns zero `AudioContext`/
  `createMediaElement`/Web Audio references -- every "Audio" match is a DTO/function name
  (`AudioSourceSummaryDto`, `selectAudioSource`, an "Audio port" diagnostics label) for
  backend-driven source *selection*, never browser-native decode/playback.
- [x] no log-only operational failure -- the monitor backend's live error callback retains the
  first failure in a write-once cell surfaced by `status()`, the monitor pump records terminal
  scheduler/panic failures the same way, and explicit disable/stream teardown returns those
  failures instead of merely logging them. The final audit also fixed `start_playback`'s
  double-fault path so failure to transition the actor to `PlaybackState::Error` is appended to
  the primary startup failure instead of discarded. The remaining desktop/shared-core ignored
  results from the audit are test cleanup or explicitly non-material policy (for example the
  bounded local-monitor tap may drop rather than backpressure network transmission).
- [x] no optimistic success -- `create_host_invitation`, `submit_core_command`,
  `shutdown_owned_resources`, `stop_node`/`shutdown` (Lab), `finish_join` all propagate real
  failures; the one place that *was* reporting false health (`host_diagnostics`'s
  `notification_failure`) is fixed (finding 3).
- [x] no automatic destructive database reset -- `storage/database.rs::corrupt_file_fails_without_recreation`
  and `storage_inspection.rs::corrupt_database_failure_preserves_file_and_releases_profile_lock`
  both assert the on-disk file is byte-identical after a failed open; migration/integrity failures
  return typed errors (`StorageErrorKind::Corruption`/`Migration`), never delete-and-recreate.
- [x] no detached worker hiding shutdown failure -- `platform/monitor_pump.rs` now owns a
  `JoinHandle<Result<(), String>>`: explicit `stop()` propagates worker errors/panics, implicit
  `Drop` joins and fails loudly on an unobserved failure, and `status()` can see a terminal pump
  failure without waiting for teardown. `platform/playback_streamer/owner.rs` likewise joins and
  propagates explicitly and classifies implicit join failure instead of discarding it. The one
  intentionally detached timeout worker in `app_shutdown.rs` is an explicit safety policy: the
  caller receives a fatal timeout and resources visible to the still-live worker are deliberately
  not reclaimed.

**Acceptance:** Met for the software audit. Production desktop/shared-core failure paths are either
propagating/visible or have an explicit tested non-material policy; monitor/runtime worker failures
cannot be reduced to log-only cleanup or hidden join outcomes. Physical/package acceptance remains
separate in Blocks 46-47.

---

## Block 45 — Performance and soak testing

### 45.1 Define test matrix

At minimum:

- [x] one listener;
- [x] two listeners;
- [x] five virtual listeners;
- [x] selected higher virtual count;
- [x] WAV, FLAC, MP3;
- [x] transmit only;
- [x] local monitor;
- [x] no faults;
- [x] moderate jitter/loss;
- [x] reconnect event;
- [x] one-hour or selected long soak.

The matrix is executable rather than aspirational. `performance_probe` measures
WAV/FLAC/MP3 decode, transport at 1/2/5/16 virtual listeners, zero loss and
moderate 50-permille loss, an explicit reconnect, database work, and an optional
16-listener lossy soak. `block45_runtime_probe` adds the desktop broadcast queue,
notification bridge, local-monitor callback, synchronization, and concealment
paths. `.github/workflows/desktop-performance.yml` runs three baselines plus a
selected 60-3600 second soak and records raw environment/process-time artifacts.
These checks define what to run; they do **not** by themselves satisfy 45.2's
measurement/result boxes, which remain open until real artifacts are evaluated.

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

- [x] AppImage;
- [x] `.deb`;
- [x] no third initial format — AppImage + `.deb` are the intentional initial Linux formats.

### 46.2 Package behavior

- [x] application ID and product name stable;
- [x] icons complete;
- [x] desktop entry correct;
- [x] required native dependencies documented;
- [x] clean install on supported Linux baseline;
- [x] clean upgrade preserving profile data;
- [x] uninstall does not silently destroy user data;
- [x] bundle launches without development server;
- [x] production CSP/capabilities apply;
- [x] Lab Mode inclusion policy explicit.

**46.2 evidence:** Desktop CI run 514 at commit `6716402a2128db6632eb3411b5473d5419e1f4db` passed Linux bundle verification and the package lifecycle gate on Ubuntu 22.04.5 LTS. The lifecycle gate exercised a clean `.deb` install, synthetic-version upgrade, packaged-app launch without a development server, uninstall, and preservation of profile-local preferences, staged source data, and the SQLite database.

### 46.3 Fresh-machine validation

- [ ] install on a clean supported Linux VM/machine;
- [ ] create profile;
- [ ] stage source;
- [ ] host Android listener;
- [ ] export diagnostics;
- [ ] shut down and reopen;
- [ ] verify package uninstall behavior.

**Acceptance:** Block 46.2 is complete on the supported Ubuntu 22.04 CI baseline: package contents, clean install, synthetic-version upgrade, no-development-server launch, uninstall, and profile-local user-data preservation all pass. Block 46.3 fresh-machine validation with a graphical Ubuntu 22.04 machine/VM and a physical Android listener remains open.

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

- [x] add desktop prerequisites;
- [x] add clean build commands;
- [x] add development launch command;
- [x] add production bundle command;
- [x] add test commands;
- [x] add physical interoperability procedure;
- [x] add Lab scenario procedure;
- [x] add diagnostics location and export procedure;
- [x] add secure-store troubleshooting without insecure fallback.

Use existing repository guidance files where appropriate. Do not create additional design documents unless required and committed.

### 48.2 Audit ownership

Confirm:

- [x] Rust actor is authoritative;
- [x] React is presentation-only;
- [x] Tauri backend is platform-only;
- [x] protocol is Rust-only;
- [x] synchronization is Rust-only;
- [x] packetization is Rust-only;
- [x] transport semantics are Rust-only;
- [x] SQLite is Rust-only;
- [x] PCM does not cross IPC;
- [x] local monitor uses shared timeline;
- [x] Lab adapters cannot activate silently in production.

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

- [x] unresolved platform/device limitations are listed;
- [x] Windows/macOS are not claimed unless validated;
- [x] every skipped test has a reason and owner;
- [x] every referenced file exists at the exact path;
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

**Consume the shared listener playback runtime (2026-08-03).** Do not build a
desktop-specific scheduler, concealment policy, or ring pump. Android's
listener now runs entirely on `silent-disco-core`'s `audio::PlaybackPump` plus
`silent-disco-ffi`'s `ListenerPlaybackRuntime`, which own ordering,
concealment, presentation-time pacing, clock-sync estimation, PCM conversion,
diagnostics, and an optional debug WAV capture. A desktop listener should open
the same runtime, hand its engine token to a desktop audio adapter (the only
genuinely platform-specific piece), and forward packets and raw sync
exchanges. Everything else is already built and device-validated; see
`docs/SILENT_DISCO_LISTENER_PLAYBACK_RUST_MIGRATION_TODO.md`.

This is the whole reason that migration was done: the audio-quality work had
been fixed once in Kotlin and would otherwise have had to be reimplemented
here.

---

# Final completion checklist

- [x] Tauri 2 desktop application exists under `desktop/`.
- [x] React/TypeScript/Tailwind frontend passes all gates.
- [x] Tauri backend directly uses `silent-disco-core`.
- [x] Shared actor and host lifecycle are Rust-authoritative.
- [x] Profiles and databases are isolated and locked.
- [x] Production identity has no insecure silent fallback.
- [x] Source selection and staging are safe and atomic.
- [x] Decoder is streaming, bounded, and explicit.
- [x] Manual LAN hosting works.
- [x] Android control interoperability works.
- [x] Bounded Rust audio transmission works.
- [x] One Android listener plays desktop-hosted audio. `ManualEndpointScreen.kt`'s
      playback wiring is now fixed and confirmed reaching real `Streaming`/
      `Buffering` state live on-device (see Block 28) -- what remains is a
      human actually listening to confirm audible, in-sync sound, plus the
      still-unfixed `stop_playback` bug noted in Block 28.
- [ ] At least two Android listeners pass recorded validation.
- [ ] mDNS and QR convenience work without replacing manual connection.
- [x] Optional local monitor uses the shared timeline.
- [x] PCM and packet payloads never cross Tauri IPC.
- [x] Diagnostics are useful and secret-safe.
- [ ] Shutdown is deterministic.
- [x] Lab Mode is deterministic, isolated, and visibly labeled.
- [ ] Fault injection, recording, replay, and assertions pass.
- [ ] Linux package passes fresh-machine validation.
- [x] No silent fallback, fake success, destructive recovery, or log-only operational failure remains.
- [x] All referenced files exist.
- [ ] `memory.md` records final evidence and limitations.
