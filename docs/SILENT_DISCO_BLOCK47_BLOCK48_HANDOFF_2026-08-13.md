# Silent Disco Blocks 47–48 Handoff — 2026-08-13

## Purpose

This document captures the exact project state at the point work paused on **Block 47 — Final Android interoperability acceptance** and **Block 48 — Final documentation and completion audit**. It is intended to be sufficient for a later ChatGPT/Claude/engineer session to resume without reconstructing the current state from chat history.

Pause timestamp: **2026-08-13 13:01 PDT**.

Repository: `ekkus93/silent_disco`

Branch: `master`

Implementation state captured before this handoff commit:

```text
2ce953a451789d6d0191dd7bb47f7a48d439e72d
Account for deferred sync acceptance test
```

This handoff document itself is added after that implementation-state commit, so branch HEAD will be newer once this file is committed.

---

# 1. Executive status

The Linux desktop-host implementation is in **release acceptance and closure**, not feature-development mode.

Most desktop-host functionality and automated validation are already implemented. The remaining work is concentrated in four areas:

1. **Block 46.3 fresh-machine package validation** on a real graphical Ubuntu 22.04 environment with a physical Android listener.
2. **Block 47 complete physical Android interoperability acceptance** with a packaged Linux desktop build and two physical Android devices.
3. A known **two-listener startup synchronization/alignment risk** in the shared Rust playback scheduler that must not be waved through merely because other tests pass.
4. **Block 48 final audit cleanup and final gates**, including stale references in the authoritative desktop TODO and a final evidence ledger in `memory.md`.

Do **not** mark Blocks 47 or 48 complete until physical acceptance evidence and the final release gates genuinely pass.

The user explicitly prefers that the assistant **does not monitor GitHub CI jobs during Ralph Loop work**. The user will monitor CI and report failures. Local/sandbox validation should still be run as fully as the available environment permits.

---

# 2. Authoritative files to read first when resuming

Read these before making further changes:

```text
CLAUDE.md
README.md
memory.md
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md
docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md
docs/AUDIO_PLAYBACK_STATE_2026-08-10.md
```

Also read this handoff file in full.

The authoritative completion ledger for the Linux desktop host is:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

This handoff explains the state and evidence; the TODO remains the checkbox ledger.

---

# 3. Current block status

## Block 46.2 — Linux package lifecycle

**Status: complete on the supported Ubuntu 22.04 CI baseline.**

The TODO records that package contents, clean install, synthetic-version upgrade, launch without a development server, uninstall behavior, and profile-local data preservation passed on the supported CI baseline.

Relevant commits:

```text
a2fbd3a8347446707b02a2dc0ef2f5c3c2ede81e  Close Block 46.2 package behavior
28d5e3c868fbbc512b56a2d7aa390833ffe57c31  Run Block 46.2 bookkeeping closure
6716402a2128db6632eb3411b5473d5419e1f4db  Keep Block 46 evidence outside repository
```

## Block 46.3 — Fresh-machine validation

**Status: open.**

Validator:

```text
desktop/scripts/validate-block46-fresh-machine.sh
```

Related commits:

```text
958bda455f06f729fd86f496f3c43c9c06bda13f  Add Block 46.3 fresh-machine validator
2532d079c68246ff34b00166e066207bffd7c1a0  Set fresh-machine validator executable bit
```

This still needs a real graphical Ubuntu 22.04 machine/VM and a physical Android listener. CI/package simulation does not replace this evidence.

