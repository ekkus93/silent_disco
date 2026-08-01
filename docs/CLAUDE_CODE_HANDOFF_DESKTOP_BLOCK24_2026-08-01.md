# Claude Code Handoff — Desktop Block 24 Physical Android Control Interoperability

**Date:** 2026-08-01  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Starting commit before this handoff document:** `7c0b8db3d42d70338b4563ff408fe4cbb1a0b0ec`  
**Primary TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`  
**Primary specification:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md`  
**Shared-core TODO:** `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`  
**Shared-core specification:** `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`

---

## 1. Purpose of this handoff

The user is switching from ChatGPT to Claude Code and will give Claude Code access to a physical Android phone. The next project task is **Desktop Block 24 — First physical Android control interoperability**.

This is not merely a manual test task. Repository inspection found a real implementation gap that must be closed before the physical acceptance run can succeed:

- the desktop host now exposes a standard LAN endpoint using the shared Rust socket transport;
- the Android listener UI currently offers nearby discovery and signed QR workflows, but no manual endpoint entry path;
- the current Android listener connection path is still centered on `WifiDirectTransportService` and Wi-Fi Direct peer discovery;
- the shared Rust core already contains the production `SocketListenerTransport`, but it is not currently exposed to Android through the UniFFI boundary.

Claude Code should Ralph Loop Block 24 from the current `master`, implement the smallest architecture-correct interoperability slice, validate it completely, run the physical phone test, and record evidence. Do not bypass the shared Rust transport by adding a second ad hoc Kotlin socket protocol.

---

## 2. Current repository state

### 2.1 Desktop Block 23 is complete

Desktop Block 23 — listener approval and management — completed successfully.

- Completion commit: `7c0b8db3d42d70338b4563ff408fe4cbb1a0b0ec`
- Commit message: `Complete desktop listener management (Desktop Block 23)`
- Successful GitHub Actions run: `30678111276`
- Successful job: `91309389781`
- Completion evidence: `docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md`

The completed validation covered:

- revision-aware approve, reject, and listener removal commands;
- real pending-control delivery rather than optimistic UI mutation;
- trusted-device persistence ordering;
- authoritative Rust snapshot reconciliation in the desktop UI;
- strict desktop Rust formatting and Clippy;
- desktop frontend checks and tests;
- Linux Tauri bundle creation;
- shared Rust tests;
- Android builds, JVM tests, lint, ABI packaging, and instrumentation.

The temporary Block 23 publishing and diagnostic workflows were removed as part of completion. Do not restore them for Block 24.

### 2.2 Block 24 is the next incomplete desktop block

`docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` currently defines Block 24 as follows:

- record desktop, Android device, network, firewall, build-SHA, and command evidence;
- create a desktop host session;
- connect Android through the desktop manual endpoint;
- receive a join request on desktop;
- approve successfully;
- exercise rejection in a separate run;
- make disconnect visible on both sides;
- make desktop end-session visible on Android;
- fail clearly for an invalid endpoint;
- fail clearly for a wrong protocol version;
- preserve diagnostics and add regressions for every defect found;
- do not claim audio interoperability.

The formal acceptance condition is:

> One Android listener completes a real control-plane session with the desktop host over the LAN.

### 2.3 No Block 24 implementation changes were made during the ChatGPT session

This handoff document is the only intended repository change from this session. The preceding work was repository inspection and architecture analysis. Block 24 checklist items must remain unchecked until real code and physical-device evidence justify completion.

---

## 3. Important findings from repository inspection

### 3.1 Android device requirement

The Android application currently declares:

- `minSdk = 29`
- `targetSdk = 36`
- application ID `com.ekkus.silentdisco`

The physical phone used for Block 24 must therefore run **Android API 29 or newer**. Reject an incompatible device explicitly rather than attempting to lower `minSdk` as part of Block 24.

Before doing implementation or device work, record:

