# Claude Code Handoff — 2026-08-11

**Repository:** `ekkus93/silent_disco`
**Branch:** `master` (clean, up to date with `origin/master`)
**HEAD at handoff time:** `3ad83834b87f6ac4f6f0dd8598e03348f21f2669`
**Primary desktop TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`
**Primary desktop spec:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md`
**Shared-core TODO:** `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`
**Shared-core spec:** `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`
**Full narrative log:** `memory.md` (read this first — every decision below has a fuller entry there)

---

## 1. Purpose of this handoff

This session ran a long sequence of parallel work pairs (desktop Tauri companion work alongside Android/shared-Rust-core work, dispatched as background agents in isolated git worktrees, one pair at a time). This document is a snapshot of where that left the project, so the next session doesn't have to reconstruct it from `git log` and agent transcripts.

Nothing is currently in flight. All worktrees have been cleaned up, all work described below is merged into `master` and pushed to `origin/master`.

---

## 2. What is complete as of this handoff

### 2.1 Desktop Lab Mode (Blocks 37–44), fully built and gated

The Tauri desktop companion now has a complete, feature-gated (`lab-mode` Cargo feature, **not** in `default`) developer testing subsystem under `desktop/src-tauri/src/lab/`:

- **Block 37** — build feature, isolated `LabRuntime`/`LabNodeHandle` runtime.
- **Block 38** — deterministic virtual clocks (`LabClock`/`LabNodeClock`), no wall-clock sleep.
- **Block 39** — virtual transport + fault injection, split across two layers: the shared core's `VirtualTransportFactory`/`FaultInjectingVirtualTransportFactory` (loss, duplication, reorder, corruption, bandwidth limit, disconnect, connection refusal — all synchronous) and the desktop-only `LabLatencyTransportFactory` (latency/jitter, needs virtual time). A hand-rolled `DeterministicPrng` (SplitMix64) was added to the shared core for this and is reused everywhere a seeded PRNG is needed.
- **Block 40** — scenario schema (JSON, versioned, fully bounded), a runner driving real `CoreActorHandle` commands, 10 typed assertion kinds.
- **Block 41** — scenario recording (on-disk format, version/seed-gated) and replay with real event-by-event divergence detection and a bounded diff — not just version-checking.
- **Block 42** — the full Tauri IPC command surface (`lab_commands.rs`/`lab_dto.rs`, 9 commands) plus the frontend `LabScreen.tsx` and Redux slice, since none of Blocks 37–41 had any frontend surface before this.
- **Block 43** — security/capability audit: capability surface confirmed minimal (`core:default` only), all 36 `#[tauri::command]` handlers read in full, two real gaps fixed (a scenario-file size check that read-before-checking-bounds, and a completely untested stale-revision rejection path in the authoritative actor), dependency review done for all direct Rust/npm deps (`cargo audit`: 0 vulnerabilities; `npm audit`: one real high-severity transitive dev dependency found and fixed).
- **Block 44** — silent-failure/fallback audit across `desktop/src-tauri` and `rust/silent-disco-core`. **Seven real production bugs found and fixed**, the sharpest being a poisoned notification mutex folding into the same `None` as "no failure" in `host_diagnostics()` — a false-healthy diagnostics signal. Full list with file/line is in `memory.md`'s Block 44 entry and the TODO's Block 44 section.

**Known, deliberately-scoped gap in Lab Mode:** `ScenarioLink`'s fault/latency fields are captured and bounded in the schema but not yet wired to any live transport — no Lab scenario today actually routes traffic through a faulted/latent link. This has been flagged consistently since Block 37 and is the natural next Lab Mode extension whenever it's prioritized (not currently blocking anything else).

### 2.2 Shared Rust core hardening (Blocks 24, 25)

