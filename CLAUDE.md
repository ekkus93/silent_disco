# CLAUDE.md — `silent_disco`

## Project purpose

This repository currently contains an Android proof of concept for an offline silent disco app. It is now undergoing a staged migration toward a cross-platform architecture with:

- a shared Rust core for authoritative domain logic, protocol handling, synchronization, audio scheduling, diagnostics, and SQLite persistence;
- native Android presentation and platform adapters implemented with Kotlin and Jetpack Compose;
- a future native iOS presentation and platform-adapter shell implemented with Swift and SwiftUI;
- a Rust-owned real-time render ring consumed by platform-native audio callbacks.

The immediate goal remains validating whether:
- one Android phone can act as the **session host**
- the same host can act as the **audio source**
- the same host can act as the **authoritative monotonic clock**
- multiple nearby Android listener devices can join and hear the same playback in tight sync

This is a **viability-focused PoC and architectural migration**, not a production app.

## Authoritative migration documents

For shared-core work, read and follow:

- `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`
- `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`

Do not create additional assistant-generated design documents unless they are committed at the exact path referenced by the spec or TODO.

## Confirmed project decisions

- Use **Rust** for the shared authoritative domain/data core.
- Rust owns all domain SQLite access, migrations, and repositories.
- Rust owns jitter buffering, playback scheduling, and the bounded SPSC render ring.
- Use **UniFFI** for ordinary Kotlin/Swift control-plane bindings.
- Use a narrow **C ABI**, not UniFFI, from real-time platform audio callbacks.
- Use **Kotlin and Jetpack Compose** for Android presentation and Android platform adapters.
- Use **Swift and SwiftUI** for the future iOS presentation and Apple platform adapters.
- Use **Oboe** for Android production playback after the Rust ring-buffer migration reaches that block.
- **BLE is included**, but only for discovery/metadata assistance.
- Android Wi-Fi Direct may remain as an Android establishment adapter during migration; it must not remain the owner of protocol/domain state.
- Keep framing bounded, versioned, and explicitly validated.
- Success means listeners hear the same thing **about 99% of the time** in real-world use.

## Rust workspace commands

The Rust workspace is under `rust/` and is pinned by `rust/rust-toolchain.toml`.

Run all Rust quality gates with:

```bash
bash scripts/check-rust.sh
```

Equivalent direct commands:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Do not claim these commands passed unless they were actually executed with the pinned toolchain.

## Core implementation priorities

Prefer decisions that improve **real-device sync reliability** over polish or feature breadth.

Implementation priority order:
1. preserve current executable behavior with compatibility fixtures
2. establish the Rust workspace and bindings safely
3. move protocol, synchronization, persistence, and state ownership into Rust
4. move packetization, jitter buffering, and scheduling into Rust
5. move Android playback to Oboe consuming the Rust-owned ring
6. move protocol sockets into Rust behind platform discovery/establishment adapters
7. validate Android behavior on physical devices
8. add the Apple smoke target and later native iOS UI

## Architecture expectations

Maintain explicit boundaries around:
- presentation
- platform adapters
- authoritative Rust domain state
- transport/networking
- sync/timing
- audio pipeline
- storage
- diagnostics

Model host and listener behavior explicitly with strongly typed state machines.

Kotlin and Rust must not remain competing authoritative owners after a migration block transfers a responsibility. Kotlin/Swift may map Rust snapshots into localized presentation models, but must not reconstruct domain legality independently.

## Networking and sync rules

- The app must work **without internet access**.
- Topology is **strict star topology**: one host, multiple direct listeners, no relays, no mesh.
- **BLE must not be used** for primary audio transport.
- **BLE must not be used** as the authoritative playback timing transport.
- Use a **monotonic clock**, never wall-clock time, for sync and playback scheduling.
- The host defines the authoritative timeline.
- Listeners map host monotonic time onto local monotonic time.
- Use repeated **NTP-style four-timestamp exchange** for sync sampling.
- Prefer low-RTT sync samples and reject bad/outlier samples.
- Re-sync during playback is expected.
- Queues and frame sizes must be bounded.
- Zero recipients and partial delivery are not full success.

## Audio rules

- The host is the authoritative audio source.
- Listeners should not independently decode local copies of the same file for the PoC.
- Current network baseline:
  - PCM
  - 16-bit
  - stereo
  - 48 kHz
- Planned Rust render-ring format:
  - float32
  - interleaved
  - stereo
  - 48 kHz
