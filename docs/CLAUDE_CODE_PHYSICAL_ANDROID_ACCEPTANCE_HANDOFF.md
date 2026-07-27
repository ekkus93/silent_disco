# Silent Disco — Claude Code Physical Android Acceptance Handoff

**Created:** 2026-07-27  
**Repository:** `ekkus93/silent_disco`  
**Target branch:** `master`  
**Validated implementation commit:** `294fd72ad703cf9bbf2b5ffc25599985f72dfbee`  
**Validated GitHub Actions run:** `30304221562`  
**Documentation head before this handoff:** `40532ef8fdd80ec41853814ec5ac8bfb48fd7097`

## 1. Mission

Continue the project from its current state and complete honest physical-device acceptance of the Silent Disco Android application.

The software implementation is complete through P0, P1, and P2. The permanent automated validation matrix is green. Do not redesign or reimplement completed work merely because physical testing has started.

The remaining work is to:

1. install the current `master` build on physical Android devices;
2. execute the host, listener, resilience, and P2 physical acceptance scenarios;
3. record reproducible evidence for every result;
4. fix any real physical-device defect through a strict Ralph loop;
5. keep the full permanent CI matrix green after source changes;
6. update the completion documents only from observed evidence.

Do not declare overall device or release acceptance until the required physical checks pass.

---

## 2. Read these files first

These files already exist in the repository and are the authoritative project records:

- `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_TODO.md`
- `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_PROGRESS.md`
- this handoff document

The TODO is now a completion ledger. It records P0, P1, P2, cleanup, and automated validation as complete, while retaining the physical-device checks as open.

The progress document summarizes the implemented architecture and the successful automated validation run.

Also inspect the current permanent workflow under `.github/workflows/` before changing build or CI behavior. Do not add a temporary observer workflow, retry wrapper, or weakened validation path.

---

## 3. Current verified state

### 3.1 Software scope

The following scope is implemented:

- startup storage gate and fail-visible startup failures;
- contextual Android permissions;
- role-first Home screen;
- two-step host setup;
- Hosting Dashboard;
- Approve once, Always allow, and Reject;
- automatic listener discovery and continuous join progress;
- automatic navigation to playback only after playable readiness;
- persistent connection, synchronization, playback, storage, and permission failures;
- Connection Help and Advanced Diagnostics;
- expert tuning safeguards;
- Settings, approved listener-device management, and trusted-host management;
- support-report redaction;
- recent-session history and availability-checked rejoin;
- P-256 trusted-host identity backed by Android Keystore;
- Rust-owned versioned ES256 QR invitations, verification, expiry, and replay protection.

### 3.2 Architecture boundary

Do not violate these ownership rules:

- Rust owns domain data, persistence, synchronization, protocol rules, trusted-host records, QR validation, replay protection, and platform-independent behavior.
- Kotlin/Jetpack Compose owns presentation, Android navigation, permission UI, Android Keystore integration, camera integration, and other Android platform behavior.
- Do not add Kotlin-owned duplicate persistence to work around a Rust or JNI defect.
- Do not infer trusted identity from a display name.
- Do not advertise durable trust before the Rust persistence write commits.

### 3.3 Automated validation evidence

GitHub Actions run `30304221562` checked out implementation commit `294fd72ad703cf9bbf2b5ffc25599985f72dfbee` and passed:

- Rust formatting;
- Clippy with warnings denied;
- Rust workspace tests;
- Android debug, PoC-debug, release, and instrumentation-APK builds;
- native Rust library packaging for `armeabi-v7a`, `arm64-v8a`, `x86`, and `x86_64`;
- Android JVM tests;
- Android lint;
- the full API 29 Gradle-managed emulator instrumentation suite.

The two documentation commits after the validated implementation commit did not change application source.

---

## 4. Non-negotiable working rules

1. Work directly on `master` unless Phillip explicitly requests otherwise.
2. Begin by pulling the current remote state with a fast-forward-only update.
3. Do not rewrite, reset, or force-push shared history.
4. Do not mark a physical check complete from CI, an emulator, a unit test, a Compose test, or code inspection.
5. Use only `PASS`, `FAIL`, `BLOCKED`, or `NOT RUN` for physical results.
6. A blocked scenario is not a pass.
7. Do not add retries, swallowed exceptions, `|| true`, broad fallback behavior, weakened assertions, or silent failure paths.
8. Treat every first-party lint or compiler warning as a defect.
9. Do not expose internal IDs, keys, file paths, or raw diagnostics in consumer UI to make testing easier.
10. Do not modify persistent databases by hand to manufacture acceptance states.
11. Preserve fail-visible behavior. A failed operation must end in a truthful terminal or retryable state.
12. Keep raw logs, screenshots, APKs, and recordings out of Git unless Phillip explicitly asks to commit sanitized artifacts.
13. Every source fix must receive an automated regression test where a deterministic seam exists.
14. After a source fix, wait for and inspect the exact GitHub Actions run for the new head SHA before calling the fix validated.