```bash
adb devices -l
adb shell getprop ro.product.manufacturer
adb shell getprop ro.product.model
adb shell getprop ro.build.version.release
adb shell getprop ro.build.version.sdk
adb shell getprop ro.product.cpu.abi
```

Also distinguish a real physical phone from an emulator. Block 24 requires physical-device evidence.

### 3.2 Desktop host transport is the shared Rust standard-IP transport

The desktop host uses the shared Rust production socket transport through:

- `rust/silent-disco-core/src/transport/socket/host.rs`
- `rust/silent-disco-core/src/transport/boundary.rs`
- `rust/silent-disco-core/src/transport/types.rs`
- `desktop/src-tauri/src/platform/host_transport.rs`
- `desktop/src-tauri/src/platform/host_transport_events.rs`
- `desktop/src-tauri/src/platform/host_pending_handshake.rs`

The desktop endpoint contains:

- host IP address;
- TCP control port;
- UDP synchronization port;
- UDP audio port;
- session ID;
- protocol version;
- invite-code requirement;
- optional expiration.

The frontend projection is `HostConnectionDto` in:

- `desktop/src-tauri/src/host_session_dto.rs`

The desktop host accepts a Rust `ControlMessage::JoinRequest`, registers the pending listener, and sends a bounded pre-approval `Hello` over the identified TCP peer. Approval, rejection, and removal use delivery-first semantics through the shared Rust actor and transport.

### 3.3 The Android listener does not currently have a manual endpoint workflow

The relevant Android screens are:

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/NearbySessionsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/SessionJoinScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

`NearbySessionsScreen` currently exposes:

- nearby session discovery;
- refresh;
- signed QR scanning;
- selection of a discovered `SessionInfo`.

It does **not** expose a manual LAN endpoint form or paste/import action.

`SessionInfo` in `app/src/main/java/com/ekkus/silentdisco/core/model/Models.kt` currently contains only:

- ID;
- session name;
- host device name;
- approval mode;
- invite-code-required flag.

It does not contain the desktop endpoint address, ports, or protocol version.

### 3.4 The current Android join path requires a discovered Wi-Fi Direct session