- Initial packet duration target: **20 ms**.
- Initial startup buffer target: **roughly 400 ms**.
- Audio packets must include enough metadata to reconstruct stream order and playback time.
- Packets must be scheduled using **authoritative host presentation time mapped into local listener time**.
- Do **not** play packets immediately on arrival.
- Prefer simple correction strategies before advanced time-stretch/resampling.
- The real-time callback must not call UniFFI, JNI, SQLite, networking, logging, allocation-heavy code, or blocking synchronization.
- The callback must never outlive the Rust audio-engine token it consumes.

## Storage rules

- Rust is the sole owner of domain SQLite access.
- Kotlin and Swift provide an application-private database path but never issue domain SQL.
- One Rust database worker owns the SQLite connection.
- Migrations are ordered, immutable, checksummed, and transactional.
- Migration/integrity failure must not silently delete or recreate the database.
- No fallback to an in-memory database.
- High-frequency packet/frame telemetry must not be written directly to SQLite.
- Private keys remain in Android Keystore or iOS Keychain; SQLite may store key identifiers and public metadata.

## UI and UX guidance

This PoC UI is a **testing/control interface**, not a polished consumer music experience.

The UI should clearly communicate:
- whether the device is hosting or joining
- whether it is connected
- whether the listener is approved
- whether sync is healthy
- what is playing
- whether the session is healthy
- whether Rust core, storage, transport, or audio startup failed

Prioritize visibility of state and failures over visual polish.

## Diagnostics are mandatory

Diagnostics are a first-class requirement, not an optional debug extra.

Expose enough information to evaluate:
- core/binding/ABI/database schema versions
- actor and worker lifecycle
- queue depth and overflow
- sync offset
- RTT
- jitter
- drift
- packet loss
- late drops and concealment
- render-ring fill and full events
- callback underruns and contained panics
- database state and migration version
- listener health
- stream state

When implementing features, prefer designs that make diagnostics easier to capture and display.

## Error handling guidance

- Make failures visible in the UI.
- Provide a recovery path where practical.
- Do not silently auto-admit nearby devices.
- Manual host approval is the default.
- Do not hide transport, sync, playback, binding, queue, or database failures behind generic success states.
- Do not introduce broad `try/catch`, `runCatching`, Rust `unwrap`, or Rust `expect` that turns a production failure into log-only behavior.
- Do not claim an operation succeeded before the responsible subsystem reports completion.
- Do not silently fall back to fake/demo/in-memory implementations.

## Out of scope until the corresponding migration block

Unless the user explicitly changes scope, do not prematurely implement:
- a full SwiftUI consumer application before the Apple packaging/smoke blocks
- internet/cloud connectivity
- accounts/login
- streaming service integration
- playlists/social features
- mesh networking or relays
- host failover/election
- large-crowd scale optimization
- polished production-grade UX

## Guidance for Claude when making changes

- Read the migration specification, TODO, and `memory.md` before editing.
- Implement the smallest coherent unchecked block or explicitly allowed sub-block.
- Preserve the distinction between **presentation**, **platform adapters**, **authorization**, **transport**, **sync**, **playback**, and **storage**.
- Do not trade away timing visibility for abstraction neatness.
- Keep sync-sensitive and real-time paths explicit and instrumentable.
- Add production-facing tests; do not test copied local logic.
- Never fabricate build, test, CI, simulator, or physical-device results.
- Do not add `Co-Authored-By:` lines to commit messages; the repository rejects them.

## Ralph Loop workflow

For the Rust migration, use `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`:

1. Pick the next unchecked item or coherent sub-block explicitly allowed by that task.
2. Read the specification and referenced production files.
3. Implement the smallest complete change.
4. Write production-facing tests.
5. Run every applicable validation command.
6. Fix failures before proceeding.
7. Mark only work that is actually complete.
8. Commit and push the coherent block.
9. Record material decisions, failures, and device results in `memory.md`.

For unrelated legacy work, use the relevant active TODO rather than automatically returning to `docs/TODO.md`.

## Memory file

- You have access to a persistent memory file, `memory.md`, in the project root that stores context about the project, previous interactions, and preferences.
- Read `memory.md` at the start of each session to restore context from prior interactions.
- Before sending back a response, update `memory.md` with any new relevant information learned during the interaction. Timestamp and format entries clearly.
- Include the model name in the heading line so memory history records both time and model.
- **NEVER fabricate or guess timestamps.** Obtain the current time immediately before writing the entry. If the entry describes a specific commit, use the commit's actual timestamp.
- Format entries as:

```markdown
## 2026-06-06T12:00:00Z - Model name - Brief description
- Key fact or decision recorded.
- Another relevant detail.
```

- Quick command — **"Read memory.md"**: re-read the file because something from a prior session was forgotten.
