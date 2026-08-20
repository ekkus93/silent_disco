# Silent Disco Hardware Validation Handoff — 2026-08-19

## Purpose

This document is the handoff for **Claude Code or another engineer working directly with the real hardware** required to finish Silent Disco release acceptance.

The software-only remediation pass is complete enough that the next work should be **hardware/environment validation first**, not speculative feature development. Make production-code changes only when a real hardware run exposes a reproducible defect.

Repository:

```text
ekkus93/silent_disco
```

Authoritative branch:

```text
master
```

Pre-handoff implementation baseline:

```text
1b29e4ab7e4357e983f8162616f0a5b2753711a3
Merge nonphysical closure v8
```

The main CI run for that exact merge commit passed:

```text
https://github.com/ekkus93/silent_disco/actions/runs/32313684341
```

This handoff file is committed after that baseline, so `master` will have a newer SHA. **Always use the current `master` HEAD and build all test artifacts from the same exact commit.**

---

# 1. What is already done

The project is in release acceptance / evidence collection, not initial implementation.

Software work already completed includes:

- shared Rust authoritative domain/runtime ownership;
- Android Rust listener-playback migration;
- Android host self-monitor migration onto the Rust playback runtime;
- real discovered-session transport attachment inside Rust;
- Android mDNS/NSD discovery for the packaged desktop host;
- manual endpoint, mDNS, BLE/Wi-Fi Direct, and QR-related application paths;
- desktop Tauri host implementation and production Linux packaging;
- listener jitter buffering, synchronization, resynchronization, concealment, and diagnostics;
- true-current-time versus write-ahead-horizon startup alignment;
- active two-scheduler regression coverage for listeners locking synchronization at different times;
- bounded mid-stream synchronization slew;
- post-start render-ring underrun realignment;
- bounded synchronization acquisition with degraded-lock visibility;
- playback-pump liveness/failure accounting;
- desktop local-monitor failure propagation;
- deterministic Lab Mode and virtual fault infrastructure;
- Block 46.3 and Block 47 interactive acceptance runners;
- Block 48 exact-path / ignored-test audit infrastructure.

Do **not** reopen these areas merely because older handoff prose describes them as unresolved. The current non-physical closure checkpoint supersedes older status notes where they conflict.

---

# 2. Read these files first

Before running hardware acceptance or editing production code, read:

```text
CLAUDE.md
README.md
memory.md
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md
docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md
docs/SILENT_DISCO_PLAYBACK_REVIEW_FIXES_TODO.md
docs/SILENT_DISCO_NONPHYSICAL_CLOSURE_CHECKPOINT_2026-08-19.md
docs/AUDIO_PLAYBACK_STATE_2026-08-10.md
```

Historical context is also available in:

```text
docs/SILENT_DISCO_BLOCK47_BLOCK48_HANDOFF_2026-08-13.md
```

However, that older handoff predates the non-physical closure work. In particular, its warning that two-listener startup alignment still needed software implementation is historical. The current software implementation separates **true current time** from the pump's future **write/release horizon**, and the two-listener scheduler regression is active. What remains is real-device validation.

The authoritative desktop completion ledger is:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

Do not infer completion from this handoff alone.

---

# 3. Working rules

1. Work from current `master` unless the user explicitly asks for another branch.
2. Begin by recording:

   ```bash
   git status --short
   git branch --show-current
   git rev-parse HEAD
   ```

3. Build the desktop bundle and Android APK from the **same commit SHA**.
4. Never mark a hardware checkbox complete from source inspection, CI, emulator tests, or a previous artifact.
5. Never convert `FAIL`, `BLOCKED`, or `NOT RUN` into `PASS` to make a block close.
6. Do not weaken the Block 46.3 or Block 47 scripts to accommodate a failing product.
7. Preserve logs and evidence from failed runs before making a fix.
8. If hardware exposes a software defect:
   - reproduce it;
   - identify the responsible layer;
   - make the smallest coherent fix;
   - add automated regression coverage where practical;
   - rerun applicable software gates;
   - rebuild artifacts from the new SHA;
   - rerun the affected physical acceptance from a clean state.