The relevant code is:

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModelListenerActions.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt`

Current behavior in `requestJoinImpl()`:

1. requires persistent storage readiness;
2. requires a selected `SessionInfo`;
3. rejects the join if the selected session is not still present in `discoveredSessions`;
4. validates the invite code;
5. constructs a Kotlin `ControlMessage.JoinRequest`;
6. uses demo simulation for debug demo sessions;
7. otherwise calls `wifiDirectService.connectToSession(session)`.

`WifiDirectTransportService.connectToSession()` requires a matching Wi-Fi Direct peer and then opens its Kotlin TCP channels after Wi-Fi Direct group formation.

This cannot satisfy the Block 24 manual LAN endpoint requirement as written. A manually entered desktop host will not be in `discoveredSessions`, will not resolve to a Wi-Fi Direct peer, and will not use the desktop’s shared Rust listener transport path.

### 3.5 The shared Rust listener transport already exists

The core already contains:

- `rust/silent-disco-core/src/transport/socket/listener.rs`
- `SocketListenerTransport::connect(...)`
- `ListenerTransportConfig`
- `ListenerTransportNode`
- `production_transport_factory().connect_listener(...)`

This transport:

- connects the TCP control channel to the desktop endpoint;
- binds local UDP synchronization and audio routes;
- sends shared Rust protocol frames;
- validates session identity and listener identity;
- emits typed transport events;
- provides bounded queues and explicit shutdown;
- already matches the desktop host transport contract.

However, the current UniFFI exports in `rust/silent-disco-ffi/src/lib.rs` expose the host actor, storage, synchronization smoke APIs, and related records, but not a production listener transport handle suitable for Android.

### 3.6 Do not assume the existing Kotlin transport is wire-compatible

The Android Wi-Fi Direct path has its own Kotlin TCP channel and codec implementation. The desktop host uses the shared Rust `ProtocolFrame` encoder/decoder and `SocketHostTransport`.

Do not assume these independent implementations are interchangeable merely because similarly named Kotlin and Rust message types exist. Prove wire compatibility with a cross-language golden test before reusing the Kotlin transport. The safer and architecture-consistent Block 24 path is to expose and use the existing Rust `SocketListenerTransport` from Android.

---

## 4. Non-negotiable architecture constraints

These constraints come from the desktop and shared-core specifications and must remain true:

1. Shared Rust owns protocol framing, transport validation, and transport state semantics.
2. Do not create a second Kotlin TCP/UDP implementation for the desktop manual endpoint.
3. Do not make Compose or `MainViewModel` authoritative for protocol or transport state.
4. Do not silently fall back to Wi-Fi Direct, virtual transport, demo sessions, or fake approval when manual LAN connection fails.
5. Do not report connection, join, approval, rejection, disconnect, or end-session success before the real Rust transport and actor report completion.
6. Do not report zero-recipient approval or rejection delivery as success.
7. Use bounded queues, bounded text fields, validated ports, validated IDs, and explicit timeouts.
8. Do not pass private keys, native pointers, or raw unbounded diagnostics through the UI layer.
9. Preserve existing nearby discovery and QR workflows unless a tested refactor intentionally unifies them.
10. Block 24 is control-plane only. Do not claim audio streaming, synchronization quality, or playback interoperability.
11. Keep `master` buildable at each committed slice.
12. Do not add `Co-Authored-By:` lines; the repository rejects them.
13. Do not mark checklist items complete from emulator-only or CI-only evidence.

---

## 5. Recommended Block 24 Ralph Loop

Claude Code should inspect the current repository before applying this plan and adjust names or boundaries when the implementation has changed. The architectural outcome matters more than copying these suggested class names literally.

### Slice 1 — Establish the exact baseline

1. Pull the current `master` and confirm the HEAD after this handoff commit.
2. Read the four primary specification/TODO files listed at the top.
3. Read `docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md` and `docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md`.
4. Record the physical phone properties and API level.
5. Record desktop OS, hardware, active interfaces, routes, and firewall state.
6. Run the permanent baseline checks before editing.

Suggested commands:

```bash
git status --short
git rev-parse HEAD
git branch --show-current

uname -a
cat /etc/os-release
ip -brief address
ip route
ss -ltnup
sudo ufw status verbose || true