---

## 5. Initial repository preparation

Run:

```bash
git checkout master
git pull --ff-only
git status --short
git log --oneline -10
```

Expected state:

- current `master` contains implementation commit `294fd72...`;
- current `master` also contains the completion-ledger documentation commits;
- this handoff file exists under `docs/`;
- the working tree is clean before testing begins.

Record the exact starting SHA in the physical-results document described below.

---

## 6. Device requirements

Final acceptance requires **at least two physical Android devices**. An emulator may assist diagnosis, but it cannot substitute for the second physical device.

For each device, record:

- manufacturer and model;
- Android version and API level;
- CPU architecture;
- app build SHA;
- Bluetooth state;
- Wi-Fi state;
- location-services state when required by that Android version;
- whether battery optimization is enabled for the app;
- whether the app was installed cleanly or upgraded in place.

At least one complete pass must use one device as host and another physical device as listener. Swap roles when practical to detect device-specific transport behavior.

Before starting:

- charge both devices;
- keep them unlocked and near one another for baseline tests;
- enable Developer Options and USB debugging where available;
- ensure each device has enough free storage;
- place a short, non-sensitive audio file on the intended host device;
- avoid using personal audio or screenshots containing private notifications.

---

## 7. Build and installation procedure

Build the current debug application from repository root:

```bash
./gradlew assembleDebug --stacktrace --console=plain
find app/build/outputs/apk -type f -name '*.apk' -print
```

Do not assume the output filename if Gradle reports a different path.

List physical devices:

```bash
adb devices -l
```

Install the same APK on each device using its serial:

```bash
adb -s <DEVICE_A_SERIAL> install -r <DEBUG_APK_PATH>
adb -s <DEVICE_B_SERIAL> install -r <DEBUG_APK_PATH>
```

If package identity is needed, derive it from the built APK or installed package rather than guessing:

```bash
aapt dump badging <DEBUG_APK_PATH> | head
adb -s <DEVICE_SERIAL> shell pm list packages | grep -i silent
```

Use a clean install only when a scenario explicitly requires first-run state. Record every app-data clear or uninstall because it can regenerate Android Keystore identity and erase Rust-owned history/trust data.

---

## 8. Evidence capture

Create a local, untracked evidence directory such as:

```bash
mkdir -p local-test-artifacts/$(date +%Y%m%d-%H%M%S)
```

Do not commit this directory.

Before reproducing a failure:

```bash
adb -s <DEVICE_SERIAL> logcat -c
```

Capture logs during the scenario:

```bash
adb -s <DEVICE_SERIAL> logcat -v threadtime > local-test-artifacts/<device>-logcat.txt
```

Stop log capture after the scenario. Also capture, when useful:

- screenshots or screen recordings;
- the app's sanitized support report;
- relevant Advanced Diagnostics values;
- exact timestamps;
- device role;
- steps to reproduce;
- expected result;
- observed result;
- whether the defect reproduces on the other device.

Do not paste secrets, active invite codes, full public keys, internal identifiers, personal paths, or unrelated logcat content into committed documentation.

---

## 9. Create the physical-results record

Create this new file before running the matrix:

- `docs/SILENT_DISCO_PHYSICAL_DEVICE_ACCEPTANCE_RESULTS.md` (**new file to create**)

Start it with:

```markdown
# Silent Disco Physical Device Acceptance Results

**Started:** YYYY-MM-DD
**Branch:** `master`
**Tested commit:** `<full SHA>`
**Tester:** Phillip / Claude Code assisted

## Devices

| Device | Model | Android/API | ABI | Install type | Notes |
|---|---|---|---|---|---|
| A |  |  |  | clean/upgrade |  |
| B |  |  |  | clean/upgrade |  |

## Result legend

- PASS: observed expected behavior on physical devices.
- FAIL: reproducible product defect or acceptance mismatch.
- BLOCKED: could not execute because a required capability or setup was unavailable.
- NOT RUN: not attempted yet.

## Results

| ID | Scenario | Status | Devices/roles | Evidence summary | Issue/fix commit |
|---|---|---|---|---|---|
```