Run:

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle
```

## Block 47 — Final Android interoperability acceptance

**Status: open.**

The acceptance runner and documentation now exist, but the complete production matrix has not been accepted on two physical Android devices.

Runner:

```text
desktop/scripts/validate-block47-android-interoperability.sh
```

Commit:

```text
270472655b4c6f9bb86752b4bef0a12773558f5e  Add Block 47 Android acceptance runner
```

The runner is deliberately fail-closed. Any `FAIL`, `BLOCKED`, or `NOT RUN` leaves Block 47 open and returns non-zero.

## Block 48.1 — Developer documentation

**Status: substantially implemented; TODO checkbox state still needs final reconciliation.**

`README.md` now documents:

- desktop prerequisites;
- Android prerequisites;
- clean builds;
- Tauri development launch;
- production bundle creation;
- Rust/Android/desktop test gates;
- physical desktop-to-Android interoperability;
- Block 46.3 fresh-machine validation;
- Lab Mode deterministic scenario procedure;
- diagnostics location/export;
- Linux Secret Service / secure-store troubleshooting;
- explicit prohibition on insecure secret fallback.

Commit:

```text
bc06380114deebccbc9f2ff4ed33bde53c7168a9  Document desktop acceptance workflow
```

`CLAUDE.md` was refreshed so it no longer treats the README as stale and now describes the Ubuntu 22.04 desktop-host release goal.

Commit:

```text
bf23ba1548996a368582b84f70bda0981e0089c8  Refresh current project guidance
```

## Block 48.2 — Ownership audit

**Status: complete in the TODO.**

The TODO already marks these architecture invariants complete:

- Rust actor authoritative;
- React presentation-only;
- Tauri backend platform-only;
- protocol Rust-only;
- synchronization Rust-only;
- packetization Rust-only;
- transport semantics Rust-only;
- SQLite Rust-only;
- PCM does not cross IPC;
- local monitor uses shared timeline;
- Lab adapters cannot silently activate in production.

Do not reopen these without concrete contradictory production behavior.

## Block 48.3 — Final gates

**Status: open.**

The final release-state gate ledger still requires proof for:

- shared Rust format;
- shared Rust strict Clippy;
- shared Rust tests;
- Android tests;
- Android lint;
- Android instrumentation;
- frontend format/lint/typecheck/tests/build;
- Tauri format/strict Clippy/tests/check;
- Linux bundle build;
- deterministic Lab scenarios;
- loopback transport integration;
- physical Android acceptance.

Several passed in earlier blocks, but Block 48 requires a final release-state run. Do not close the final-gate checklist from stale evidence after material code changes.

## Block 48.4 — Honest completion

**Status: partially complete and still open.**

Already checked in the TODO:

- unresolved platform/device limitations are listed;
- Windows/macOS are not claimed unless validated.

Still open in the TODO:

- every skipped test has a reason and owner;
- every referenced file exists at the exact path;
- `memory.md` contains the final ledger.

Ignored-test accountability has now been implemented in code, but the TODO checkbox still needs final verification/reconciliation.

The exact-path requirement is **not ready to close** because the authoritative desktop TODO still contains stale references left over from source-file splits.

---

# 4. Work completed immediately before this pause

The following commits were added directly to `master` during the Block 47/48 pass:

```text
bc06380114deebccbc9f2ff4ed33bde53c7168a9  Document desktop acceptance workflow
270472655b4c6f9bb86752b4bef0a12773558f5e  Add Block 47 Android acceptance runner
21a219e6bcbbc9a6b38a1ab755d085c5a6491cf3  Add Block 48 completion audit
bf23ba1548996a368582b84f70bda0981e0089c8  Refresh current project guidance
65f53daef228ec31692f881bcb51adef094b644a  Account for manual interoperability tests
2ce953a451789d6d0191dd7bb47f7a48d439e72d  Account for deferred sync acceptance test
```

No side branch contains required unpublished work from this pass.

Primary files changed/added:

```text
README.md
CLAUDE.md
desktop/scripts/validate-block47-android-interoperability.sh
scripts/audit-block48-completion.py
desktop/src-tauri/src/platform/start_playback_tests/manual.rs
rust/silent-disco-core/src/audio/scheduler/resync_tests.rs
```

`.github/workflows/ci.yml` was **not** successfully changed in this pass; see the CI caveat below.

---

# 5. Block 47 physical acceptance runner

## Required environment

The runner expects:

- graphical Ubuntu **22.04** amd64;
- active D-Bus user session;
- working Linux Secret Service provider/keyring;
- packaged desktop `.deb` and AppImage;
- Android debug APK for the intended release candidate;
- two distinct **physical** Android devices attached via `adb`;
- real LAN/Wi-Fi connectivity suitable for desktop ↔ Android transport.

It rejects emulator-like devices for the physical acceptance run.

## Invocation

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

An optional fifth argument specifies the evidence directory. Otherwise it uses a timestamped directory similar to:

```text
$HOME/silent-disco-block47-evidence-YYYYMMDDTHHMMSSZ/
```

## Evidence recorded

The runner records:

- desktop package version;
- `.deb` SHA-256;
- AppImage SHA-256;
- Android APK SHA-256;
- installed Android app version;
- device serial/manufacturer/model;
- Android version/API level;
- device ABI/fingerprint;
- network topology;
- commands executed;
- per-matrix PASS/FAIL/BLOCKED/NOT RUN status;
- evidence notes;
- diagnostics export hash;
- measured synchronization notes;
- known limitations;
- structured final evidence JSON.

## Matrix covered

1. manual endpoint join;
2. mDNS discovery;
3. QR invitation;
4. rejection followed by approval;
5. one-listener audio;
6. two-listener audio;
7. pause/resume/stop/end;
8. Android disconnect/reconnect;
9. desktop network-interface disruption and recovery;
10. host source failure;
11. local monitor failure while preserving listener-transmit policy;
12. desktop restart;
13. controlled clean shutdown;
14. profile/session-history reopen;
15. diagnostics export;
16. explicit synchronization measurement/observation;
17. explicit known-limitations record.

The runner succeeds only when **every matrix item is PASS**. Do not weaken this behavior.

---

# 6. Critical unresolved synchronization risk

This is the most important technical concern to revisit before treating Block 47 as a routine checklist run.

## Ignored acceptance test

File:

```text
rust/silent-disco-core/src/audio/scheduler/resync_tests.rs
```

Test:

```text
two_listeners_locking_sync_at_different_moments_play_the_same_audio_together
```

It remains ignored because the intended cross-listener startup-alignment behavior is not safely implemented.

Its ignore annotation now records:

```text
reason: alignment is unimplemented after the first attempt regressed on-device and was reverted
owner: shared audio synchronization / Block 47 physical acceptance
```

Commit:

```text
2ce953a451789d6d0191dd7bb47f7a48d439e72d  Account for deferred sync acceptance test
```

## Why it matters

Two listeners can finish synchronization/buffering at different moments. If each starts from the head of its local buffered stream rather than the host-timeline frame that should actually be heard at that moment, listeners can be shifted by different startup delays while both appear individually healthy.

The target property is that listeners locking at different moments still play the **same authoritative host timeline together**, within the intended packet/timing tolerance.

## Previous failed approach

A prior implementation tried to skip frames already due when a late-starting listener began. That change regressed a real physical listener and was reverted.

The reconstructed failure mechanism:

- `PlaybackPump` deliberately writes audio into the render ring ahead of actual playback;
- it computes a release horizon roughly as `local_now + write_lead`;
- write-ahead is on the order of hundreds of milliseconds;
- the failed logic treated that future write horizon as actual current monotonic time;
- valid future audio was therefore treated as stale/already due;
- this caused aggressive dropping/resynchronization and real-device playback regression.

Relevant files:

```text
rust/silent-disco-core/src/audio/playback_pump/scheduling.rs
rust/silent-disco-core/src/audio/scheduler/engine.rs
rust/silent-disco-core/src/audio/scheduler/resync_tests.rs
```

The current pump deliberately does the equivalent of:

```text
poll_time_ms = local_now_ms + write_lead_ms
scheduler.poll(poll_time_ms)
```

## Safer direction

Do **not** simply reapply the old "skip everything before poll time" behavior.

A safer design should explicitly distinguish:

1. **actual local monotonic now** — what is physically already in the past;
2. **write/release horizon** — how far ahead the pump is filling the render ring.

If startup alignment discards host frames that genuinely cannot be heard on time, stale/due decisions should be based on true current time plus known output/ring latency, not blindly on the write-ahead horizon.

A likely implementation shape is to make scheduler/pump interfaces explicit about both notions of time instead of overloading one `poll(local_now_ms)` argument.

Any fix needs regression coverage for the previous failure: a large write lead must **not** cause valid future frames to be discarded merely because they are inside the pump's prefill horizon.

After a fix, run at minimum:

```bash
bash scripts/check-rust.sh
```

plus Android/shared-playback tests and real-device playback. This path has already demonstrated that non-device tests alone can miss a damaging timing regression.

Do not mark Block 47 complete while this issue remains known but unvalidated.

---

# 7. Ignored-test accountability

Block 48.4 requires every skipped test to have a reason and owner.

Four manual desktop interoperability tests now carry explicit `reason` and `owner` annotations in:

```text
desktop/src-tauri/src/platform/start_playback_tests/manual.rs
```

They cover:

- real Android WAV/song-change manual listener test;
- real Android FLAC test;
- real Android MP3 test;
- two-emulator listener integration test.

Commit:

```text
65f53daef228ec31692f881bcb51adef094b644a  Account for manual interoperability tests
```

The shared-core two-listener alignment acceptance test also now carries reason/owner accountability.

Before checking the TODO item, rerun the audit and/or independently grep Rust sources for `#[ignore]` so a later newly-added skip is not missed.