adb devices -l
adb shell getprop ro.product.manufacturer
adb shell getprop ro.product.model
adb shell getprop ro.build.version.release
adb shell getprop ro.build.version.sdk
adb shell ip -brief address || adb shell ip addr
```

### Slice 2 — Define the bounded manual endpoint contract

Create or extend a shared boundary record that represents exactly the desktop data required by the Android listener:

- address;
- control port;
- synchronization port;
- audio port;
- session ID;
- protocol version;
- invite-code requirement;
- safe display/session name if required by UI;
- optional expiration if the desktop supplies it.

Validation must reject:

- blank or malformed addresses;
- multicast, unspecified, or otherwise unsupported destinations according to the approved policy;
- port zero and out-of-range values;
- duplicate ports if the core contract prohibits them;
- invalid or oversized session IDs;
- unsupported protocol versions;
- expired endpoint data;
- extra unknown fields if a serialized copy/paste format is introduced.

Prefer one canonical copy/paste representation generated from authoritative data rather than six unrelated text fields with inconsistent parsing. A structured manual form may still be provided for debugging, but it must feed the same validator.

Do not invent a new protocol version. Use the actual version advertised by the desktop/session record and reject mismatches before claiming a connection attempt.

### Slice 3 — Expose the shared Rust listener transport through UniFFI

Implement the smallest bounded Android-facing listener transport adapter in `silent-disco-ffi` around the existing shared Rust production transport.

Likely responsibilities:

- open/connect using a validated endpoint, session ID, device ID, and local address policy;
- expose local synchronization/audio routes needed by the host handshake;
- send the shared Rust `JoinRequest`;
- receive the host `Hello`, `JoinApproval`, `JoinRejection`, `Disconnect`, and end-session/stop message actually used by the protocol;
- expose typed events or snapshots to Kotlin;
- expose counters and a bounded diagnostic summary;
- make shutdown idempotent and join all workers;
- surface timeout, protocol, unauthorized-session, peer-closed, and I/O failures distinctly;
- never detach a background worker whose failure becomes log-only.

Before designing a new listener state machine, inspect whether the shared actor already provides the required listener-side commands/events. Reuse it when available. If the listener actor migration is incomplete, keep the adapter narrowly transport-oriented and document the remaining ownership boundary; do not move new domain policy into Kotlin.

Add Rust tests for:

- valid loopback/manual endpoint connection;
- invalid address and port;
- wrong session ID;
- wrong protocol version;
- host unavailable/connection refused;
- host closes before approval;
- approval received;
- rejection received;
- disconnect received;
- queue saturation;
- repeated close;
- worker shutdown/join;
- no event delivery after close.

### Slice 4 — Add Android manual endpoint UI and state

Add a clear manual connection entry point to the listener workflow. A reasonable location is `NearbySessionsScreen`, alongside refresh and QR scan.

Required UX behavior:

- action labeled clearly, such as `Enter host details` or `Connect manually`;
- bounded input or paste field;
- validation errors next to the affected data;
- protocol mismatch shown before or during connection with an actionable explanation;
- no requirement for nearby-device/Wi-Fi Direct permission when using ordinary LAN manual connection unless a platform-specific permission is genuinely required;
- no requirement that a manual session be present in `discoveredSessions`;
- cancel returns to a stable listener state and closes the Rust transport;
- retry uses the same validated endpoint and does not silently switch transports;
- UI state derives from typed transport/actor results rather than optimistic transitions.

Refactor `SessionInfo` only if it remains an accurate domain type. It may be cleaner to introduce a separate `ManualHostEndpoint` or `ListenerConnectionTarget` rather than overloading discovery-only data.

Add Compose/JVM tests for:

- manual entry action visible and accessible;
- valid paste/input enables connection;
- invalid address;
- invalid port;
- unsupported protocol version;
- invite-code-required endpoint;
- cancellation;
- retry;
- manual target does not require membership in `discoveredSessions`;
- screen-reader labels and keyboard behavior;
- no demo-session simulation for a real manual endpoint.

### Slice 5 — Integrate control-plane lifecycle

The physical path must produce these real state transitions:

1. Android connects to the desktop TCP control endpoint.
2. Android sends a shared Rust `JoinRequest` for the exact session ID.
3. Desktop displays the pending request.
4. Desktop sends `Hello` over the pending peer.
5. Desktop approval is delivered to the identified Android peer.
6. Android displays approval/connected state only after receiving the real message.
7. Desktop rejection is delivered and displayed clearly in a separate run.
8. Desktop removal/disconnect is visible on Android.
9. Android leave/disconnect is visible on desktop.
10. Desktop end-session or stop is visible on Android.

Preserve delivery-first semantics. A desktop UI click is not evidence of Android receipt.

### Slice 6 — Run automated validation

Run the relevant permanent matrix before the physical test and again after every defect fix.

At minimum:

```bash
bash scripts/check-source-file-line-counts.sh

cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

cd ..
./gradlew test
./gradlew lintDebug
./gradlew assembleDebug
./gradlew assembleAndroidTest

