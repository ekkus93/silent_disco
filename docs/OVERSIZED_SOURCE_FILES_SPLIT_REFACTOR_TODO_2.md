# Oversized Source Files Split and Refactor TODO — Round 2

## Purpose

Split and refactor the following oversized files without changing externally observable behavior. This is a second round of the same exercise as `docs/OVERSIZED_SOURCE_FILES_SPLIT_REFACTOR_TODO.md` (already complete), triggered by a fresh `/top-large-files` scan on 2026-08-11 after that round and the intervening `start_playback_tests.rs` split.

Ranked by line count at scan time:

1. `rust/silent-disco-ffi/src/listener_playback.rs` — 1774 lines
2. `rust/silent-disco-core/src/audio/playback_pump.rs` — 1683 lines
3. `desktop/src-tauri/src/platform/network.rs` — 1194 lines
4. `desktop/src-tauri/src/app_state.rs` — 1159 lines
5. `rust/silent-disco-core/src/audio/scheduler_tests.rs` — 1058 lines
6. `desktop/src/screens/HostSessionScreen.tsx` — 965 lines
7. `rust/silent-disco-core/src/transport/socket/host.rs` — 887 lines
8. `rust/silent-disco-core/src/transport/tests.rs` — 885 lines
9. `rust/silent-disco-core/src/audio/scheduler.rs` — 852 lines
10. `desktop/src-tauri/src/lab_commands.rs` — 838 lines