---

# 8. Block 48 completion audit

File:

```text
scripts/audit-block48-completion.py
```

Commit:

```text
21a219e6bcbbc9a6b38a1ab755d085c5a6491cf3  Add Block 48 completion audit
```

This dependency-free Python-standard-library audit checks:

- exact repository paths referenced by selected maintained guidance files;
- required Block 48 README sections/commands;
- reason/owner accountability for ignored Rust tests.

Run:

```bash
python3 scripts/audit-block48-completion.py
```

## Important limitation

The current master version audits:

```text
README.md
CLAUDE.md
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md
```

It does **not yet include** the authoritative TODO:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

because that TODO still contains stale paths from prior file splits.

Therefore a passing `scripts/audit-block48-completion.py` result is **not sufficient by itself** to close Block 48.4's "every referenced file exists at the exact path" item.

The last reported lightweight local result in this pass was:

```text
25 repository references checked
5 ignored Rust tests accounted for
required developer-guidance sections present
```

Treat this as the scope of the current lightweight audit, not proof that the full TODO is clean.

Once the stale TODO references are fixed, add the TODO back into the exact-reference audit and rerun it.

---

# 9. Known stale references in the desktop TODO

At least the following old references remain inside:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

Known stale references identified during the audit:

```text
listener_playback.rs
lab_commands.rs
app/coreSlice.ts
app_state.rs:327
transport/socket/host.rs:73-90
```