cd desktop
npm ci
npm run bindings:check
npm run format:check
npm run lint
npm run typecheck
npm test -- --run
npm run build

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo check --all-features
```

Run `npm run tauri build` when desktop/Tauri code or generated bindings change, and retain Linux bundle evidence.

### Slice 7 — Install and run on the physical phone

Build and install the exact tested APK:

```bash
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell pm clear com.ekkus.silentdisco
adb shell monkey -p com.ekkus.silentdisco -c android.intent.category.LAUNCHER 1
```

Use `adb shell am start` instead of `monkey` if the exact launcher activity command is preferred.

Capture logs in a bounded file during each run:

```bash
mkdir -p /tmp/silent-disco-block24
adb logcat -c
adb logcat -v threadtime > /tmp/silent-disco-block24/android-logcat.txt
```

Also capture desktop terminal output or structured diagnostics. Do not commit raw logs containing unrelated device/user data; extract and sanitize only the evidence needed for the repository.

### Slice 8 — Execute the physical acceptance matrix

Use the desktop and phone on the same routable LAN. Avoid guest-network client isolation. Confirm the phone can route to the desktop address before debugging application protocol.

Record:

- desktop model and OS;
- phone manufacturer/model and Android/API version;
- exact repository SHA used for both builds;
- router/access point;
- desktop Ethernet/Wi-Fi connection;
- desktop IP;
- phone IP;
- firewall rules;
- endpoint ports displayed by desktop;
- exact launch/install commands;
- timestamps for each scenario.

Run these scenarios separately:

#### A. Approval success

- desktop starts a real host session;
- use a small local audio file if the host setup UI requires a source, but do not start playback;
- Android enters/pastes the exact endpoint;
- Android requests access;
- desktop shows the real pending request;
- desktop approves;
- Android confirms real approval receipt;
- desktop shows the connected listener;
- capture both sides’ diagnostics.

#### B. Rejection

- start a fresh session or fresh listener identity/state;
- Android requests access;
- desktop rejects;
- Android displays a clear rejection and a safe recovery action;
- desktop does not show the listener as connected.

#### C. Listener disconnect

- establish approval;
- disconnect/leave from Android;
- desktop reflects disconnection;
- no stale connected-listener row remains.

#### D. Host removal or disconnect

- establish approval;
- remove/disconnect the listener from desktop;
- Android reflects the real disconnect message;
- Android does not remain in an approved/connected state.

#### E. Desktop end-session

- establish approval;
- end the desktop session;
- Android reports host/session termination distinctly from a transient network error.

#### F. Invalid endpoint

- enter an unroutable or closed endpoint safely;
- Android times out within the bounded policy;
- error identifies connection failure and supports retry/edit;
- no success state is emitted.

#### G. Wrong protocol version

- alter only the entered protocol version or use a dedicated test fixture;
- Android rejects the mismatch clearly;
- no connection or join success is claimed.

### Slice 9 — Fix defects and add regressions

For every physical-test failure caused by code:

1. capture the first concrete failure;
2. identify the owning layer;
3. add the smallest automated regression test;
4. implement the correction;
5. rerun the focused test;
6. rerun the complete required matrix;
7. rerun the physical scenario.

Do not paper over failures with delay increases, broad exception handling, `runCatching` that only logs, or optimistic UI updates.

### Slice 10 — Record evidence and close Block 24

Create a permanent evidence document, suggested path:

- `docs/DESKTOP_BLOCK24_ANDROID_CONTROL_INTEROPERABILITY.md`

Include:

- exact source and completion SHAs;
- phone and desktop details;
- network topology and firewall state;
- endpoint values with any sensitive values redacted appropriately;
- commands;
- automated test results;
- physical scenario results;
- defects found and regression tests added;
- known limitations;
- an explicit statement that audio interoperability was not claimed.

Update `memory.md` with material decisions and exact evidence.

Mark Block 24 checklist items `[x]` only after the physical scenarios pass. Commit and push the completed block. Ensure no temporary workflows, scripts, logs, APKs, generated caches, or device artifacts remain tracked.

---

## 6. Suggested implementation ownership boundaries

A likely clean boundary is:

### Shared Rust core

Owns:

- endpoint validation primitives where platform-independent;
- protocol version validation;
- `SocketListenerTransport` connection and framing;
- listener transport events;
- queue bounds, counters, shutdown, and typed errors.

### Rust FFI crate

Owns:

- Android-safe records/enums;
- an opaque, bounded listener transport handle;
- conversion between FFI records and core domain types;
- observer/callback or polling boundary;
- panic containment and stable error mapping;
- no domain policy duplication.

### Android platform/application layer

Owns:

- manual input/paste UI;
- Android lifecycle integration;
- coroutine dispatch off the main thread;
- observing FFI events and projecting them into app UI state;
- permissions only when actually required;
- device/log collection for physical acceptance.

### Desktop

Should require little or no production change unless the physical test exposes a host defect. Its manual endpoint, pending request, approval, rejection, removal, and end-session paths were already validated in Block 22/23 automation.

---

## 7. Files Claude Code should read first

### Planning and evidence

- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`
- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md`
- `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`
- `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`
- `docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md`
- `docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md`
- `memory.md`

### Shared Rust transport

- `rust/silent-disco-core/src/transport/mod.rs`
- `rust/silent-disco-core/src/transport/boundary.rs`
- `rust/silent-disco-core/src/transport/types.rs`
- `rust/silent-disco-core/src/transport/socket/host.rs`
- `rust/silent-disco-core/src/transport/socket/listener.rs`
- `rust/silent-disco-core/src/protocol/`

### FFI

- `rust/silent-disco-ffi/src/lib.rs`
- `rust/silent-disco-ffi/src/host_control/`
- `scripts/generate-uniffi-kotlin.sh`
- `scripts/build-rust-android.sh`

### Android listener

- `app/build.gradle.kts`
- `app/src/main/AndroidManifest.xml`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModelListenerActions.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/model/Models.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/TransportModels.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/WifiDirectTransportService.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/NearbySessionsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/SessionJoinScreen.kt`
- Android unit and instrumentation tests under `app/src/test` and `app/src/androidTest`.