9. Do not silently fall back to fake audio, virtual transport, temporary profiles, plaintext identities, or Lab adapters in production testing.
10. Do not claim Windows/macOS/iOS support from these Linux/Android runs.
11. Raw hardware evidence should remain outside the repository unless an authoritative TODO explicitly requests otherwise. Record concise results, artifact hashes, device details, measurements, and evidence locations in `memory.md` and the maintained TODO ledgers.
12. The user normally monitors GitHub Actions. Do not spend the hardware-validation loop polling CI unless the user asks you to investigate a specific run/failure.

---

# 4. Required hardware and environment

## 4.1 Desktop acceptance host

Required for release acceptance:

- graphical **Ubuntu 22.04 amd64** machine or VM;
- active graphical `DISPLAY`;
- active user D-Bus session;
- working Secret Service/keyring provider;
- functional desktop audio output for local-monitor testing;
- real LAN connectivity to the Android devices;
- ability to reversibly disrupt the desktop network interface;
- ability to make the desktop audio output temporarily unavailable for monitor-failure testing.

A newer Ubuntu machine can be useful for debugging, but it does not satisfy the accepted Block 46/47 baseline.

## 4.2 Android hardware

Block 46.3 needs at least:

- one physical Android listener.

Block 47 needs:

- **two distinct physical Android devices**;
- both visible through `adb`;
- both running the APK built from the same accepted commit as the desktop package.

The Block 47 runner explicitly rejects emulator-like devices.

Inventory first:

```bash
adb devices -l
```

Useful manual inventory commands when debugging:

```bash
adb -s <SERIAL> shell getprop ro.product.manufacturer
adb -s <SERIAL> shell getprop ro.product.model
adb -s <SERIAL> shell getprop ro.build.version.release
adb -s <SERIAL> shell getprop ro.build.version.sdk
adb -s <SERIAL> shell getprop ro.product.cpu.abi
adb -s <SERIAL> shell getprop ro.build.fingerprint
```

## 4.3 Network

Use a real LAN/Wi-Fi topology that allows desktop-to-phone traffic and mDNS discovery. Record:

- AP/router model if known;
- desktop connection type;
- each phone's Wi-Fi band if relevant;
- VLAN/guest-network isolation if present;
- VPN/firewall state;
- any client-isolation setting.

Do not fix discovery by disabling security globally and then forget to document the change. If a firewall/network policy blocks the product, preserve the failure and record the actual required rule or limitation.

---

# 5. Build release-candidate artifacts

From a clean/current checkout:

```bash
git switch master
git pull --ff-only
git status --short
git rev-parse HEAD
```

Record the SHA as the release-candidate SHA.

## 5.1 Android

Prerequisites are documented in `README.md`: JDK 17, Android SDK platform 36, build-tools 36.0.0, Android NDK 28.2.13676358, and `adb`.

Build:

```bash
./gradlew clean
./gradlew assembleDebug --stacktrace --console=plain
```

Expected APK:

```text
app/build/outputs/apk/debug/app-debug.apk
```

For a fuller pre-device build/gate pass:

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

## 5.2 Linux desktop

Prerequisites and Ubuntu packages are documented in `README.md`.

Build the ordinary production bundle, **not Lab Mode**:

```bash
cd desktop
npm ci
npm run tauri build
cd ..
```

Expected outputs:

```text
desktop/src-tauri/target/release/bundle/appimage/*.AppImage
desktop/src-tauri/target/release/bundle/deb/*.deb
```

Verify package structure/lifecycle before hardware acceptance when practical:

```bash
python3 desktop/scripts/verify-linux-bundles.py \
  --bundle-dir desktop/src-tauri/target/release/bundle \
  --tauri-config desktop/src-tauri/tauri.conf.json \
  --cargo-manifest desktop/src-tauri/Cargo.toml

bash desktop/scripts/smoke-linux-package-lifecycle.sh \
  desktop/src-tauri/target/release/bundle \
  com.ekkus.silentdisco.desktop \
  silent-disco-desktop
```

If using GitHub Actions artifacts, use the `desktop-linux-bundle` artifact produced from the exact release-candidate SHA. Do not mix an old desktop package with a newer APK.

---

# 6. First hardware milestone — Block 46.3 fresh-machine validation

This is the next acceptance block.

Runner:

```text
desktop/scripts/validate-block46-fresh-machine.sh
```