These are remnants of source splitting/refactoring.

Do not mechanically delete them merely to make an audit green. For each one:

1. locate the current implementation satisfying the original task/assertion;
2. replace the stale path with the correct current path(s);
3. preserve semantic traceability;
4. rerun exact-reference validation.

After repair, update `scripts/audit-block48-completion.py` so the desktop TODO itself is included in audit scope.

Only then consider closing the exact-path checkbox.

---

# 10. README/developer instructions now in place

The current `README.md` is maintained release/developer guidance.

Important commands include:

## Shared Rust

```bash
bash scripts/check-rust.sh
```

## Android build/test/lint

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

## Android managed-device instrumentation

```bash
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain
```

## Desktop aggregate gate

```bash
cd desktop
npm ci
npm run check
```

## Desktop Rust strict gates

```bash
cd desktop/src-tauri
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --all-features
```

## Production bundle

```bash
cd desktop
npm ci
npm run tauri build
```

## Development launch

```bash
cd desktop
npm ci
npm run tauri dev
```

## Lab Mode

```bash
cd desktop
npm ci
npm run tauri:lab:dev
```

The README also includes a deterministic Lab scenario example, diagnostics/export guidance, and fail-closed Linux Secret Service troubleshooting.

Production secure-store failures must remain visible. Do not add production fallback through plaintext files, environment variables, SQLite, React preferences/state, or random-per-launch identity.

---

# 11. CI/workflow caveat

An attempt was made to wire the Block 48 lightweight audit and a short-lived physical-acceptance Android APK artifact into `.github/workflows/ci.yml`.

That workflow edit was **not committed to `master`** because the available GitHub workflow-file write path was restricted in that environment.

Therefore, as of the state captured here:

- do **not** assume `scripts/audit-block48-completion.py` runs automatically in CI;
- do **not** assume CI publishes an artifact named `android-physical-acceptance-apk`;
- use existing Android build outputs/artifacts or build the APK explicitly for Block 47;
- if automatic CI integration is desired, add it later through a normal git/PR workflow with appropriate workflow permissions.

This CI integration would be useful hardening, but it is not a substitute for physical Block 47 acceptance.

---

# 12. Validation actually performed in the last pass

The last work environment did **not** have a complete toolchain/cache for every project gate.