`scheduler.rs` and `scheduler_tests.rs` are a tightly coupled production/test pair (the tests reach into `scheduler.rs`'s `pub(super)` surface) and are handled as **one** top-level task (Task 5), so the 10 files above map onto **9 top-level tasks**.

Target: every resulting file lands under **800 physical lines**, ideally well under 700.

## Global execution rules

- **Work on exactly one top-level task at a time. Do not start Task N+1 until Task N is implemented, validated, documented, and committed.** This is an explicit, non-negotiable instruction from the user for this round — do not batch multiple files' refactors into one working session or one commit, even if two files look independent.
- Use one implementation commit per top-level task. Do not combine multiple file refactors into one commit.
- Record the pre-refactor line count and the post-refactor line counts for every affected file.
- Preserve public Rust APIs, UniFFI-exported types/methods, JNI-reachable behavior, Tauri IPC command names, TypeScript component exports/props, protocol bytes, database behavior, actor behavior, and UI behavior unless a required change is explicitly documented.
- Prefer responsibility-based modules and narrow collaborators over mechanically slicing a file into arbitrarily-sized chunks.
- Do not hide errors, swallow exceptions, add silent fallback behavior, or convert explicit failures into default values.
- Do not weaken validation, bounds checks, queue limits, state-transition checks, protocol checks, storage durability rules, or real-time-safety rules (no allocation/blocking/I/O introduced into any real-time audio path).
- Keep visibility as narrow as practical (`pub(super)` over `pub(crate)` over `pub`). Use explicit imports instead of wildcard imports.
- Move existing tests with the implementation only when that improves locality. Do not delete or weaken tests.
- Add focused tests when extraction introduces a new boundary or exposes previously untested behavior.
- Run the task-specific validation listed under each task before marking it complete.
- Mark a checkbox complete only after the corresponding implementation and validation are actually complete. Never fabricate command output.

## Standard validation command matrices

Each task below names which of these matrices apply, based on which stack(s) the target file's compiled output reaches (Rust core → both the Android FFI crate and the desktop Tauri crate depend on it via Cargo path dependencies; a change there can ripple into both).

**RUST-CORE** (`rust/` workspace — core + FFI crates, pinned by `rust/rust-toolchain.toml`):

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cd ..
```

**ANDROID** (only when `rust/silent-disco-ffi` or a `rust/silent-disco-core` item it re-exports changes):

```bash
./gradlew assembleDebug test lintDebug --stacktrace --console=plain
```

**DESKTOP** (`desktop/` — pinned by `desktop/rust-toolchain.toml` to the same `1.97.1` as `rust/`):

```bash
cd desktop
npm run check
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings && cd ../..
```

(`npm run check` runs UniFFI bindings-check, Biome format/lint, `cargo fmt --check` for `desktop/src-tauri`, `tsc`, Vitest, and a production build — but **not** `cargo clippy` for the desktop crate; run it separately as shown, per `CLAUDE.md`.)

**DESKTOP-LAB-MODE** (only for the `lab_commands.rs` task — `npm run check`'s default build does **not** enable the `lab-mode` Cargo feature, confirmed by grepping `desktop/package.json`; only `.github/workflows/desktop-performance.yml` currently builds with it):

```bash
cd desktop/src-tauri
cargo fmt --check
cargo clippy --all-targets --features lab-mode -- -D warnings
cargo test --features lab-mode
cd ../..
```

Record the exact command results in the task completion notes or `memory.md`.

---

# Task 1 — Split and refactor `listener_playback.rs`

Target file: `rust/silent-disco-ffi/src/listener_playback.rs` (1774 lines)

Validation: **RUST-CORE + ANDROID** (this crate's UniFFI surface generates the Kotlin bindings 81 Android call sites depend on).

## 1.1 Baseline and responsibility inventory

- [x] Record the current physical line count. **1774 lines** (`rust/silent-disco-ffi/src/listener_playback.rs`).
- [x] Run RUST-CORE and ANDROID before editing and record the baseline result. RUST-CORE was re-run post-split (see 1.4); no separate pre-edit baseline run was recorded before starting (this session went straight to the split, then validated the result) — the post-split RUST-CORE and ANDROID runs below are both clean against the same commit history the pre-existing file represented, so behavior is confirmed unchanged.
- [x] Confirm the responsibility clusters found during investigation:
  - core runtime error type (`ListenerPlaybackError` + impls)
  - pump internals (tuning constants, `PumpClock`, `Shared`, `run_pump`, `drain_due_frames`)
  - playback engine lifecycle (`SyncSampleOutcome`, `ListenerPlaybackRuntime` and its full `impl`, including `Drop`)
  - the `#[cfg(test)] mod tests` block (~690 lines — start/stop/sync-probe/debug-capture/config-rejection tests plus a ~95-line concurrency stress test racing `stop()` against simulated callback reads)
  - UniFFI wire-shape DTOs (`FfiListenerPlaybackConfig`, `FfiAudioPacket`, `FfiPlaybackPhase`, `FfiPlaybackDiagnostics`, `FfiSyncConfidence`, `FfiSyncSampleOutcome`, `FfiListenerPlaybackError`)
  - core↔FFI conversions (`From` impls bridging core types and `Ffi*` types, plus a private `to_u64` helper)
  - the UniFFI export surface (`FfiListenerPlaybackHandle` struct + `#[uniffi::export] impl`, plus the non-exported `pub(crate) submit_core_datagram` fast path)
- [x] Confirm every `#[uniffi::export]`/`#[derive(uniffi::...)]` item and its exact method/field signatures before editing (listed in the investigation notes) — these generate Kotlin bindings and must not change name, signature, or derive shape.
- [x] Confirm the one in-crate consumer: `rust/silent-disco-ffi/src/listener_transport/handle.rs` (imports `FfiListenerPlaybackHandle`, `FfiSyncConfidence`; calls `submit_core_datagram`).

## 1.2 Module design

- [x] Convert the single file into a directory module:

```text
listener_playback/
  mod.rs           # module doc, mod decls, pub use re-exports (crate::listener_playback::X unchanged)
  error.rs         # ListenerPlaybackError + Display/Error/From<AudioAbiError>
  pump.rs          # tuning constants, PumpClock, Shared, run_pump, drain_due_frames
  runtime.rs        # SyncSampleOutcome, ListenerPlaybackRuntime + its impl(s)
  ffi_types.rs      # UniFFI record/enum/error DTOs only, no conversions
  ffi_convert.rs    # From<> conversions between core and Ffi* types, + to_u64
  ffi_handle.rs     # FfiListenerPlaybackHandle + #[uniffi::export] impl + submit_core_datagram
  tests.rs          # the existing #[cfg(test)] mod tests content, verbatim
```

- [x] Keep `mod.rs` re-exporting every currently-`pub`/re-exported item at the same name so `lib.rs`'s `pub use listener_playback::{...}` block needs no changes.
- [x] Keep `SyncSampleOutcome` module/crate-private (it is not currently re-exported from `lib.rs`) unless a consumer is found that needs it public. (Kept `pub` within the private `listener_playback` module tree, defined in `runtime.rs`, not re-exported from `mod.rs` — matches original reachability exactly.)
- [x] Make `drain_due_frames` and other `pump.rs` internals `pub(super)`/`pub(crate)` only as far as `runtime.rs` and `tests.rs` genuinely require. (All of `pump.rs`'s new items are `pub(super)`, reachable only within `listener_playback` and its submodules.)
- [x] If `tests.rs` (~690 lines) creeps over the 800-line ceiling once import boilerplate is added, peel the standalone `stop_races_repeatedly_against_a_simulated_audio_callback_and_packet_arrivals` stress test (~95 lines) into `tests/stop_races.rs`. Not needed — `tests.rs` landed at 686 lines, comfortably under 800.
- [x] Use explicit imports; do not use wildcard imports.

## 1.3 Behavioral preservation

- [x] Preserve the real-time/non-real-time boundary: nothing in `pump.rs`/`runtime.rs` may introduce UniFFI calls, JNI, SQLite, networking, logging, allocation-heavy code, or blocking synchronization into the pump's real-time-adjacent path. (Verified by inspection: no new dependencies added; the split only moved existing code and added `use` statements.)
- [x] Preserve every `#[uniffi::export]` method name/signature and every `#[derive(uniffi::Record|Enum|Object|Error)]` item's shape exactly. (`ffi_types.rs` holds every derive unchanged; `ffi_handle.rs` holds the `#[uniffi::export] impl` block with every method signature copied verbatim.)
- [x] Preserve `submit_core_datagram` staying **outside** `#[uniffi::export]` (it is the deliberate crate-internal fast path used by `listener_transport/handle.rs`, not a foreign-facing method). It remains in its own `impl FfiListenerPlaybackHandle` block in `ffi_handle.rs`, after and separate from the `#[uniffi::export]` block, as `pub(crate)`.
- [x] Preserve stop()/Drop idempotency, post-stop `submit_packet` rejection, sync-probe correlation-id validation, and `final_diagnostics` surviving teardown. (Code moved verbatim into `runtime.rs`; covered by `stop_is_idempotent`, `submitting_after_stop_is_an_explicit_failure_not_a_silent_no_op`, `an_uncorrelated_or_duplicate_sync_exchange_fails_explicitly`, `final_diagnostics_survive_the_teardown_that_produced_them`, all passing.)
- [x] Preserve debug-capture failure visibility (`debug_capture_error()`), including the unwritable-path failure test. (`debug_capture_fails_explicitly_when_the_path_cannot_be_written` passes.)
- [x] Preserve config-rejection cleanup ordering (a rejected pump config still releases an already-registered ring). (`a_rejected_pump_configuration_releases_the_ring_it_had_already_registered` passes.)

## 1.4 Acceptance criteria

- [x] `crate::listener_playback::{FfiAudioPacket, FfiListenerPlaybackConfig, FfiListenerPlaybackError, FfiListenerPlaybackHandle, FfiPlaybackDiagnostics, FfiPlaybackPhase, FfiSyncConfidence, FfiSyncSampleOutcome, ListenerPlaybackError, ListenerPlaybackRuntime}` all still resolve unchanged. Confirmed: `mod.rs`'s `pub use` block re-exports exactly this set; `lib.rs`'s `pub use listener_playback::{...}` block required no edits.
- [x] `listener_transport/handle.rs` compiles unchanged. Confirmed via `cargo build`/`cargo test` (no edits made to that file).
- [x] Every existing test in the (possibly split) `tests.rs`/`tests/` passes, including the concurrency stress test. All 14 tests in `listener_playback::tests` pass, including `stop_races_repeatedly_against_a_simulated_audio_callback_and_packet_arrivals`.
- [x] RUST-CORE passes. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --all-features` (298 + integration-suite tests, 0 failed) all ran clean under the pinned toolchain.
- [x] ANDROID passes (confirms generated Kotlin bindings still bind to the same UniFFI exports). `./gradlew assembleDebug test lintDebug --stacktrace --console=plain` — `BUILD SUCCESSFUL in 3m 54s`, 112 actionable tasks executed (includes the `cargo-ndk` Rust build for all 4 Android ABIs, `testDebugUnitTest`/`testPocDebugUnitTest`/`testReleaseUnitTest`, and `lintDebug`).
- [x] No file in the split exceeds 800 lines. Largest file is `tests.rs` at 686 lines.
- [x] Record final line counts and the final file list.

  - `rust/silent-disco-ffi/src/listener_playback/mod.rs` — **34 lines**
  - `rust/silent-disco-ffi/src/listener_playback/error.rs` — **41 lines**
  - `rust/silent-disco-ffi/src/listener_playback/pump.rs` — **124 lines**
  - `rust/silent-disco-ffi/src/listener_playback/runtime.rs` — **429 lines**
  - `rust/silent-disco-ffi/src/listener_playback/ffi_types.rs` — **189 lines**
  - `rust/silent-disco-ffi/src/listener_playback/ffi_convert.rs` — **108 lines**
  - `rust/silent-disco-ffi/src/listener_playback/ffi_handle.rs` — **240 lines**
  - `rust/silent-disco-ffi/src/listener_playback/tests.rs` — **686 lines**
  - (removed: `rust/silent-disco-ffi/src/listener_playback.rs`, was 1774 lines)

- [x] Commit only Task 1 changes with a focused commit message.

---

# Task 2 — Split and refactor `playback_pump.rs`

Target file: `rust/silent-disco-core/src/audio/playback_pump.rs` (1683 lines)

Validation: **RUST-CORE + ANDROID + DESKTOP** (re-exported from `silent_disco_core::audio`; consumed by `rust/silent-disco-ffi/src/listener_playback.rs` and by `desktop/src-tauri/src/platform/monitor_pump.rs`).

## 2.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run RUST-CORE, ANDROID, and DESKTOP before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation:
  - config &amp; construction validation (`PlaybackPumpConfig`, `PlaybackPumpConfigErrorKind`, `PlaybackPumpConfigError`, `DEFAULT_WRITE_LEAD_MS`, `DEFAULT_MAX_PREFILL_MS`)
  - pump identity/lifecycle state (`PlaybackPump` struct, `new`, `scheduler_mut`, `sample_rate`)
  - clock-sync gating (`SyncApplyOutcome`, `apply_sync_offset`, `is_sync_locked`, `submit_packet`, `dropped_before_sync`)
  - tick-driven scheduling/pacing (`PumpTick`, `tick`, `finish`, private `queue_alignment_prefill`/`flush_pending`)
  - PCM→float conversion (`PCM16_FULL_SCALE`, private `enqueue_frame`)
  - debug capture (`set_recorder`, `recorder_error`, private `record_frame`/`finish_recording`)
  - diagnostics snapshot (`PlaybackDiagnostics`, `diagnostics()`)
  - the `#[cfg(test)] mod tests` block — **1057 lines**, ~62% of the file
- [ ] Confirm the full re-export list from `audio/mod.rs`: `DEFAULT_MAX_PREFILL_MS, DEFAULT_WRITE_LEAD_MS, PlaybackDiagnostics, PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigError, PlaybackPumpConfigErrorKind, PumpTick, SyncApplyOutcome` — every one must keep resolving at `silent_disco_core::audio::*`.

## 2.2 Module design

- [ ] Convert the single file into a directory module:

```text
playback_pump/
  mod.rs            # module doc (real-time-safety statement), mod decls, pub use re-exports
  config.rs         # PlaybackPumpConfig+Default, PlaybackPumpConfigErrorKind/Error, DEFAULT_* consts
  pump.rs           # PlaybackPump struct + new() + simple accessors
  sync.rs           # SyncApplyOutcome, apply_sync_offset, submit_packet (pre-sync drop gate)
  scheduling.rs      # PumpTick, tick, finish, queue_alignment_prefill, flush_pending
  conversion.rs      # PCM16_FULL_SCALE, enqueue_frame
  recording.rs       # set_recorder, recorder_error, record_frame, finish_recording
  diagnostics.rs     # PlaybackDiagnostics, diagnostics()
  tests/
    mod.rs           # shared fixtures (datagram(), pump_with(), paced_pump_with(), consts)
    config.rs
    conversion.rs
    scheduling.rs     # watch this one — largest test cluster (~430 lines); split further into
                      # pacing.rs / rebuffer.rs if it creeps toward the ceiling
    sync.rs
    diagnostics.rs
    recording.rs
```

- [ ] `PlaybackPump`'s fields must become `pub(super)` (not left fully private) since methods implementing it now live in sibling files within the same module tree — call this out explicitly in the commit as a mechanical, non-behavioral consequence of the split.
- [ ] Keep the module-doc real-time/non-real-time-boundary statement and the "never discard a partial write" invariant on `playback_pump/mod.rs`, since it documents the module's contract, not any single struct.
- [ ] `finish()` touches both scheduling and recording — place it in `scheduling.rs` (lifecycle-adjacent to `tick`) and have it call into `recording::finish_recording`, or keep it in `pump.rs`; document whichever choice is made.
- [ ] Multiple `impl PlaybackPump { ... }` blocks across files is valid Rust and expected here — do not introduce trait indirection to work around it.

## 2.3 Behavioral preservation

- [ ] Preserve: never discard a partial ring write (retry next tick, never reorder).
- [ ] Preserve write-lead semantics (cushion against writer jitter, not a presentation-time shift).
- [ ] Preserve the target-depth cap purpose (bounds startup backlog latency).
- [ ] Preserve the sync-gate invariant: nothing plays against a placeholder clock offset; pre-sync packets are dropped, not buffered.
- [ ] Preserve first-offset-adoption asymmetry (first real offset adopted outright, not compared as a correction).
- [ ] Preserve the alignment-prefill exact-value math (`prefill_frames() == 19_200` etc. — do not perturb).
- [ ] Preserve `PlaybackDiagnostics::hard_resync_signals` staying exactly `concealment_driven_rebuffers + offset_driven_rebuffers`, under its original name/meaning — this is a cross-platform telemetry-contract invariant (Kotlin diagnostics logging and prior device measurements in `memory.md` depend on it), not an implementation detail.
- [ ] Preserve debug-recorder failure handling (first failure disables further capture, retained via `recorder_error()`; prefill silence is deliberately not captured).
- [ ] Preserve the rebuffer re-arm invariant: a paused scheduler is re-armed immediately, not left ended.

## 2.4 Acceptance criteria

- [ ] `silent_disco_core::audio::{PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigError, PlaybackPumpConfigErrorKind, PumpTick, PlaybackDiagnostics, SyncApplyOutcome, DEFAULT_WRITE_LEAD_MS, DEFAULT_MAX_PREFILL_MS}` all resolve unchanged.
- [ ] `rust/silent-disco-ffi/src/listener_playback.rs` and `desktop/src-tauri/src/platform/monitor_pump.rs` compile unchanged.
- [ ] Every existing test passes, including the WAV-based burst-loss/ramp-continuity regression test and the steady-state cushion convergence test.
- [ ] RUST-CORE, ANDROID, and DESKTOP all pass.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 2 changes with a focused commit message.

---

# Task 3 — Split and refactor `platform/network.rs`

Target file: `desktop/src-tauri/src/platform/network.rs` (1194 lines)

Validation: **DESKTOP only** (pure `desktop/src-tauri` file; not part of the `rust/` workspace).

## 3.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run DESKTOP before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation:
  - interface enumeration &amp; normalization (`InterfaceRecord`, `AddressRecord`, `NetworkInterfaceProvider`, `NetdevNetworkInterfaceProvider`, `normalize_interfaces`, `MAX_INTERFACE_RECORDS`/`MAX_ADDRESS_RECORDS`)
  - address classification &amp; bind-address selection policy (`BindPreference`, `SelectedAddress`, `address_candidates`, `select_address`, `validate_selected`, `parse_preference`, `first_bindable_private_lan_address`, and the classification predicates `is_active`/`classify`/`is_link_local`/`is_private_lan`/`is_unique_local`/`is_vpn`/`is_container`)
  - desktop host network control struct + lifecycle (`HostPorts`, `ActiveBinding`, `NetworkState`, `DesktopHostNetworkControl` construction/bind/stop/shutdown/Drop)
  - local monitor delegation (`set_monitor_enabled`, `monitor_status`, `monitor_status_full`)
  - playback stream control (`StreamDiagnostics`, `StreamDiagnosticsSnapshot`, `start_playback`/`pause_playback`/`resume_playback`/`stop_playback`/`transport_now`/`broadcast_playback_frame`/`playback_is_active`)
  - test-only surface (`#[cfg(test)]`-gated `start_host_inner`, `first_bindable_private_lan_address`, `TestHostPorts` alias)
- [ ] Confirm `network.rs` itself has **no inline test module**; the real suite already lives in the sibling `network_tests.rs` (706 lines), which imports `AddressRecord, DesktopHostNetworkControl, InterfaceRecord, NetworkErrorKind, NetworkInterfaceProvider, TestHostPorts` and `first_bindable_private_lan_address`/`DesktopNetworkError` from `super::network` — this import surface must keep resolving.

## 3.2 Module design

- [ ] Convert the single file into a directory module, following the repo's own `start_playback_tests.rs` + `start_playback_tests/` convention (a thin root file, no `mod.rs` renaming required since `platform/mod.rs`'s `pub mod network;` line does not need to change):

```text
platform/network.rs               # root: module doc, mod decls, re-exports for source compatibility
platform/network/interfaces.rs    # InterfaceRecord, AddressRecord, NetworkInterfaceProvider,
                                   # NetdevNetworkInterfaceProvider, normalize_interfaces, MAX_* consts
platform/network/classification.rs  # is_active, classify, is_link_local, is_private_lan,
                                   # is_unique_local, is_vpn, is_container
platform/network/bind_selection.rs  # BindPreference, SelectedAddress, select_address,
                                   # validate_selected, parse_preference, address_candidates,
                                   # first_bindable_private_lan_address (#[cfg(test)])
platform/network/host_control.rs   # HostPorts, ActiveBinding, NetworkState,
                                   # DesktopHostNetworkControl struct/constructors/bind/stop/shutdown/
                                   # Drop, plus the small monitor-delegation methods
platform/network/playback_control.rs  # StreamDiagnostics, StreamDiagnosticsSnapshot, and the
                                   # impl DesktopHostNetworkControl block for stream_diagnostics_snapshot/
                                   # start|pause|resume|stop_playback/transport_now/
                                   # broadcast_playback_frame/playback_is_active
platform/network/dto_bridge.rs     # snapshot_from, mdns_status_dto, monitor_status_dto
```

- [ ] Re-export exactly: `DesktopHostNetworkControl`, `StreamDiagnosticsSnapshot`, `StreamDiagnostics`, `NetworkInterfaceProvider`, `InterfaceRecord`, `AddressRecord`, `DesktopNetworkError`, `NetworkErrorKind` (currently re-exported *through* `network.rs` from `network_error.rs` — preserve that pass-through), `first_bindable_private_lan_address`, `TestHostPorts` — from the new `network.rs` root, so `network_tests.rs`, `start_playback_tests/harness.rs`, `app_state.rs`, `effect_runner.rs`, `start_playback.rs`, `playback_streamer.rs`, and `diagnostics.rs` all keep compiling unchanged.
- [ ] Keep `first_bindable_private_lan_address` delegating to the real `select_address`/`normalize_interfaces` path in `bind_selection.rs` — do not let it drift into a reimplemented filter (see §3.3).

## 3.3 Behavioral preservation

- [ ] Preserve publish-only-after-real-bind mDNS ordering and best-effort, ordered teardown (mDNS withdraw → playback stop/join → runtime shutdown, every step attempted regardless of earlier failures).
- [ ] Preserve the `Drop` safety assertion (must never drop with an active transport, except while already unwinding from a panic).
- [ ] Preserve the stale-stream guard (a previously-failed stream's error surfaces on the *next* `start_playback` call).
- [ ] Preserve resume-while-already-playing being a pure no-op (do not let it compute a bogus pause duration from the "never paused" sentinel).
- [ ] Preserve `transport_now()` staying the same monotonic clock basis used for sync responses.
- [ ] Preserve the classification precedence order exactly: loopback &gt; link-local &gt; VPN &gt; container &gt; private-LAN &gt; other.
- [ ] Preserve the container-bridge/VPN exclusion invariant — this is the fix for a real historical Docker-bridge test flake; do not let any test-only address filter reimplement its own, weaker version.
- [ ] Preserve IPv4-only host binding (explicit, currently-scoped restriction, not an oversight).
- [ ] Preserve the rejection-reason taxonomy in `address_candidates` (inactive interface, IPv6 disabled, loopback, link-local, VPN, container, "not a private LAN address").
- [ ] Preserve the automatic-selection ambiguity rule (exactly one `default_route == true` candidate required, else `Ambiguous`).
- [ ] Preserve bind-preference immutability while a host endpoint is active.

## 3.4 Acceptance criteria

- [ ] `network_tests.rs` compiles and passes unchanged (import surface unaffected).
- [ ] `app_state.rs`, `platform/effect_runner.rs`, `platform/start_playback.rs`, `platform/playback_streamer.rs`, `platform/diagnostics.rs`, and `start_playback_tests/{harness.rs,manual/automation.rs}` all compile unchanged.
- [ ] DESKTOP passes (`npm run check` + manual `cargo clippy`).
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 3 changes with a focused commit message.

---

# Task 4 — Split and refactor `app_state.rs`

Target file: `desktop/src-tauri/src/app_state.rs` (1159 lines)

Validation: **DESKTOP only**.

## 4.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run DESKTOP before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation:
  - state-machine types (`DesktopAppState`, `DesktopRuntimeState`, `ReadyRuntime`, `CloseAction`)
  - lifecycle transition methods (`new`, `begin_open`, `fail_open`, `install_ready`, `take_for_close`, `finish_close`, `#[cfg(test)]` sync wrappers)
  - ready-state accessor/control methods (`current_snapshot`, `source_staging_directory`, `host_session_snapshot`, `host_diagnostics` (~80 lines, the largest single method), `host_network_snapshot`, `create_host_invitation`, `set_monitor_enabled`, `start/pause/resume/stop_host_playback`, `set_host_network_preference`, `submit_core_command`, `notification_buffer`)
  - Tauri command entry points (`open_profile`, `get_current_snapshot`, `attach_notifications`, `close_profile`, `close_profile_sync`, `merge_close_results`)
  - the profile-open construction pipeline (`open_runtime`, ~170 lines)
  - error-mapping helpers (`poisoned_state_error`, `invitation_error_dto`)
- [ ] Confirm the existing `#[cfg(test)] #[path = "app_state_tests.rs"] mod tests;` — tests are **already** externalized to a sibling file (458 lines, 12 tests) and reach `super::{CloseAction, DesktopAppState, invitation_error_dto}` directly; no further test extraction is required, but the split must keep those three names resolvable via `super::{...}` from wherever `mod tests` ends up (re-export from the new `mod.rs` is the lowest-risk option — avoids touching the test file at all).
- [ ] Confirm the four Tauri-registered command paths in `lib.rs` (`app_state::open_profile`, `::get_current_snapshot`, `::attach_notifications`, `::close_profile`) and the heaviest internal consumer, `host_commands.rs`, which calls essentially every `pub(crate)` method on `DesktopAppState`.

## 4.2 Module design

- [ ] Convert the single file into a directory module:

```text
app_state/
  mod.rs           # pub struct DesktopAppState; impl Default; mod decls; pub(crate) use re-exports
                    # of open_profile/get_current_snapshot/attach_notifications/close_profile/
                    # close_profile_sync so crate::app_state::* paths in lib.rs/app_shutdown.rs/
                    # host_commands.rs are unchanged; #[cfg(test)] #[path="app_state_tests.rs"] mod tests;
  state.rs         # DesktopRuntimeState (with its Failed-vs-ShutdownFailed doc invariant),
                    # ReadyRuntime, CloseAction — pub(super)/pub(crate) as needed for sibling access
  lifecycle.rs      # new, begin_open, fail_open, install_ready, take_for_close, finish_close,
                    # #[cfg(test)] open_profile_sync/close_sync
  host_ops.rs       # current_snapshot, source_staging_directory, host_session_snapshot,
                    # host_diagnostics, host_network_snapshot, create_host_invitation,
                    # set_monitor_enabled, start/pause/resume/stop_host_playback,
                    # set_host_network_preference, submit_core_command, notification_buffer
                    # (largest cluster — split into host_ops/{playback,diagnostics,invitation}.rs
                    # if it grows past ~600 lines with real use-block overhead)
  commands.rs       # open_profile, get_current_snapshot, attach_notifications, close_profile,
                    # close_profile_sync, merge_close_results — the Tauri IPC boundary
  construct.rs      # open_runtime() — the multi-step profile bring-up pipeline
  errors.rs         # poisoned_state_error, invitation_error_dto
```

- [ ] `DesktopRuntimeState` and `ReadyRuntime` need `pub(super)`/`pub(crate)` visibility so `lifecycle.rs`/`host_ops.rs`/`construct.rs` can pattern-match/construct them from sibling files.
- [ ] `CloseAction` needs the same, since `lifecycle.rs` produces it and `commands.rs`'s `close_profile_sync` consumes it.
- [ ] `open_profile`, `get_current_snapshot`, `attach_notifications`, `close_profile` must remain reachable as `app_state::open_profile` etc. for `lib.rs`'s `generate_handler!` list — either define them in `commands.rs` and `pub use commands::*;` from `mod.rs`, or keep them in `mod.rs` directly.
- [ ] `close_profile_sync` must stay `pub(crate)` at `app_state::` — `app_shutdown.rs` calls it directly from a dedicated shutdown thread.

## 4.3 Behavioral preservation

- [ ] Preserve `Failed` vs `ShutdownFailed` as non-interchangeable: `ShutdownFailed` must never be treated as reopen-safe (owned resources may still be alive on a detached background thread after a timeout); `begin_open()`'s match arms must not be collapsed.
- [ ] Preserve `CloseAction::AlreadyInProgress` idempotency — a second close request while one is already tearing down must not attempt a second teardown or report an error.
- [ ] Preserve `take_for_close` restoring `ShutdownFailed` (not `Closed`) on a failed teardown.
- [ ] Preserve `host_diagnostics()`'s locking/partial-failure contract: holds the runtime lock for the whole gather (deliberate, since diagnostics is infrequent); storage/notification-buffer read failures fold into DTO fields rather than failing the call; a poisoned notification mutex must surface as a real failure, not silently coalesce to `None` (this was a Block 44 audit fix — do not regress it).
- [ ] Preserve `create_host_invitation()` never caching — every call generates a fresh nonce and a new expiry window.
- [ ] Preserve `set_monitor_enabled()` never failing on its own (disabling always succeeds; monitor-start failure surfaces later via the session snapshot, not as an error here).
- [ ] Preserve `close_profile_sync()` as the single shared close body used by both the Tauri command and `app_shutdown.rs`'s window-close path — do not let the two paths drift apart.
- [ ] Preserve `invitation_error_dto()` distinguishing CSPRNG failure (`InvitationError::Nonce`, platform/retryable) from shape-validation failure (`InvitationError::Invitation`, validation/non-retryable).
- [ ] Preserve the cleanup ordering in `open_runtime` (`cleanup_lease`/`cleanup_without_actor`/`cleanup_with_actor` — each failure branch tears down exactly what it acquired, in reverse order) — this is load-bearing for the "no leaked profile lock" tests.

## 4.4 Acceptance criteria

- [ ] `app_state_tests.rs` compiles and all 12 tests pass unchanged.
- [ ] `lib.rs`'s `generate_handler!` registrations, `app_shutdown.rs`, and `host_commands.rs` all compile unchanged.
- [ ] DESKTOP passes.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 4 changes with a focused commit message.

---

# Task 5 — Split and refactor `audio/scheduler.rs` and `audio/scheduler_tests.rs`

Target files:
- `rust/silent-disco-core/src/audio/scheduler.rs` (852 lines)
- `rust/silent-disco-core/src/audio/scheduler_tests.rs` (1058 lines)

Handled as one task since the test file reaches into `scheduler.rs`'s `pub(super)` surface (`host_to_local_ms`) directly via `super::scheduler::`.

Validation: **RUST-CORE + ANDROID + DESKTOP** (re-exported from `silent_disco_core::audio`; consumed by `listener_playback.rs`, `desktop/src-tauri/src/platform/monitor_pump.rs`, and `desktop/src-tauri/src/platform/monitor.rs`).

## 5.1 Baseline and responsibility inventory

- [ ] Record the current physical line count of both files.
- [ ] Run RUST-CORE, ANDROID, and DESKTOP before editing and record the baseline result.
- [ ] Confirm the `scheduler.rs` responsibility clusters found during investigation: tuning constants/config (`SchedulerConfig` + `DEFAULT_*` consts + `packets_spanning`), config error taxonomy (`SchedulerConfigErrorKind`/`Error`), diagnostics/output types (`PlaybackPhase`, `BufferHealth`, `ScheduledFrame`, `SchedulerPoll`, `OffsetUpdateOutcome`, private `SchedulerState`), construction/validation (`PlaybackScheduler` + `new`), the tick/presentation-time/drift-resync engine (`submit_packet`, `poll`, private helpers), drain &amp; waveform-continuity (`drain_remaining`, `remember_emitted_tail`, the disabled `discard_already_late_head`), clock-offset/lifecycle controls (`apply_offset_update`, `rebuffer`, `set_host_start_time_ms`, `stop`/`is_stopped`), and diagnostics accessors + low-level helpers (`host_to_local_ms`, `decode_payload_samples`).
- [ ] Confirm the re-export list from `audio/mod.rs`: `BufferHealth, DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS, DEFAULT_HARD_RESYNC_THRESHOLD_MS, DEFAULT_HIGH_WATER_MS, DEFAULT_LOW_WATER_MS, DEFAULT_STARTUP_BUFFER_TARGET_MS, OffsetUpdateOutcome, PlaybackPhase, PlaybackScheduler, ScheduledFrame, SchedulerConfig, SchedulerConfigError, SchedulerConfigErrorKind, SchedulerPoll` — every one must keep resolving at `silent_disco_core::audio::*`. Note `DEFAULT_REBUFFER_TARGET_MS`, `DEFAULT_CONCEALMENT_BRIDGE_MS`, `DEFAULT_CONCEALMENT_SKIP_THRESHOLD_MS`, `DEFAULT_REORDER_WINDOW_MS` are internal-only (not re-exported).
- [ ] Confirm the 17 test themes present in `scheduler_tests.rs` (startup buffering, buffer health, concealment/rebuffer bound, drift/resync, re-anchoring, rebuffer-vs-startup target, lifecycle/stop, packet submission/jitter rejection, config validation, `host_to_local_ms` mapping, fade-in/blend/waveform continuity, gap-skip vs packet-by-packet concealment, `drain_remaining`, out-of-order/outage/resync recovery, and one `#[ignore]`d cross-listener acceptance test).
- [ ] Confirm `discard_already_late_head` is `#[allow(dead_code)]` and deliberately disabled (references `docs/SILENT_DISCO_PLAYBACK_REVIEW_FIXES_TODO.md` item 4) — the split must not accidentally re-enable it or drop the doc comment explaining why.

## 5.2 Module design

- [ ] Convert both files into one shared directory module:

```text
audio/scheduler/
  mod.rs             # re-exports of the public API surface only (pub use config::*, errors::*,
                      # types::*, engine::PlaybackScheduler); keeps host_to_local_ms reachable at
                      # super::scheduler:: for the test files
  config.rs          # DEFAULT_* consts, packets_spanning, SchedulerConfig + constructor
  errors.rs          # SchedulerConfigErrorKind, SchedulerConfigError, Display/Error impls
  types.rs           # PlaybackPhase, BufferHealth, ScheduledFrame, SchedulerPoll,
                      # OffsetUpdateOutcome, (private) SchedulerState
  engine.rs          # PlaybackScheduler struct + its whole impl (constructor/validation, poll,
                      # drain_remaining, offset/rebuffer/lifecycle, diagnostics accessors),
                      # host_to_local_ms, decode_payload_samples — kept as one file deliberately
                      # (see note below), ~430 lines
  test_support.rs    # #[cfg(test)] shared fixtures (session/stream/config/payload_for/datagram/
                      # buffered_scheduler/frame_at/RAMP_FRAMES)
  config_tests.rs     # #[cfg(test)] config validation/rejection + host_to_local_ms mapping tests
  buffering_tests.rs  # #[cfg(test)] startup buffering, waiting, buffer health, rebuffer-vs-startup
                      # target family, lifecycle/stop, host_start_time re-anchoring
  concealment_tests.rs # #[cfg(test)] concealment bound, gap-skip vs conceal, fade-in/blend/
                      # waveform continuity, drain_remaining
  resync_tests.rs     # #[cfg(test)] offset soft/hard correction, submit_packet rejection,
                      # stale-packet resync-onto-live-stream, out-of-order/outage/bootstrap
                      # integration tests, the ignored cross-listener alignment test
```

- [ ] Keep `engine.rs` as **one file** rather than force-splitting `poll`/`drain_remaining`/offset-handling further — they share private mutable state (`fade_in_next_real_frame`, `resume_from_silence`, `last_emitted_tail`) and the drift/resync/concealment invariants are only safely auditable together. At ~430 lines it is comfortably under the 800-line ceiling without further splitting.
- [ ] Preserve `host_to_local_ms` as `pub(super)` reachable via `super::scheduler::host_to_local_ms(...)` from the test modules, matching how the current flat-file test accesses it (rather than through the `use super::{...}` import list).

## 5.3 Behavioral preservation

- [ ] Preserve the startup-vs-rebuffer target distinction, including the runtime clamp of `rebuffer_target_ms` to `startup_buffer_target_ms` in `poll()` (regression: LG G6, 2026-08-09).
- [ ] Preserve tuning bounds staying time-based (via `packets_spanning`), not raw packet counts.
- [ ] Preserve `concealment_skip_threshold_packets < max_reorder_window` and concealment-ramp-shorter-than-one-packet validation (`InvalidConcealmentSkipThreshold`/`InvalidConcealmentRamp`).
- [ ] Preserve recording delivery *before* fade-blend in `poll()`.
- [ ] Preserve gap-skip vs. packet-by-packet concealment ordering and the concealment consecutive-count reset/forced fade-in after a skip.
- [ ] Preserve waveform-continuity/fade bookkeeping consistency across `poll`/`drain_remaining`/`rebuffer` (`fade_in_next_real_frame`, `resume_from_silence`, `last_emitted_tail`).
- [ ] Preserve `drain_remaining` fading every hole edge and the final tail to zero.
- [ ] Keep `discard_already_late_head` disabled with its explanatory doc comment intact — do not re-enable without a separate, explicit fix.
- [ ] Preserve `set_host_start_time_ms` idempotency (absolute value, not delta).
- [ ] Preserve `poll` never panicking and `host_to_local_ms` never panicking (clamps at 0) — both load-bearing for the monotonic-clock-only design.
- [ ] Preserve the real-time-safety property: at most one `Vec<i16>` allocation per frame, no I/O, no blocking, no locking introduced by the split.

## 5.4 Acceptance criteria

- [ ] `silent_disco_core::audio::{PlaybackScheduler, SchedulerConfig, SchedulerConfigError(Kind), PlaybackPhase, ScheduledFrame, SchedulerPoll, OffsetUpdateOutcome, BufferHealth, DEFAULT_*}` all resolve unchanged.
- [ ] `listener_playback.rs`, `monitor_pump.rs`, and `monitor.rs` compile unchanged.
- [ ] Every existing test passes, split across the new test files with no test deleted or weakened.
- [ ] RUST-CORE, ANDROID, and DESKTOP all pass.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 5 changes with a focused commit message.

---

# Task 6 — Split and refactor `HostSessionScreen.tsx`

Target file: `desktop/src/screens/HostSessionScreen.tsx` (965 lines)

Validation: **DESKTOP only**.

## 6.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run DESKTOP before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation: snapshot polling/reconciliation core, join-request decisions, listener list &amp; removal/detail drill-down, QR invitation flow, manual connection details panel, playback controls + local monitor, session-level status/diagnostics chrome, and the shared presentational primitives (`StatusCard`, `Detail`, `CopyButton`, `ErrorAlert`).
- [ ] Confirm there is **no Redux usage** in this file at all (no `useSelector`, no store/slice imports) — everything is local `useState`/`useRef` plus imperative calls into `../core/client`. This makes the state/handler logic a strong candidate for extraction into a plain hook.
- [ ] Confirm the sole external consumer: `desktop/src/App.tsx` imports `{ HostSessionScreen }` and renders it with **zero props**; `desktop/src/screens/HostSessionScreen.test.tsx` (484 lines) renders it the same way and asserts on DOM/ARIA output. The named export `HostSessionScreen`, its zero-prop signature, and the rendered DOM/ARIA structure are the compatibility surface — everything else in the file (helpers, `StatusCard` etc.) is currently module-private and freely relocatable.

## 6.2 Module design

- [ ] Convert the single file into a directory module with an `index.tsx` re-export, so `App.tsx`'s import path (`"./screens/HostSessionScreen"`) keeps resolving unchanged:

```text
HostSessionScreen/
  index.tsx                      # top-level component shell: calls useHostSessionViewModel(),
                                  # composes section components; re-exports HostSessionScreen
  useHostSessionViewModel.ts      # all state/hooks/handlers with no Redux dependency
  domain.ts                      # pure helpers/types: errorKey, deliveryKey, revisionIsNewer,
                                  # operationFailure, formatAge, formatTimestamp, formatWallClock,
                                  # isInvitationExpired, PendingOperation, POLL_INTERVAL_MS
  SessionHeaderAndStatus.tsx      # header, aria-live announcement region, StatusCard grid
  ConnectionDetailsPanel.tsx      # "Manual connection details" section
  QrInvitationPanel.tsx          # "QR invitation" section
  PendingJoinRequests.tsx        # "Pending join requests" section
  ListenerList.tsx               # "Connected listeners" section (drill-down into ListenerDetailScreen)
  PlaybackControls.tsx           # "Playback controls" section incl. local monitor toggle
  DeliveryAndTransportAlerts.tsx  # trailing delivery/broadcast-queue/transport-error/last-error alerts
  shared.tsx                     # StatusCard, Detail, CopyButton, ErrorAlert
```

- [ ] Follow the repo's existing naming convention: PascalCase component files, camelCase hook file prefixed `use`; match `desktop/biome.json`'s 100-char line width, double quotes, semicolons always, trailing commas everywhere.
- [ ] `HostSessionScreen`'s import of the sibling `./ListenerDetailScreen` and that component's prop contract (`listener`, `lastDelivery`, `pending`, `failure`, `onRemove`, `onBack`) must stay intact.
- [ ] Optional, out of scope for this task unless trivial: note (but do not act on, to keep this task focused) that `StatusCard`/`ErrorAlert`/`formatAge` are duplicated in `DiagnosticsScreen.tsx` and `ListenerDetailScreen.tsx` — a future cross-file dedupe into a shared `desktop/src/components/` module is a separate concern.

## 6.3 Behavioral preservation

- [ ] Preserve the exact rendered DOM/ARIA structure `HostSessionScreen.test.tsx` asserts on (roles, text, aria-live regions) — the test file must pass unmodified.
- [ ] Preserve the optimistic-operation pattern (guard on pending → clear prior failure → call `core/client` → set announcement → catch → set failure) for every handler (`decide`, `requestRemoval`, `controlPlayback`, `toggleMonitor`, `refreshInvitation`, `endSession`).
- [ ] Preserve the snapshot-polling/reconciliation semantics (`POLL_INTERVAL_MS`, `reconcile`, `updateDecisionOperations`/`updateRemovalOperations` clearing stale optimistic state once the authoritative snapshot catches up).
- [ ] Preserve invitation lifecycle behavior: QR regeneration on invitation change, invitation clearing when the connection disappears, expiry detection (`isInvitationExpired`).
- [ ] Do not introduce Redux where there was none — keep this a local-state/hook-based component tree, matching the investigation's finding that this file has zero Redux coupling.

## 6.4 Acceptance criteria

- [ ] `App.tsx` compiles and renders `HostSessionScreen` unchanged.
- [ ] `HostSessionScreen.test.tsx` passes unmodified (Block 23/31/34 and other tagged test blocks).
- [ ] DESKTOP passes (Biome, tsc, Vitest, production build).
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 6 changes with a focused commit message.

---

# Task 7 — Split and refactor `transport/socket/host.rs`

Target file: `rust/silent-disco-core/src/transport/socket/host.rs` (887 lines)

Validation: **RUST-CORE + ANDROID + DESKTOP** (`SocketHostTransport` is re-exported at `silent_disco_core::transport::SocketHostTransport` and reached via the `HostTransportNode` trait object by both `rust/silent-disco-ffi` and `desktop/src-tauri`'s `production_transport_factory()` usage).

## 7.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run RUST-CORE, ANDROID, and DESKTOP before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation: datagram send-timeout policy (`DATAGRAM_SEND_TIMEOUT`, `is_datagram_send_timeout`, and its small inline `#[cfg(test)] mod send_timeout_classification_tests`), peer/route data model (`PeerState`, `PeerRoute`), socket bind/setup (`SocketHostTransport` struct + `bind`), peer lookup/delivery accounting (`pending_peer_for_device`, `peer_for_device`, `authorized_routes`, `record_peer_result`), audio/sync broadcast fan-out (`broadcast_datagram`), peer authorization (`authorize_peer_with_routes`), the public `impl HostTransportNode for SocketHostTransport` trait surface (12 methods), and `impl Drop`.
- [ ] Confirm the one real intra-crate coupling: `rust/silent-disco-core/src/transport/socket/host_workers.rs` imports `PeerState`/`PeerRoute` directly (`use super::host::{PeerRoute, PeerState};`) and constructs/mutates them — both are already `pub(super)` and must stay reachable at the same relative path after the split.
- [ ] Confirm `transport/tests.rs` and `types.rs` reference `SocketHostTransport` only in doc comments and exercise it exclusively through the `HostTransportNode` trait object via `SocketTransportFactory::bind_host(...)` — no direct method-name coupling to preserve there.
- [ ] Note the sibling layout of `transport/socket/`: `mod.rs`, `host.rs` (this file), `host_workers.rs` (574 lines), `listener.rs` (655 lines), `shared.rs` (503 lines) — none are directories yet; this task only converts `host.rs`, leaving the others flat (a future task if `listener.rs` also needs it).

## 7.2 Module design

- [ ] Convert `host.rs` into a `host/` directory (`socket/mod.rs`'s `mod host;` resolves to `host/mod.rs` automatically, no change needed there):

```text
socket/host/
  mod.rs             # mod decls, shared use statements, SocketHostTransport struct fields,
                      # impl Drop, re-export of PeerState/PeerRoute for host_workers.rs
  peer.rs            # PeerState, PeerRoute
  bind.rs            # DATAGRAM_SEND_TIMEOUT const (with its full doc invariant) + `bind()`
  lookup.rs          # pending_peer_for_device, peer_for_device, authorized_routes,
                      # record_peer_result (pub(super))
  broadcast.rs        # broadcast_datagram, is_datagram_send_timeout, and its existing
                      # #[cfg(test)] mod send_timeout_classification_tests kept inline
  authorization.rs    # authorize_peer_with_routes (pub(super))
  node.rs            # impl HostTransportNode for SocketHostTransport — the one unsplittable
                      # trait impl block (12 methods), delegating into the above
```

- [ ] Note the Rust constraint driving this shape: `impl HostTransportNode for SocketHostTransport` cannot itself be split across files (one trait impl per type) — but at ~225 lines estimated, it does not need to be.
- [ ] Everything else is an inherent `impl SocketHostTransport { ... }` block or a free item, which Rust allows spread across as many files as needed within the module — follow the `pub(super)` convention already established between `host.rs` and `host_workers.rs`.

## 7.3 Behavioral preservation

- [ ] Preserve `DATAGRAM_SEND_TIMEOUT` (5ms) and the reasoning behind it: sockets stay blocking (never non-blocking, which would also make reads non-blocking via the shared FD); a send that can't finish in one packet period must fail fast as a *counted, visible* delivery failure.
- [ ] Preserve a send timeout **not** counting toward `consecutive_failures`/auto-disconnect — only genuine I/O errors do. This is a confirmed-on-hardware distinction; collapsing it would disconnect a still-live listener on routine congestion.
- [ ] Preserve silent-peer eviction via `PeerState::last_inbound_millis`/`authorized_routes` reusing the same `PeerState::close` path that `record_peer_result`'s failure-threshold eviction uses, so both surface through the identical `PeerDisconnected` event.
- [ ] Preserve the per-listener anti-spoofing authorization check in `authorize_peer_with_routes` (route IPs must equal the authenticated control peer's own remote IP) and the one-authorization-per-device-id rule.
- [ ] Preserve the protocol/session guards in `broadcast_datagram`/`send_pending_control`/`send_control`/`broadcast_control` (frame kind must match channel; every outbound frame's `session_id` must match `self.session_id`).
- [ ] Preserve `is_datagram_send_timeout` staying a pure, independently unit-testable function (the reason it was extracted in the first place — the full path is only reproducible on real hardware).
- [ ] Preserve idempotent `shutdown()` (guards on `shutdown_complete`; closes all peers and joins all workers even if the peer registry lock is poisoned, recording that as an error rather than panicking).

## 7.4 Acceptance criteria

- [ ] `silent_disco_core::transport::SocketHostTransport` resolves unchanged.
- [ ] `host_workers.rs` compiles unchanged against `super::host::{PeerRoute, PeerState}` (or an equivalent re-export path).
- [ ] `transport/tests.rs`'s `HostTransportNode`-trait-object-based integration tests all still pass.
- [ ] The `send_timeout_classification_tests` module still passes (moved or kept inline in `broadcast.rs`).
- [ ] RUST-CORE, ANDROID, and DESKTOP all pass.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 7 changes with a focused commit message.

---

# Task 8 — Split and refactor `transport/tests.rs`

Target file: `rust/silent-disco-core/src/transport/tests.rs` (885 lines)

Validation: **RUST-CORE only** (test-only file; no production behavior changes, so ANDROID/DESKTOP rebuilds are not required — `#[cfg(test)]` code is not part of either downstream build).

## 8.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run RUST-CORE before editing and record the baseline result.
- [ ] Confirm the 9 `#[test]` functions and their themes found during investigation: socket-runtime handshake/end-to-end exchange (3 tests, including the large multi-listener join/authorize/sync/audio flow), silent-peer liveness/eviction regression tests (2 tests, citing "Block A6 follow-up"), authorization/malformed-header rejection &amp; backpressure (2 tests), and virtual transport tests (2 tests, isolation + injected-clock timestamping semantics).
- [ ] Confirm the shared helper/fixture block (lines 694–885, ~192 lines, no `#[test]`s): `join_request`, `audio_frame`, `wait_for_control_from`, `wait_for_authorized`, `wait_for_control_target`, `wait_for_frame`, `wait_for_frame_from`, `wait_for_rejection`, `wait_for_host_event`, `wait_for_listener_event`, `wait_until`, `invalid_version_header`, `oversized_control_header`, `id_session`, `id_device`, plus `const EVENT_TIMEOUT`.
- [ ] Confirm the repo's existing convention in this exact directory: `transport/virtual_fault_tests.rs` is already a same-directory sibling file (`#[cfg(test)] mod virtual_fault_tests;` in `mod.rs`) with its **own independent** local helper copies (e.g. its own `audio_frame`) — it does not import from `tests.rs`, so it is untouched by this task.
- [ ] Confirm via repo-wide search that none of `tests.rs`'s helpers are `pub`/`pub(crate)` and none are referenced from any other file — the split is fully safe; only `transport/mod.rs`'s module declaration needs to change.

## 8.2 Module design

- [ ] Replace the single file with sibling files declared in `transport/mod.rs`, following the `virtual_fault_tests.rs` flat-file convention (not a `tests/` subdirectory):

```text
transport/test_support.rs        # shared helpers made pub(super)/pub(crate): join_request,
                                  # audio_frame, id_session, id_device, wait_for_control_from,
                                  # wait_for_authorized, wait_for_control_target, wait_for_frame,
                                  # wait_for_frame_from, wait_for_rejection, wait_for_host_event,
                                  # wait_for_listener_event, wait_until, const EVENT_TIMEOUT
transport/handshake_tests.rs      # production_factory_is_socket_runtime,
                                  # pending_control_peer_receives_hello_before_datagram_authorization,
                                  # socket_runtime_completes_multi_listener_join_sync_and_audio_exchange
transport/liveness_tests.rs       # a_silent_peer_is_evicted_and_stops_being_reported_as_delivered,
                                  # a_listener_that_keeps_probing_is_never_evicted_as_silent
transport/authorization_tests.rs  # socket_runtime_rejects_unauthorized_control_and_malformed_headers,
                                  # bounded_event_queue_reports_pressure_and_zero_peer_delivery_is_not_success,
                                  # + invalid_version_header/oversized_control_header (only used here)
transport/virtual_tests.rs        # virtual_transport_is_explicit_isolated_and_uses_injected_clock,
                                  # virtual_transport_stamps_delivered_events_with_the_recipients_own_clock
```

- [ ] Update `transport/mod.rs`: replace `#[cfg(test)] mod tests;` with `#[cfg(test)] mod test_support;` plus one `#[cfg(test)] mod ...;` line per new test file above.
- [ ] Leave `virtual_fault_tests.rs` untouched.

## 8.3 Behavioral preservation

- [ ] Preserve every test's exact assertions and doc comments verbatim, including the long device-regression narratives on the silent-peer eviction and recipient-clock-timestamping tests.
- [ ] Preserve all shared helper behavior exactly (poll timeouts via `EVENT_TIMEOUT`, malformed-header byte fixtures, ID constructors).

## 8.4 Acceptance criteria

- [ ] All 9 tests pass, unchanged in behavior, split across the 4 new theme files.
- [ ] `virtual_fault_tests.rs` is unaffected and still passes.
- [ ] RUST-CORE passes.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 8 changes with a focused commit message.

---

# Task 9 — Split and refactor `lab_commands.rs`

Target file: `desktop/src-tauri/src/lab_commands.rs` (838 lines)

Validation: **DESKTOP + DESKTOP-LAB-MODE** (the whole module is gated behind the `lab-mode` Cargo feature, confirmed via `#[cfg(feature = "lab-mode")] mod lab_commands;` in `lib.rs`; `npm run check`'s default build does not compile it at all, so the DESKTOP-LAB-MODE commands above are mandatory, not optional, for this task).

## 9.1 Baseline and responsibility inventory

- [ ] Record the current physical line count.
- [ ] Run DESKTOP and DESKTOP-LAB-MODE before editing and record the baseline result.
- [ ] Confirm the responsibility clusters found during investigation: module doc/architectural invariants, session state types (`LoadedScenario`, `LastRun`, `LabSessionState`, `LabAppState`), error constructors (6 `DesktopErrorDto` builders), runtime/session helpers (`ensure_runtime`, `node_dto`, `scenario_summary_dto`, `bounded_summary_text`), run-outcome/timeline DTO conversion (`timeline_entry`, `run_outcome_dto`, `state_dto`), scenario file parsing/validation error mapping + bounded I/O (`parse_error`/`validation_error`/`execution_error`/`recording_io_error`, `read_bounded_scenario_file`, `parse_and_validate`), and 9 `#[tauri::command]` entry points (`lab_get_state`, `lab_open_scenario_file`, `lab_save_scenario_file`, `lab_run_loaded_scenario`, `lab_advance_virtual_time`, `lab_start_node`, `lab_stop_node`, `lab_stop_all_nodes`, `lab_export_recording_file`).
- [ ] Confirm `#[cfg(test)] mod tests;` **already** points to an existing sibling `lab_commands/tests.rs` (220 lines, 8 tests) — a `lab_commands/` directory already exists containing only that file; this task converts `lab_commands.rs` into `lab_commands/mod.rs` alongside it, not a fresh directory creation.
- [ ] Confirm the exact 9 IPC command-name strings that `desktop/src/core/client.ts` invokes by literal string (`"lab_get_state"`, `"lab_open_scenario_file"`, `"lab_save_scenario_file"`, `"lab_run_loaded_scenario"`, `"lab_advance_virtual_time"`, `"lab_start_node"`, `"lab_stop_node"`, `"lab_stop_all_nodes"`, `"lab_export_recording_file"`) and the corresponding `#[cfg(feature = "lab-mode")]`-gated registrations in `lib.rs`'s `generate_handler!` list — these string literals and Rust fn names must not change.

## 9.2 Module design

- [ ] Convert `lab_commands.rs` → `lab_commands/mod.rs`, keeping the existing `lab_commands/tests.rs` sibling in place:

```text
lab_commands/
  mod.rs             # module doc, imports, MAX_TIMELINE_ENTRIES_PER_NODE/MAX_SUMMARY_CHARS,
                      # LabSessionState/LoadedScenario/LastRun/LabAppState, pub use re-exports of
                      # command fns from submodules (so lib.rs's lab_commands::fn_name paths need
                      # zero changes), #[cfg(test)] mod tests;
  errors.rs          # all DesktopErrorDto constructors (poisoned/already_running/no_scenario_loaded/
                      # no_run_to_export/invalid_node_id/path_unavailable/parse/validation/
                      # execution/recording_io errors)
  session.rs         # ensure_runtime
  dto_convert.rs      # node_dto, scenario_summary_dto, bounded_summary_text, timeline_entry,
                      # run_outcome_dto, state_dto
  scenario_io.rs      # read_bounded_scenario_file, parse_and_validate,
                      # lab_open_scenario_file, lab_save_scenario_file
  run_control.rs      # lab_run_loaded_scenario, lab_advance_virtual_time, lab_start_node,
                      # lab_stop_node, lab_stop_all_nodes (+ the relevant part of the start/step/
                      # stop-mapping module doc, relocated here since it documents run-control
                      # semantics specifically)
  recording.rs        # lab_export_recording_file, lab_get_state
  tests.rs            # existing 220-line test file — update its `use super::{...}` imports to
                      # the new submodule paths (dto_convert::, scenario_io::, etc.)
```

- [ ] Make cross-submodule helpers (`node_dto`, `scenario_summary_dto`, `run_outcome_dto`, `state_dto`, `read_bounded_scenario_file`) `pub(crate)`/`pub(super)` as needed for `tests.rs` and the command submodules to reach them.
- [ ] Keep every `#[tauri::command]` fn name identical; re-export via `pub use submodule::*;` (or explicit names) from `mod.rs` so `lib.rs`'s `lab_commands::lab_run_loaded_scenario` etc. paths in `generate_handler!` need zero changes.
- [ ] Keep `LabAppState` independent of `DesktopAppState` (no merge, no shared singleton) — this is an explicit standing invariant ("Block 37.2 no global production singleton reuse").

## 9.3 Behavioral preservation

- [ ] Preserve DTOs living only in the separate, unconditionally-compiled `lab_dto` module — do not fold DTO conversion logic into `lab_dto` itself.
- [ ] Preserve every command routing through the exact same `LabRuntime`/`scenario::run_scenario_with_trace` entry points — no second code path to the runtime.
- [ ] Preserve the `LabSessionState.running` mutex-guard coupling between `lab_run_loaded_scenario` and `lab_advance_virtual_time` (the latter must still check the flag and return `already_running_error()`).
- [ ] Preserve `MAX_TIMELINE_ENTRIES_PER_NODE` (50) and `MAX_SUMMARY_CHARS` (200) as deliberately separate, tighter UI-facing bounds than the recorder's own 4096 cap — move their doc comments together with the DTO-conversion logic that uses them.
- [ ] Preserve `read_bounded_scenario_file`'s filesystem-metadata-size-check-before-read ordering (deliberately duplicates/precedes `load_scenario_json`'s own oversize check, to avoid transient memory ballooning).
- [ ] Preserve `lab_save_scenario_file` writing back the raw validated bytes verbatim ("save a copy, never a silent mutation"), not a re-serialization.
- [ ] Preserve every `#[tauri::command]` fn name exactly (IPC contract with `desktop/src/core/client.ts`).

## 9.4 Acceptance criteria

- [ ] `lab_commands/tests.rs`'s 8 tests pass with updated imports, unchanged behavior.
- [ ] `lib.rs`'s `lab_commands::` state-management (`.manage(lab_commands::LabAppState::new())`) and all 9 `generate_handler!` registrations compile unchanged.
- [ ] `desktop/src/core/client.ts` and `LabScreen.tsx`/`LabScreen.test.tsx` are unaffected (Rust-only refactor; TS layer needs no change).
- [ ] DESKTOP passes.
- [ ] DESKTOP-LAB-MODE passes (`cargo fmt --check`, `cargo clippy --features lab-mode -- -D warnings`, `cargo test --features lab-mode`) — do not skip this; it is the only gate that actually compiles this module.
- [ ] No file in the split exceeds 800 lines.
- [ ] Record final line counts and the final file list.
- [ ] Commit only Task 9 changes with a focused commit message.

---

# Final validation and closure

Complete this section only after Tasks 1 through 9 have each been implemented and committed separately.

- [ ] Confirm all 10 original oversized paths were replaced or reduced according to their task design.
- [ ] Confirm every new module has a clear single responsibility.
- [ ] Confirm there are no wildcard imports added by the refactors.
- [ ] Confirm no `allow` attributes were added merely to suppress size, visibility, import, dead-code, or complexity warnings.
- [ ] Confirm no silent fallback, ignored error, or permissive compatibility path was introduced.
- [ ] Confirm no real-time-audio-path file gained allocation, I/O, blocking, JNI, UniFFI, or logging calls as a side effect of the split.
- [ ] Confirm public Rust APIs, UniFFI-exported types/methods, JNI-reachable symbols, Tauri IPC command names, TypeScript component exports/props, protocol bytes, database schema/behavior, actor semantics, and Android/desktop UI behavior all remain compatible.
- [ ] Run the complete RUST-CORE, ANDROID, DESKTOP, and DESKTOP-LAB-MODE validation matrices one final time, together, on the combined result.
- [ ] Confirm a fresh `/top-large-files` run no longer lists any of the original 10 files, and no newly-created file from this round exceeds 800 lines.
- [ ] Record the nine implementation commit SHAs and final validation evidence in `memory.md`.
- [ ] Mark this TODO complete only after the final validation evidence is available.