Every scenario below must receive one row. Add detailed subsections for failures or unusual behavior.

---

## 10. Original physical acceptance matrix

The completion ledger currently retains 29 original physical checks. Execute them using the following IDs.

### 10.1 Host workflow

#### H-01 — Contextual permissions

- Begin from a clean permission state.
- Start the host workflow.
- Verify only host-relevant nearby permissions are requested at the point of need.
- Verify the rationale is understandable and denial remains visible.

#### H-02 — Audio-file selection

- Select a real local audio file through the Android document picker.
- Verify the chosen file remains selected through forward/back navigation.
- Verify cancellation does not create a fake selection.

#### H-03 — Manual approval

- Start a session using manual approval.
- Request access from the listener.
- Verify the host sees a pending request and can approve it.

#### H-04 — Invite-code approval

- Start a session requiring an invite code.
- Verify the displayed/shared code exactly matches the accepted listener code.
- Verify irrelevant code input is not shown in other modes.

#### H-05 — Approve once

- Approve the listener for the current session only.
- Verify the listener joins.
- Start a later session and verify the listener is not treated as durably approved.

#### H-06 — Always allow

- Choose Always allow.
- Verify persistence completes before durable trust is advertised.
- Start a later compatible session and verify the approved listener behavior is durable.
- Verify approved-device management reflects authoritative Rust state.

#### H-07 — Playback start, pause, and stop

- Start playback of real audio.
- Verify listener audio begins only after the host is actually playing.
- Pause and resume.
- Stop playback and verify both devices show truthful state.

#### H-08 — End-session confirmation

- While hosting, use Android back, app-bar back, and explicit End Session paths.
- Verify the safe action is initially preferred.
- Cancel once and confirm the session remains active.
- Confirm once and verify a clean Home back stack.

#### H-09 — Host Connection Help

- Open Connection Help during healthy hosting and during an induced problem when possible.
- Verify normal UI uses plain language and Advanced Diagnostics remains available separately.

### 10.2 Listener workflow

#### L-01 — Automatic discovery

- Enter Nearby Sessions with permissions granted.
- Verify discovery starts without a separate mandatory Scan action.
- Leave and re-enter; verify no duplicate or permanently stuck scan.

#### L-02 — Session selection

- Verify session and host names are understandable.
- Verify access requirements are visible before selection.
- Verify no raw internal IDs appear.

#### L-03 — Invite-code entry

- Join an invite-code session using the correct code.
- Verify incorrect input remains visible with an actionable error.

#### L-04 — Waiting-for-approval UI

- Request a manual join.
- Verify the listener sees a persistent waiting state until the host acts.
- Verify duplicate requests are prevented.

#### L-05 — Automatic playback navigation

- Approve and connect the listener.
- Verify the app opens Now Playing exactly once only when audio is playable.
- Verify there is no manual Continue to Playback step.

#### L-06 — Local volume

- Change listener volume while playing.
- Verify it affects local output without changing host playback state.

#### L-07 — Reconnect/problem UX

- Interrupt transport temporarily.
- Verify the UI does not claim stable playback while reconnecting.
- Verify recovery actions appear only when useful.

#### L-08 — Resynchronize audio

- Induce or simulate a valid desynchronization state when practical.
- Verify Resynchronize audio is available only when valid and produces a truthful result.

#### L-09 — Leave confirmation

- Use Android back, app-bar back, and explicit Leave Session.
- Cancel once and verify playback remains active.
- Confirm once and verify resources are released and Home has a clean stack.

#### L-10 — Listener Connection Help

- Verify healthy, reconnecting, desynchronized, and connection-lost presentations when reproducible.
- Verify raw metrics are confined to Advanced Diagnostics.

### 10.3 Resilience

#### R-01 — Permission denial and permanent denial

- Deny a required nearby permission.
- Deny again or select Don't ask again where supported.
- Verify persistent explanation and the system-Settings recovery path.

#### R-02 — Recoverable storage startup failure

- Use an existing supported test seam or a controlled, reversible setup.
- Do not corrupt production data manually without a documented safe procedure.
- Verify Retry exists only for a recoverable failure.

#### R-03 — Fatal storage startup failure