Validated locally/syntactically:

- shell syntax for the Block 47 acceptance runner;
- shell syntax for the Block 46.3 validator;
- dependency-free Block 48 Python audit;
- repository/reference inspection against current `master`.

Environment limitations:

- Rust/Rustup were unavailable in the sandbox used for that pass;
- Node offline cache was incomplete; `npm ci --offline` could not satisfy all dependencies and `yargs-parser` was one missing package reported during investigation.

Therefore the pass **did not claim** a fresh successful run of:

- shared Rust format/Clippy/tests;
- desktop Rust Clippy/tests/check;
- full frontend `npm run check`;
- Android Gradle tests/lint/instrumentation;
- Linux bundle build;
- physical Android acceptance.

Do not convert inability to run a gate into a pass.

---

# 13. Recommended resume order

## Step 1 — Re-read state and verify `master`

Start with:

```bash
git status
git branch --show-current
git log --oneline -15
```

Confirm this handoff and the commits listed above are present.

If a fresh zip of `master` is supplied, use that as the local source of truth as the user normally requests. Do not assume unmerged branches contain required work.

## Step 2 — Fix stale TODO references

Update:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

by mapping each stale reference to the current production source location.

Then include the TODO in `scripts/audit-block48-completion.py` and run:

```bash
python3 scripts/audit-block48-completion.py
```

Do not close exact-path completion until the TODO itself is in scope and passes.

## Step 3 — Resolve/reassess two-listener startup alignment

Investigate:

```text
rust/silent-disco-core/src/audio/scheduler/engine.rs
rust/silent-disco-core/src/audio/scheduler/resync_tests.rs
rust/silent-disco-core/src/audio/playback_pump/scheduling.rs
```

Goal:

- separate actual current monotonic time from the future write horizon;
- skip only audio genuinely no longer schedulable/audible on the host timeline;
- avoid the previous real-device regression;
- add regression coverage for the large write-lead case;
- eventually make the ignored cross-listener alignment test a normal passing test when behavior is truly implemented.

## Step 4 — Run shared Rust gates

```bash
bash scripts/check-rust.sh
```

Fix all format, strict Clippy, and test failures before proceeding.

## Step 5 — Run Android and desktop final automated gates

At minimum:

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

```bash
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain
```

```bash
cd desktop
npm ci
npm run check
npm run tauri build
```

and:

```bash
cd desktop/src-tauri
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --all-features
```

Also run the deterministic Lab and loopback integration gates required by Block 48.3.

## Step 6 — Complete Block 46.3 fresh-machine validation

Use graphical Ubuntu 22.04 amd64 plus a real Android listener:

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle
```

Capture external evidence as designed by the validator.

## Step 7 — Build/identify exact Block 47 artifacts

Use a packaged Linux desktop bundle and Android APK corresponding to the same intended release candidate. Record hashes before testing.

Do not accidentally test an old installed APK against a newer desktop package and call it final acceptance.

## Step 8 — Run Block 47 with two physical Android devices

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

If anything is `FAIL`, `BLOCKED`, or `NOT RUN`, leave Block 47 open and fix/retest.

Pay particular attention to:

- whether two devices hear the same audio at effectively the same time;
- whether one listener's later startup/reconnect shifts it relative to the other;
- whether diagnostics reflect true delivery/sync state;
- whether network disruption creates false-success UI;
- whether local-monitor failure improperly kills or masks listener transmission;
- whether source failure silently substitutes/falls back;
- whether restart/reopen preserves expected profile/session history.

## Step 9 — Re-run final Block 48 gates after physical fixes

If Block 47 exposes a production issue and code changes, rerun applicable Block 48.3 gates afterward. The final release ledger must represent the code actually physically accepted.

## Step 10 — Final documentation/TODO/memory closure

Once the release candidate passes:

1. mark only genuinely satisfied Block 47 matrix items;
2. reconcile Block 48.1 documentation items against the current README;
3. mark ignored-test accountability only after the final grep/audit;
4. mark exact-path references only after the TODO is included in the audit and passes;
5. add the final release/evidence ledger to `memory.md`;
6. record package/APK hashes, device models/OS versions, topology, commands, physical results, sync measurements, and known limitations;
7. close Blocks 47 and 48 only if their acceptance statements are actually true.

---

# 14. Final-gate command checklist

## Shared Rust

```bash
bash scripts/check-rust.sh
```

## Android JVM/build/lint

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

## Android instrumentation

```bash
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain
```

## Desktop aggregate

```bash
cd desktop
npm ci
npm run check
```

## Desktop Rust strict

```bash
cd desktop/src-tauri
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --all-features
```

## Production bundle

```bash
cd desktop
npm run tauri build
```

## Bundle verification

```bash
python3 desktop/scripts/verify-linux-bundles.py \
  --bundle-dir desktop/src-tauri/target/release/bundle \
  --tauri-config desktop/src-tauri/tauri.conf.json \
  --cargo-manifest desktop/src-tauri/Cargo.toml