Invocation:

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle
```

Optional explicit evidence directory:

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle \
  "$HOME/silent-disco-block46-evidence-$(date -u +%Y%m%dT%H%M%SZ)"
```

The runner intentionally requires interaction. Do not automate away its physical/UI confirmations.

## 6.1 What it validates

The script fail-closes on the release baseline and verifies:

1. Ubuntu 22.04 amd64.
2. Graphical session and user D-Bus availability.
3. Silent Disco is not already installed.
4. Exactly one `.deb` and one AppImage are present.
5. SHA-256 values for package artifacts are recorded.
6. The `.deb` installs normally.
7. The packaged app launches without a development server.
8. A fresh production profile/database/source directory is created.
9. The generated 48 kHz stereo validation WAV can be selected through the real UI.
10. The staged source bytes exactly match the selected WAV by SHA-256.
11. A **physical Android listener** can join the desktop host and appear connected.
12. Diagnostics export succeeds to the required path and is valid bounded JSON.
13. Normal window close performs controlled shutdown.
14. Profile/database/source state survives shutdown.
15. Relaunch reaches Host setup without profile/bridge startup failure.
16. Package uninstall removes the application binary/package.
17. Uninstall **does not delete user profile data or the staged source**.
18. Structured evidence JSON is written.

## 6.2 Expected evidence

The evidence directory contains items including:

```text
environment.txt
first-launch.log
second-launch.log
block46-source.wav
block46-diagnostics.json
block46-fresh-machine-evidence.json
xdg-data/
xdg-config/
xdg-cache/
```

Preserve that directory even when the run fails.

## 6.3 Acceptance condition

Block 46.3 is complete only when the script exits successfully on the real Ubuntu 22.04 graphical baseline with the physical Android join explicitly confirmed.

After a pass:

- record release-candidate SHA;
- record `.deb` and AppImage SHA-256;
- record Android device/model/OS used for the join;
- record the evidence directory;
- update `memory.md`;
- reconcile only the Block 46.3 TODO checkboxes actually proven by the run.

---

# 7. Second hardware milestone — Block 47 Android interoperability acceptance

Do this after Block 46.3 passes using release-candidate artifacts from one exact commit.

Runner:

```text
desktop/scripts/validate-block47-android-interoperability.sh
```

Invocation:

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

Optional explicit evidence directory is the fifth argument.

The runner installs the `.deb` and APK, inventories both devices, hashes the `.deb`, AppImage, and APK, records topology, then requires explicit evidence for every matrix item.

## 7.1 Required matrix

Every one of these must be `PASS`:

1. **Manual endpoint join** — device A joins the packaged desktop using manual endpoint details.
2. **mDNS discovery** — device B discovers the packaged desktop normally and reaches the join flow.
3. **QR invitation** — a fresh QR invitation is scanned and completed; stale/expired data is not silently reused.
4. **Approval and rejection** — a real pending listener can be rejected, remains unauthorized, then can retry and be approved.
5. **One-listener audio** — real network audio is audible and continuous without a local-file fallback.
6. **Two-listener audio** — both phones hear the same authoritative stream and remain acceptably synchronized side-by-side.
7. **Pause / Resume / Stop / End** — both listeners reflect every transition truthfully and together.
8. **Android disconnect/reconnect** — disrupting one phone is visible, the other remains truthful, and the first can reconnect without restarting the desktop.
9. **Desktop interface disruption** — reversible LAN disruption is visible, false delivery is not claimed, and recovery/reconnection works.
10. **Host source failure** — making the selected staged source unavailable produces a visible source/decode failure with no silent substitution.
11. **Local monitor failure** — making desktop output unavailable produces a visible monitor failure while Android transmission follows the intended policy.
12. **Desktop restart** — packaged desktop closes and relaunches using the same real profile without startup/bridge fallback.
13. **Clean shutdown** — no crash/hang and no profile/database/source loss.
14. **Reopen profile/session history** — the same profile and expected history remain present; no empty replacement profile is silently created.
15. **Diagnostics export** — exported JSON is bounded, version-matched, contains both listeners, is not truncated, and contains synchronization confidence.
16. **Measured synchronization** — record actual offset/RTT/drift and/or an explicit audible/measurement delta.
17. **Known limitations** — record real limitations; use `none` only when genuinely none were observed.