- Use the existing deterministic test seam or a controlled test build if physical reproduction is supported.
- Verify no fake Continue path exists.
- If physical reproduction is unsafe or unavailable, mark BLOCKED with the reason; do not mark PASS from emulator coverage.

#### R-04 — Host rejection

- Reject a pending listener.
- Verify the listener receives a persistent, understandable rejection and can return safely.

#### R-05 — Invalid invite code

- Submit an invalid code.
- Verify the failure remains visible and permits correction without duplicate join state.

#### R-06 — Host leaves during join

- End the host session while the listener is requesting, waiting, connecting, or syncing.
- Verify the listener reaches a truthful terminal state and does not navigate to playback.

#### R-07 — Listener moves out of range

- Increase distance or otherwise interrupt the physical link.
- Verify connection-loss/recovery behavior is visible and no stale Playing state remains.

#### R-08 — Host/listener reconnect

- Restore range or radio availability.
- Verify recovery does not duplicate navigation, listeners, or playback commands.

#### R-09 — Playback engine failure

- Reproduce only through a safe, deterministic input or known test seam.
- Verify the app never claims Playing if output failed.
- Mark BLOCKED if no safe physical trigger exists.

#### R-10 — Process recreation and configuration change

- Rotate devices during setup and active workflows.
- Background/foreground the app.
- Where safe, use Developer Options process recreation.
- Verify no duplicate effect, join, approval, end, leave, or playback command occurs.

---

## 11. Required P2 physical extension

The current 29-item physical matrix originated before P2 was promoted into required scope. Automated P2 coverage is green, but final device acceptance must also include the following physical checks.

Add these items to the TODO's physical section before declaring overall acceptance complete.

### 11.1 Recent sessions and rejoin

#### P2R-01 — Recent-session history appears truthfully

- Complete or leave a listener session.
- Return to Home.
- Verify the recent-session card records history without claiming the host is currently available.

#### P2R-02 — Offline recent session remains unavailable

- End the host session.
- Tap Check availability on the listener.
- Verify a fresh scan occurs and the app reports that the session is not currently nearby.

#### P2R-03 — Live exact-session rejoin

- Leave the listener while the host keeps the same session active.
- From the recent-session card, tap Check availability.
- Verify only a newly observed exact session ID authorizes navigation/rejoin.

### 11.2 Trusted hosts

#### P2T-01 — Trust host through a verified signed invitation

- Scan a valid host invitation.
- Choose Trust host.
- Verify the listener can join only after live discovery confirms the signed session.
- Verify the trusted host appears in Settings from Rust-owned persistence.

#### P2T-02 — Trusted grouping requires the exact verified association

- Verify the signed session appears under Trusted hosts.
- Verify another nearby session with the same visible host name is not trusted solely by name.

#### P2T-03 — Trusted identity survives normal app restart

- Force-stop and reopen both apps without clearing data.
- Verify the host identity and listener trusted-host record persist.

#### P2T-04 — Deleting trusted host removes grouping

- Delete the trusted host from Settings.
- Verify the authoritative list refreshes after committed deletion.
- Verify the nearby session no longer appears under Trusted hosts.

#### P2T-05 — Identity change is not silently trusted

- Using a controlled procedure, clear/reinstall the host app so Android Keystore identity changes, then reuse the same visible name.
- Verify the listener does not treat the new key as the previously trusted host.
- Record the destructive setup because it erases host-side data.

### 11.3 QR joining

#### P2Q-01 — Join once does not persist trust

- Scan a valid QR invitation.
- Choose Join once.
- Verify the session can be joined after live discovery.
- Verify the host is not added to persistent trusted-host management.

#### P2Q-02 — Replay is rejected

- After a successful invitation consumption, scan the exact same QR payload again on the same listener.
- Verify replay is rejected visibly and no join starts.

#### P2Q-03 — Expired invitation is rejected

- Retain a generated invitation beyond its expiry and scan it.
- Verify Rust validation rejects it visibly.
- Do not change device clocks in a way that invalidates unrelated tests without recording the procedure.

#### P2Q-04 — Signed QR never bypasses live discovery

- Scan a valid invitation while the exact host session is not advertising or is no longer active.
- Verify the app does not navigate into a fake connection and explains that the session is not currently available.

#### P2Q-05 — Camera denial and recovery

- Deny camera access once and verify rationale/retry behavior.
- Permanently deny it where supported and verify the system-Settings recovery path.
- Grant access and verify scanning can proceed.