- **Block 24 — FFI and concurrency hardening.** Found and fixed a real bug: `CoreActorRuntime`'s snapshot storage was a `std::sync::RwLock`, which has no reader/writer fairness guarantee — a concurrent read loop could starve the actor's write lock indefinitely (reproduced as a genuine hang, confirmed via thread-state inspection, fixed by switching to a plain `Mutex`). SAFETY-audited the workspace's 3 real `unsafe` blocks (all pre-documented, all in `silent-disco-ffi/src/audio_abi.rs`). Verified ASan/TSan are actually usable in this environment (contrary to an earlier assumption) and ran clean passes with them. Decided against `loom` with reasoning (the real bug was OS-scheduler-level, not something loom's model would catch). Confirmed no panic can cross the UniFFI or C ABI boundary; found a residual, undocumented gap at the JNI layer (audited clean today, no `catch_unwind` of its own — worth revisiting if that layer grows). Also found and documented (not fixed — genuinely out of scope, confirmed pre-existing via `git stash`) an intermittent flaky test in `listener_playback` under full-parallel release runs.
- **Block 25 — protocol/storage fuzz and property testing.** Decided against `cargo-fuzz` (no nightly toolchain available) and against adding `proptest` (kept the dependency footprint minimal, reused Block 39's `DeterministicPrng` instead). Built property round-trip tests, byte/mutation fuzzing of frame decoding, hostile-length rejection tests, and storage corruption/busy/read-only tests. **Found and fixed a real bug**: `DatabaseConnection::open` requested `SQLITE_OPEN_READ_WRITE`, but SQLite's Unix VFS silently falls back to read-only on a permission failure for an existing file, so `open()` was returning `Ok` for a database the process couldn't actually write to — fixed with a `verify_write_capable` check that fails fast instead of deferring to the first write. Left disk-full simulation and an injectable storage-IO boundary genuinely unimplemented (real environment/architecture constraints, documented rather than faked).

### 2.3 Android — `MainViewModel` reduction and sync-estimator migration (shared-core-migration Block 21, now fully closed)

- **Block 21 initial pass**: found `MainViewModel` was already further along than the TODO assumed (earlier blocks had already wired Rust snapshot mapping, lifecycle controllers, and Oboe-backed playback). Real work done: deleted genuinely dead Kotlin, fixed one remaining inline legality-reconstruction in `HostDashboardScreen.kt`. Left two gaps open, both since closed this session (see below): host audio packetization (still Kotlin-owned, correctly deferred — it's shared-core Block 23 scope, no Android-callable FFI entry point exists yet) and pre-runtime listener sync estimation.
- **Sync-estimator rewiring (follow-up, closed)**: `MainViewModelSynchronization.kt`'s pre-runtime path now uses `RustCoreBridge.openSyncEstimator()`/`RustSyncEstimator` instead of the local `ListenerSyncController`/`ClockSyncEstimator`, matching the already-Rust-authoritative post-runtime path. The old Kotlin sync classes were deleted. **This was verified on a real Android emulator** (confirmed `/dev/kvm` is usable in this sandbox, booted an x86_64 API 36 AVD, ran the existing `RustSyncEstimatorInstrumentedTest` for real — `OK (2 tests)`). The physical LG G6 was deliberately left untouched (see §3).
- **Effect-runner test coverage (follow-up, closed)**: built real fake-controller test infrastructure (`FakeHostCoreController`/`FakeListenerCoreController`/`FakeBleTransport`/`FakeSessionTransport`/`FakeRustDomainStore`), which required introducing real seams into production code (`RustDomainStore`/`BleTransport` interfaces, constructor injection points on `MainViewModel`) since none of this was fakeable before. Added 24 tests covering every distinct real failure path in `executeRustPlatformEffect`/`executeRustTransportEffect`/`executeRustStorageEffect`/`executeRustListenerPlatformEffect`. **Found a real bug, deliberately left unpatched**: `executeRustHostNotification`'s `controller.notifications.collect { ... }` loop has no exception handling — an unhandled exception on any platform-effect path silently and permanently kills host-effect processing for the rest of the session, with no diagnostic surfaced. This needs a real recovery/diagnostic design, not a reflexive `runCatching` — flagged as the most important open item from this session (see §4).

Block 21's acceptance criterion ("Kotlin/Compose is a native presentation and platform adapter shell; Rust is the only domain/data engine") is now much closer to fully met. The one remaining known exception is host audio packetization, which is explicitly Block 23 scope.

---

## 3. Environment constraints confirmed this session (don't re-derive these)

- **Android emulator**: genuinely usable in this sandbox (`/dev/kvm` present, boots in ~24s). Use it freely for anything that doesn't specifically need physical hardware.
- **Physical LG G6**: per standing project memory, may be attached but should be treated as temporarily unavailable — every agent this session was explicitly instructed to leave it untouched and use the emulator instead. Confirm current status with the user before assuming otherwise.
- **Nightly Rust toolchain**: installable and usable here (`rustup toolchain install nightly` works after one clean reinstall). ASan/TSan builds work via `-Z sanitizer=...` + `-Z build-std`. `cargo-fuzz` specifically was still ruled out (needs a persistent nightly *default*, conflicts with the pinned-stable `rust-toolchain.toml` workflow) — property-style tests using hand-rolled PRNGs are this project's established substitute.
- **Disk space**: was a recurring problem for concurrent agent builds (multiple `cargo`/`npm` build trees at once). Two mitigations already used successfully: point `CARGO_TARGET_DIR` at `/dev/shm` (tmpfs), or clean stale `target/` directories in the *non-worktree* main checkout (never another agent's worktree). Currently 84G+ free after cleanup; watch this if running another large parallel batch.
- **Agents getting stuck waiting on their own background/Monitor calls**: happened twice to one agent (Core Block 24) this session — it spawned some internal async/monitor wait that would never resolve for a subagent and returned a stub "waiting for notification" message with real uncommitted work sitting in its worktree. Had to be resumed twice with an explicit "run everything synchronously, do not wait on a monitor" instruction before it finished. Later agent prompts in this session had that instruction baked in up front and did not hit the problem again — keep including it in any future dispatched-agent prompts.

---

## 4. Open items for the next session

Roughly in the order they'd naturally come up:

1. **`executeRustHostNotification`'s unguarded `collect` loop** (Android, `MainViewModelRustHost.kt`) — an unhandled exception on any platform-effect path silently kills host-effect processing for the rest of the session with no diagnostic. This is a real violation of the project's "make failures visible" rule and was deliberately left unpatched pending a real recovery/diagnostic design (not a quick `runCatching`). Documented in `memory.md`'s effect-runner-coverage entry and the Block 21 TODO notes.
2. **`app/src/androidTest/.../P2UiTest.kt` fails to compile** — a pre-existing, unrelated break (confirmed via `git stash` to already exist on commits before this session's work), currently blocking `./gradlew connectedAndroidTest`/`compileDebugAndroidTestKotlin` as a whole. Two separate agents this session had to work around it (temporarily quarantining the file, always restored before commit) to get real instrumented-test runs through. Worth a dedicated fix so instrumented tests aren't blocked wholesale.
3. **Lab Mode's link-level fault/latency wiring to live transport** — schema-level support exists (Blocks 39–41), nothing routes real Lab traffic through it yet. Natural extension whenever prioritized.
4. **Desktop Block 45 — Performance and soak testing** (`SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`): test matrix, measurement, evidence-based limits. Not started.
5. **Desktop Blocks 46–48** — Linux packaging, final Android interoperability acceptance, final documentation/completion audit. Not started; 47 needs physical-device access.
6. **Shared-core Block 22 — Package Rust for Apple platforms**: confirmed **not executable in this Linux sandbox** (needs a real Xcode/macOS toolchain for XCFramework/simulator builds). Don't dispatch this here; needs a macOS environment.
7. **Shared-core Block 23 — long-term audio decoder boundary decision**: explicitly gated ("do not start until Rust packetization, scheduler, and Android Oboe output are stable"). This is also where the deferred host-audio-packetization Kotlin→Rust FFI entry point belongs. Worth revisiting readiness before starting.
8. **Shared-core Block 26 — Diagnostics and observability completion**: not started. Touches both Rust core and Kotlin/Swift export surfaces, so less cleanly parallelizable against pure-Android or pure-desktop work than recent blocks were — plan single-threaded or split carefully by file scope if parallelizing.
9. **Shared-core Blocks 27–29 — physical-device validation** (single-device, two-device, multi-listener): hardware-blocked, not parallelizable by adding more agents. Task list already tracks **A7 / Block 29 two physical listeners** as pending.
10. **Shared-core Blocks 30–31 — remove migration flags/obsolete code, final quality gate**: sequencing-dependent on the above; not started.

---

## 5. Working pattern this session (for continuity)

Two independent, file-disjoint tracks were run per round — almost always one desktop/Tauri track and one Android-or-shared-Rust-core track — dispatched as background `Agent` calls with `isolation: "worktree"`, each given:

- explicit instructions to read the relevant spec + TODO block + recent `memory.md` entries first;
- the real architectural context needed (existing APIs, prior decisions) so it doesn't rediscover the codebase from scratch;
- the standing quality gates to run for real (`bash scripts/check-rust.sh`, `cd desktop && npm run check`, `./gradlew test lintDebug`, as applicable);
- explicit fetch/rebase/retry-on-push-rejection instructions, since both agents in a pair append to the same `memory.md` and race to push;
- the "no `Co-Authored-By:` trailer" rule and the "run synchronously, don't wait on a background monitor" rule (added after the Block 24 stall).

Each pair's results were verified for real after completion (`git fetch`/`git log origin/master`, not just trusting the agent's self-report) before being relayed. Worktrees were cleaned up (`git worktree remove --force`) once merged. This pattern worked well and is worth continuing.