### Desktop host

- `desktop/src-tauri/src/host_session_dto.rs`
- `desktop/src-tauri/src/platform/host_transport.rs`
- `desktop/src-tauri/src/platform/host_transport_events.rs`
- `desktop/src-tauri/src/platform/host_pending_handshake.rs`
- `desktop/src/screens/HostSessionScreen.tsx`
- `desktop/src/screens/ListenerDetailScreen.tsx`

---

## 8. Explicitly out of scope for Block 24

Do not expand Block 24 into later phases unless a prerequisite defect must be fixed:

- audio packetization completion;
- real audio streaming to Android;
- playback synchronization measurements;
- multi-listener scaling;
- mDNS publication;
- QR invitation changes beyond preserving existing behavior;
- local desktop monitor audio;
- lowering Android `minSdk`;
- redesigning the whole Android navigation stack;
- replacing the desktop transport.

Those belong to Blocks 25 and later.

---

## 9. Definition of done

Block 24 is complete only when all of the following are true:

- a physical API 29+ Android phone was used;
- Android can enter or paste the desktop’s real manual endpoint;
- Android connects through the shared Rust production listener transport;
- a real join request appears on desktop;
- approval is delivered and observed on Android;
- rejection is delivered and observed on Android in a separate run;
- listener disconnect is visible on desktop;
- desktop removal/disconnect is visible on Android;
- desktop end-session is visible on Android;
- invalid endpoint fails clearly and within bounded time;
- wrong protocol version fails clearly;
- every discovered code defect has an automated regression test;
- shared Rust, Android, desktop frontend, desktop Rust, source-size, generated-binding, and bundle gates pass as applicable;
- physical topology and results are documented;
- `memory.md` and the Block 24 checklist are updated accurately;
- no audio interoperability claim is made;
- no temporary workflows, logs, generated files, or test artifacts remain in the final commit.

Until those conditions are met, report Block 24 as in progress rather than complete.