---

## 12. Ralph loop for any physical failure

For every `FAIL` result:

1. **Reproduce** the failure at least twice when practical.
2. **Minimize** the scenario and determine whether it is device-specific, Android-version-specific, transport-specific, or general.
3. **Capture evidence** from both devices, including timestamps and app state.
4. **Inspect authoritative state** rather than relying only on what the UI displays.
5. **Identify the root cause**. Do not patch the test procedure to hide a product defect.
6. **Implement the narrowest correct fix** while preserving Rust/Kotlin ownership.
7. **Add a deterministic regression test** at the lowest appropriate layer:
   - Rust unit/property test for protocol, persistence, identity, expiry, or replay;
   - Kotlin unit test for presentation mapping or orchestration;
   - Compose/instrumentation test for navigation or visible state;
   - physical regression scenario when hardware behavior cannot be emulated.
8. **Run relevant local gates**.
9. **Commit intentionally to `master`** with a focused message.
10. **Inspect the exact GitHub Actions run and head SHA**. Do not assume green status.
11. **Retest the failed physical scenario** on the original device pair.
12. **Retest adjacent scenarios** likely affected by the fix.
13. **Update the results document and TODO only after observation**.

Do not use retries, sleeps without a state-based reason, suppressed assertions, silent fallback, or success-state fabrication to make a physical test pass.

---

## 13. Validation after source changes

Run the relevant local commands first.

Rust:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cd ..
```

Android build, JVM tests, and lint:

```bash
./gradlew \
  assembleDebug \
  assemblePocDebug \
  assembleRelease \
  assembleDebugAndroidTest \
  test \
  lintDebug \
  --stacktrace \
  --console=plain
```

Permanent managed-device instrumentation command:

```bash
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace \
  --console=plain
```

The permanent CI workflow is the final automated source of truth. Record the exact successful run ID and tested commit SHA after every code-changing fix series.

Documentation-only result recording does not prove or invalidate application behavior, but the working tree must still be clean and the recorded tested SHA must remain explicit.

---

## 14. Documentation update rules

During physical testing, update:

1. `docs/SILENT_DISCO_PHYSICAL_DEVICE_ACCEPTANCE_RESULTS.md` with detailed evidence;
2. `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_TODO.md` checkboxes only after a physical PASS;
3. `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_PROGRESS.md` with counts, tested devices, failures, fixes, and remaining work.

Do not erase the distinction between:

- software implementation complete;
- automated validation complete;
- physical-device acceptance pending or complete.

If a regression reopens completed implementation work, add a clearly labeled section such as:

```markdown
## Regressions discovered during physical acceptance

- FAIL/PASS history
- root cause
- fix commit
- CI run
- physical retest evidence
```

Do not silently change an already completed item back and forth without preserving the failure and fix record.

---

## 15. Completion criteria

Physical acceptance is complete only when:

- at least two physical Android devices have been used;
- all executable original H/L/R scenarios pass;
- any scenario that remains BLOCKED is explicitly reviewed by Phillip and is not represented as PASS;
- all P2 physical-extension scenarios pass or receive an explicit scope decision from Phillip;
- every discovered product defect has a root-cause fix and regression coverage where possible;
- the full permanent CI matrix passes at the final source commit;
- the final source commit is retested physically after the last relevant source change;
- the TODO and progress documents match the evidence record;
- no raw logs, screenshots, APKs, temporary workflows, or local artifacts were accidentally committed;
- `git status --short` is clean;
- remote `master` points to the intended final commit.

Final reporting must identify:

- device pair and Android versions;
- tested source SHA;
- final green GitHub Actions run ID;
- number of PASS, FAIL, BLOCKED, and NOT RUN scenarios;
- every fix commit produced during physical acceptance;
- any accepted limitation or intentionally deferred scenario.

---

## 16. Immediate next action

1. Pull current `master`.
2. Read the TODO and progress documents.
3. Create `docs/SILENT_DISCO_PHYSICAL_DEVICE_ACCEPTANCE_RESULTS.md` from the template above.
4. Inventory the available physical Android devices.
5. Build and install the same debug APK on both devices.
6. Start with H-01 through H-04 and L-01 through L-04 to establish a stable host/listener connection baseline.
7. Continue through playback, resilience, and P2 checks.
8. Ralph-loop every genuine failure until the physical matrix and permanent CI are both green.