```

## Package lifecycle

```bash
bash desktop/scripts/smoke-linux-package-lifecycle.sh \
  desktop/src-tauri/target/release/bundle \
  com.ekkus.silentdisco.desktop \
  silent-disco-desktop
```

## Block 48 lightweight audit

```bash
python3 scripts/audit-block48-completion.py
```

Remember the current TODO-audit limitation.

## Block 46.3

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle
```

## Block 47

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

---

# 15. Release principles that must not be weakened

## No silent fallback

Do not add fake/demo/in-memory/plaintext-secret/alternate-decoder/alternate-protocol/local-file fallbacks that make a broken production path look successful.

## No fabricated evidence

Never claim CI, physical hardware, instrumentation, builds, or tests passed unless the corresponding evidence really exists.

## Rust remains authoritative

Do not duplicate legality/state ownership in React/Kotlin/Tauri to work around final bugs.

## Real-time path remains constrained

Do not solve synchronization by adding blocking/logging/SQLite/UniFFI/JNI work to the real-time audio callback.

## Physical sync quality outranks paperwork

If two listeners are audibly shifted, Block 47 fails even if documentation is perfect.

## Platform scope remains Linux + Android for this release

Do not claim Windows/macOS support from code portability alone.

---

# 16. Quick status table

| Area | State at pause | What closes it |
|---|---|---|
| Block 46.2 package lifecycle | Complete | Already accepted on Ubuntu 22.04 CI baseline |
| Block 46.3 fresh-machine | Open | Graphical Ubuntu 22.04 + physical Android validation |
| Block 47 runner | Implemented | Runner exists; matrix must still be physically executed |
| Block 47 physical acceptance | Open | Every physical matrix item PASS with evidence |
| Two-listener startup alignment | Known risk/open | Safe implementation + regression tests + real-device validation |
| Block 48.1 docs | Substantially implemented | Reconcile TODO against README |
| Block 48.2 ownership | Complete in TODO | No action unless contradiction is found |
| Block 48.3 final gates | Open | Fresh final release-state gate run |
| Ignored-test accountability | Implemented; final verification pending | Final audit/grep and TODO update |
| Exact referenced paths | Open | Repair stale TODO refs; include TODO in audit; pass |
| Final `memory.md` ledger | Open | Add final package/device/evidence results |
| CI integration of audit/APK artifact | Not committed | Optional workflow follow-up with proper permissions |

---

# 17. Suggested first prompt when work resumes

> Read `docs/SILENT_DISCO_BLOCK47_BLOCK48_HANDOFF_2026-08-13.md`, `memory.md`, `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`, and the desktop-host spec. Verify current master, then continue the Ralph Loop from the first unresolved item in the handoff. Do not monitor CI; I will report CI failures. Do not mark physical/device items done without physical evidence.

---

# 18. Bottom line

The project is close to Linux desktop-host release completion, but it is **not yet accepted**.

The highest-risk remaining technical issue is proving that two real Android listeners remain aligned to the same authoritative host timeline, including when they synchronize/start at different moments, without reintroducing the previous write-ahead-horizon regression.

After that is safe, the remaining path is evidence-heavy but straightforward:

1. clean up stale TODO references;
2. pass final automated gates;
3. complete Block 46.3;
4. run the complete Block 47 matrix with two physical Android devices;
5. record evidence and limitations;
6. rerun affected gates after any fix;
7. update the TODO and `memory.md` honestly;
8. close Block 48 only when the final release candidate has executable evidence and no known silent fallback.