Any `FAIL`, `BLOCKED`, or `NOT RUN` leaves Block 47 open and makes the runner exit non-zero.

## 7.2 Evidence produced

The runner records:

```text
commands.txt
steps.jsonl
device-a.txt
device-b.txt
network-topology.txt
block47-diagnostics.json
measured-synchronization.txt
known-limitations.txt
block47-android-interoperability-evidence.json
```

The final JSON also records release artifact hashes and result counts.

Preserve failed-run evidence too.

---

# 8. High-value physical playback checks that must not be skipped

The broad Block 47 matrix should cover these, but inspect the dedicated playback-review ledger as well:

```text
docs/SILENT_DISCO_PLAYBACK_REVIEW_FIXES_TODO.md
```

The remaining physical emphasis includes:

## 8.1 Discovered-session playback path

The discovered endpoint path was migrated and hardened in software but still requires real-device end-to-end proof. Current `master` supports endpoint-bearing mDNS discovery for the packaged desktop host, so use the **current production flow**, not older handoff wording that predates desktop mDNS support.

Verify:

- discovery finds the real packaged host;
- selection opens the real Rust listener transport;
- join succeeds;
- sync reaches a playable state;
- network audio is audible;
- diagnostics report truthful playback/sync state.

## 8.2 Two-phone synchronization

This is the core product acceptance property.

Software regression coverage now exists for listeners locking synchronization at different moments, but that does not substitute for two phones.

Run both devices side-by-side and record at minimum:

- time-to-first-audio for each listener;
- whether one device has a long silent start;
- whether a mid-stream hiccup occurs after sync acquisition;
- listener sync confidence;
- offset;
- RTT;
- drift estimate;
- audible or measured inter-listener delta;
- underrun/resynchronization diagnostics if any.

Do not call the result synchronized merely because both phones are playing.

## 8.3 Sync acquisition / startup regression

A historical device run exposed long startup silence and a later hiccup when sync acquisition was slow. Software now probes faster before lock and avoids feeding unschedulable packets into the scheduler while sync is unlocked.

Physically verify that:

- startup no longer has the previously observed multi-second dead-air behavior under ordinary LAN conditions;
- the stream does not later hiccup merely because pre-lock packets were stale;
- degraded-lock state, when used, is visible rather than reported as a perfect lock.

## 8.4 Audio route/device removal

The non-physical checkpoint still lists Android audio-device removal/route-change behavior as physical evidence.

Exercise realistic route changes available on the devices under test, for example:

- speaker ↔ wired output where supported;
- speaker ↔ Bluetooth output where appropriate;
- removal of an active output route.

Record whether playback continues, reopens, fails visibly, or requires reconnect. Do not hide route failure behind a healthy `Playing` state.

---

# 9. Failure triage protocol

When a hardware step fails:

## 9.1 First preserve facts

Before restarting everything, save:

- the acceptance evidence directory;
- desktop logs;
- diagnostics export if available;
- both device identities;
- exact master SHA;
- desktop and APK hashes;
- exact network topology;
- exact reproduction sequence;
- relevant `adb logcat` excerpts;
- whether the failure reproduces on one or both phones.

Useful capture pattern:

```bash
adb -s <SERIAL> logcat -c
# reproduce
adb -s <SERIAL> logcat -d > device-<SERIAL>-logcat.txt
```

## 9.2 Classify before editing

Determine whether the failure is primarily:

- packaging/install;
- profile/secure-store;
- discovery/mDNS;
- approval/join protocol;
- network transport;
- synchronization;
- packet scheduling/jitter buffer;
- Android audio backend/route;
- desktop local monitor;
- source/decode;
- UI truthfulness only;
- external LAN/firewall/environment.

Do not patch a lower layer merely because it is convenient.

## 9.3 After a software fix

Run applicable automated gates before rebuilding hardware artifacts.

Shared Rust:

```bash
bash scripts/check-rust.sh
```

Android:

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

Desktop frontend/bindings:

```bash
cd desktop
npm ci
npm run check
cd ..
```

Desktop Rust:

```bash
cd desktop/src-tauri
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --all-features
cd ../..
```

Then rebuild the `.deb`/AppImage/APK from the new commit and rerun the affected physical acceptance. Never continue using pre-fix artifacts.

---

# 10. Real performance / soak evidence after functional acceptance

The current non-physical checkpoint leaves real performance measurements open. Do these only after the hardware path is functionally correct.

Consult the Block 45 sections of:

```text
docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md
```

Collect the metrics actually requested there. Current open categories include real evidence for:

- CPU;
- RSS/memory;
- throughput;
- queue/backlog behavior;
- UI responsiveness;
- delivery behavior;
- synchronization;
- underruns;
- audio callback behavior;
- shutdown latency;
- database/persistence behavior.

Rules:

- preserve raw measurements outside the repo;
- record machine/device/topology versions;
- record workload duration and listener count;
- do not invent regression thresholds before the measured distribution supports them;
- do not derive a claimed listener limit from synthetic Lab Mode alone.

---

# 11. Final release-state gates and bookkeeping

After hardware acceptance and any resulting fixes, run the final release-state matrix required by Block 48.3.

At minimum reconcile current results for:

- shared Rust format;
- shared Rust strict Clippy;
- shared Rust tests;
- Android JVM tests;
- Android lint;
- Android instrumentation;
- desktop frontend format/lint/typecheck/tests/build;
- Tauri Rust format/strict Clippy/tests/check;
- Linux bundle build and lifecycle;
- deterministic Lab scenarios;
- loopback transport integration;
- physical Android acceptance.

Run the completion audit:

```bash
python3 scripts/audit-block48-completion.py
```

The non-physical closure pass had this audit green for exact referenced paths and ignored-test reason/owner accountability. Re-run it after any hardware-driven source/doc changes.

## 11.1 `memory.md` final evidence ledger

For each accepted hardware run record:

- date/time;
- current master SHA;
- desktop package version;
- `.deb` SHA-256;
- AppImage SHA-256;
- Android APK SHA-256;
- Android app version;
- both device manufacturer/model/Android/API/ABI;
- Ubuntu version/hardware or VM details;
- LAN topology;
- commands used;
- Block 46.3 result;
- every Block 47 matrix result;
- measured synchronization;
- diagnostics hash/path;
- performance/soak measurements when performed;
- known limitations;
- evidence directory location.

Only then update the authoritative TODO checkboxes.

---

# 12. Recommended execution order for Claude Code

Use this order unless a real failure forces a detour:

1. Pull current `master` and record SHA.
2. Read the authoritative files listed above.
3. Confirm Ubuntu 22.04 graphical/Secret-Service baseline.
4. Build fresh Android and production desktop artifacts from the same SHA.
5. Run package verification/lifecycle smoke.
6. Run **Block 46.3** fresh-machine acceptance with one physical Android listener.
7. Preserve evidence and update the ledger only if it passes.
8. Connect/inventory two physical Android devices.
9. Run **Block 47** complete interoperability matrix.
10. Give special attention to discovered-session playback, two-phone alignment, startup sync behavior, and audio-route changes.
11. If a physical defect appears, preserve evidence, fix minimally, run software gates, rebuild, and rerun the affected physical test.
12. After functional acceptance, collect real Block 45 performance/soak evidence still required by the TODO.
13. Run the final Block 48.3 release-state gates.
14. Run `python3 scripts/audit-block48-completion.py`.
15. Write the final `memory.md` evidence ledger.
16. Mark only evidence-backed TODO items complete.

---

# 13. Definition of done for the hardware phase

The hardware phase is done only when all of the following are true:

- Block 46.3 passes on graphical Ubuntu 22.04 amd64 using the packaged production desktop app and a physical Android listener.
- Block 47's runner reports every matrix item `PASS` with two physical Android devices.
- Two-listener synchronization is physically measured/observed and documented, not merely inferred from automated tests.
- Previously device-only startup/sync/audio-route concerns have been revalidated on current `master` artifacts.
- Any hardware-discovered software defects have automated regression coverage where practical and are fixed without weakening acceptance.
- Required real performance/soak evidence is collected honestly.
- Final release-state automated gates are green after the last source change.
- Block 48 completion audit passes.
- `memory.md` contains a final artifact/device/topology/result/measurement ledger.
- Authoritative TODO checkboxes reflect only evidence actually collected.

Until those conditions are met, leave the corresponding hardware/release checkboxes open.
