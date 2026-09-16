# Claude Code Handoff — Block 46.3 Fresh-Machine Closeout — 2026-09-16

**Repository:** `ekkus93/silent_disco`  
**Primary TODO:** `docs/BLOCK_46_3_FRESH_MACHINE_CLOSEOUT_TODO_2026-09-15.md`  
**Parent TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`  
**Validator:** `desktop/scripts/validate-block46-fresh-machine.sh`  
**Staging workflow:** `.github/workflows/block46-self-hosted-stage.yml`  
**Current master at handoff:** `b46c727de70a5efac9f03e0293449dca34d97016`  
**Continuation host requested by user:** current Ubuntu laptop (`kawa`) with the Samsung Galaxy A54 physically connected over ADB  

## Purpose of this handoff

Continue and close Block 46.3 from the current `master` state without returning to
the home self-hosted runner for the physical-device/manual portion. The user wants
Claude Code to continue on the laptop they are currently using, with the Samsung
Galaxy A54 connected directly to that laptop.

Do not infer or fabricate any graphical/manual acceptance result. The remaining
work is primarily exact-SHA local staging, physical A54 verification, manual GUI
validation, evidence preservation, and documentation reconciliation.

## Current repository state

At handoff, remote `master` is:

```text
b46c727de70a5efac9f03e0293449dca34d97016
```

This commit is:

```text
ci: skip sudo when Block 46 prerequisites are installed (#58)
```

PR #58 changed the Block 46.3 staging workflow so that an already-provisioned
machine does not require passwordless `sudo`. It first verifies the required
Ubuntu desktop build packages and only attempts `sudo apt-get` if packages are
actually missing.

The user has now manually installed the requested Ubuntu build prerequisites on
`kawa` and accepted/installed the required Android SDK licenses/components. The
Android SDK used there is under `~/Android/Sdk`.

Required Android SDK components are:

```text
platforms;android-36
build-tools;36.0.0
ndk;28.2.13676358
```

The user also ran the SDK license acceptance successfully.

## Exact-head CI evidence already obtained

For the current `master` SHA `b46c727de70a5efac9f03e0293449dca34d97016`:

- GitHub Actions `CI` run `35068123366` completed successfully.
- GitHub Actions `Source file line limit` run `35068123478` completed successfully.

The successful CI includes the normal Rust quality gates, Android build/unit/lint,
and Android instrumentation coverage used by this repository.

The PR #58 implementation head was also qualified before merge; do not redo old
PR work merely because the original closeout TODO still contains stale references
to PR #52. Treat the TODO as a checklist that now needs reconciliation to the
actual history.

## Last self-hosted staging attempt

The most recent Block 46.3 staging run against exact current `master` was:

```text
workflow: Stage Block 46.3 release candidate
run id:   35068123405
job id:   104703010933
head SHA: b46c727de70a5efac9f03e0293449dca34d97016
result:   failure
```

This run proved the following successfully before the failure:

- exact dispatched revision checkout;
- supported Ubuntu baseline check;
- Node/Java/Gradle/Rust setup;
- Android SDK root resolution;
- desktop/Android prerequisite verification;
- production Linux Tauri bundle build;
- Linux bundle payload verification;
- Android debug APK build from the same exact checkout.

The Android Gradle build reported `BUILD SUCCESSFUL`.

The only failing gate was:

```text
Confirm a physical Android device is still reachable
```

The exact failure was:

```text
No authorized online Android device is visible to adb.
```

That failure is expected because no phone was connected to the home self-hosted
runner at that time. Do not treat it as a product/build defect.

Because the ADB gate failed, the workflow step that creates
`$HOME/silent-disco-block46-stage/<SHA>/` was skipped on that machine.

## Artifact preserved from the failed staging run

Even though the ADB gate failed, the workflow uploaded the successfully built
artifacts from exact SHA `b46c727de70a5efac9f03e0293449dca34d97016`.

Artifact metadata:

```text
run id:       35068123405
artifact id:  10435246993
artifact name: block46-self-hosted-stage-b46c727de70a5efac9f03e0293449dca34d97016
size:         238990976 bytes
expires:      2026-09-23T07:21:36Z
artifact zip SHA256 reported by upload step:
24e27acc5b1197a8910801d4f63ea74eef06b355b7c72044ec3b8e29aef3cfb1
```

The artifact contains the build log plus the built Android APK and Linux bundle
outputs. It is useful as historical exact-SHA evidence, but the preferred path
for the laptop handoff is to perform a fresh exact-SHA local build/stage on the
laptop with the A54 attached so that all physical/manual evidence is produced on
the machine actually being used for acceptance.

## Important change in execution venue

The original closeout TODO assumes the physical acceptance flow is executed on
the configured self-hosted Ubuntu runner. The user is intentionally changing the
execution venue: Claude Code should continue on the current laptop with the A54
attached rather than depend on the home runner.

Preserve the acceptance semantics when doing this:

1. The laptop must itself satisfy the Block 46.3 supported host baseline before
   its results are used as acceptance evidence: Ubuntu 22.04 or 24.04, amd64.
2. Desktop and Android artifacts used for manual validation must come from the
   same exact `master` SHA.
3. Record that SHA everywhere in the evidence.
4. Do not mix an APK from one SHA with desktop bundles from another SHA.
5. Do not claim the old self-hosted staging run completed staging; it did not.
6. Update the closeout TODO/handoff documentation to explain that final physical
   acceptance was performed locally on the laptop rather than on the home
   self-hosted runner.
7. Do not fabricate GUI observations or pipe synthetic answers into the manual
   validator.

If the laptop does not report a supported Ubuntu/amd64 baseline, stop and resolve
that discrepancy before claiming Block 46.3 acceptance.

## Recommended continuation sequence on the laptop

### 1. Synchronize to exact current master

Start by making sure the working copy has no unintended local changes and is at
current remote `master`.

```bash
git fetch origin
git checkout master
git pull --ff-only origin master
git rev-parse HEAD
```

Expected handoff SHA before any new repository work:

```text
b46c727de70a5efac9f03e0293449dca34d97016
```

If `master` has advanced, use the new exact `master` SHA consistently for every
artifact and all acceptance evidence; do not fall back to the old handoff SHA.

### 2. Verify the laptop baseline

Record the exact output of:

```bash
cat /etc/os-release
dpkg --print-architecture
uname -a
```

Acceptance requires Ubuntu 22.04 or 24.04 and `amd64`.

Verify required desktop packages are installed. The packages previously requested
and installed by the user are:

```text
build-essential
curl
dbus-x11
file
libasound2-dev
libayatana-appindicator3-dev
librsvg2-dev
libssl-dev
libwebkit2gtk-4.1-dev
libxdo-dev
patchelf
wget
xauth
xvfb
```

A useful verification command is:

```bash
dpkg-query -W -f='${binary:Package}\t${Status}\n' \
  build-essential curl dbus-x11 file libasound2-dev \
  libayatana-appindicator3-dev librsvg2-dev libssl-dev \
  libwebkit2gtk-4.1-dev libxdo-dev patchelf wget xauth xvfb
```

### 3. Verify Android SDK and A54 connectivity

Confirm the SDK and packages:

```bash
export ANDROID_SDK_ROOT="$HOME/Android/Sdk"
export ANDROID_HOME="$ANDROID_SDK_ROOT"
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --list_installed
```

Connect the Samsung Galaxy A54 to the laptop, enable USB debugging, and authorize
this laptop on the device.

Then run:

```bash
adb start-server
adb devices -l
```

The A54 must appear with state `device`, not `unauthorized` or `offline`.

Record physical device identity and Android/API details, for example:

```bash
SERIAL="<A54 serial from adb devices>"
adb -s "$SERIAL" shell getprop ro.product.manufacturer
adb -s "$SERIAL" shell getprop ro.product.model
adb -s "$SERIAL" shell getprop ro.product.name
adb -s "$SERIAL" shell getprop ro.product.device
adb -s "$SERIAL" shell getprop ro.build.version.release
adb -s "$SERIAL" shell getprop ro.build.version.sdk
adb -s "$SERIAL" shell getprop ro.build.fingerprint
```

### 4. Build both release candidates from the same exact SHA

Capture the source SHA first:

```bash
MASTER_SHA="$(git rev-parse HEAD)"
printf '%s\n' "$MASTER_SHA"
```

Build the Linux Tauri bundles using the repository's existing production path:

```bash
cd desktop
npm ci
npm run tauri build
cd ..
```

If the known transient `Peer disconnected` failure occurs, follow the retry
behavior encoded in `.github/workflows/block46-self-hosted-stage.yml` rather than
changing product behavior or skipping the build.

Verify the generated Linux bundles with the existing repository verifier:

```bash
python3 desktop/scripts/verify-linux-bundles.py \
  --bundle-dir desktop/src-tauri/target/release/bundle \
  --tauri-config desktop/src-tauri/tauri.conf.json \
  --cargo-manifest desktop/src-tauri/Cargo.toml
```

Build the Android APK from the same checkout:

```bash
./gradlew assembleDebug --stacktrace --console=plain
test -f app/build/outputs/apk/debug/app-debug.apk
```

### 5. Create an exact-SHA local stage on the laptop

Use the same layout expected by the staging workflow:

```bash
MASTER_SHA="$(git rev-parse HEAD)"
STAGE_ROOT="$HOME/silent-disco-block46-stage"
STAGE_DIR="$STAGE_ROOT/$MASTER_SHA"

rm -rf "$STAGE_DIR"
mkdir -p \
  "$STAGE_DIR/desktop-bundle/deb" \
  "$STAGE_DIR/desktop-bundle/appimage" \
  "$STAGE_DIR/android"

cp desktop/src-tauri/target/release/bundle/deb/*.deb \
  "$STAGE_DIR/desktop-bundle/deb/"
cp desktop/src-tauri/target/release/bundle/appimage/*.AppImage \
  "$STAGE_DIR/desktop-bundle/appimage/"
cp app/build/outputs/apk/debug/app-debug.apk \
  "$STAGE_DIR/android/app-debug.apk"
cp desktop/scripts/validate-block46-fresh-machine.sh \
  "$STAGE_DIR/validate-block46-fresh-machine.sh"
chmod 0755 "$STAGE_DIR/validate-block46-fresh-machine.sh"
```

Create a local `STAGE_MANIFEST.txt` that records at minimum:

```text
source_sha=<exact git SHA>
host_os=<Ubuntu ID/version>
host_architecture=amd64
staged_at_utc=<timestamp>
execution_venue=local-laptop
physical_listener=Samsung Galaxy A54
```

Generate hashes from inside the stage directory:

```bash
cd "$STAGE_DIR"
sha256sum \
  desktop-bundle/deb/*.deb \
  desktop-bundle/appimage/*.AppImage \
  android/app-debug.apk \
  validate-block46-fresh-machine.sh > SHA256SUMS
sha256sum -c SHA256SUMS
```

Preserve the successful hash-verification output.

### 6. Install the exact staged Android APK

While still using the same stage:

```bash
adb devices -l
adb install -r android/app-debug.apk
```

Record installation output and confirm the installed app is the exact staged
candidate rather than an older APK.

### 7. Run the real graphical fresh-machine validator

From a normal graphical terminal on the laptop:

```bash
cd "$STAGE_DIR"
bash ./validate-block46-fresh-machine.sh ./desktop-bundle
```

Do not pipe answers into the validator. Each manual prompt must be answered only
after the requested behavior has actually been observed on the packaged desktop
application and the physical A54.

The required manual acceptance includes, as applicable to the validator prompts:

- package installation;
- launch without a development server;
- fresh profile creation;
- staging/selecting a real supported audio source;
- starting a real desktop host session;
- using the Samsung Galaxy A54 as the physical Android listener;
- join/approval/session behavior;
- required playback and GUI behavior;
- diagnostics export;
- clean shutdown;
- packaged app/profile reopen;
- persistence behavior;
- package uninstall behavior;
- documented profile-local user-data preservation behavior.

Do not mark any item passed unless it was observed.

## Evidence that still must be collected

The existing CI/staging evidence is useful but does not close Block 46.3. The
following final evidence is still required from the laptop/A54 run:

- exact final `master` SHA used for the local build and manual validation;
- Ubuntu distribution/version and `amd64` architecture of the laptop;
- exact desktop `.deb` hash;
- exact AppImage hash;
- exact Android APK hash;
- successful `sha256sum -c SHA256SUMS` output;
- A54 serial/model/product/device identity;
- Android version and API level;
- successful exact staged APK installation output;
- validator `environment.txt`;
- validator evidence JSON;
- packaged-app launch logs;
- exported diagnostics;
- exact commands used;
- explicit pass/fail outcome for every manual validator prompt;
- any screenshots or other direct observations needed to support the manual
  acceptance record.

Keep every piece of evidence tied to the exact same source SHA.

## Documentation that must be reconciled after a successful validator run

After the real laptop/A54 validator succeeds, update at least:

1. `docs/BLOCK_46_3_FRESH_MACHINE_CLOSEOUT_TODO_2026-09-15.md`
   - replace stale PR #52/current-run assumptions with the actual final history;
   - record current/final `master` SHA;
   - record PR #58 and any later PRs needed for closeout;
   - record CI run IDs;
   - record the failed self-hosted staging run `35068123405` accurately as a
     build-success/ADB-absent run, not as acceptance success;
   - record the local laptop/A54 staging and manual evidence;
   - check only items actually satisfied.

2. `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`
   - close Block 46.3 only after the physical/manual validator has genuinely
     passed and the evidence is preserved.

3. `docs/SILENT_DISCO_RALPH_HANDOFF_2026-09-14.md`
   - update the project handoff to the completed Block 46.3 state and identify
     the exact next task (likely Block 47 work, subject to the parent TODO).

4. This handoff file, if useful, with the final acceptance outcome and evidence
   paths so that its status is not left ambiguous.

Then run the applicable documentation/source-size/normal CI gates required by
repository policy, commit the reconciliation through the protected path, and
merge it to `master`.

## What is already solved and should not be reopened without evidence

The following issues were already investigated and remediated in the history
leading to current `master`:

- Android hosted-runner instrumentation path was moved to a snapshot-free API-29
  emulator path and qualified.
- Hosted-runner disk reclamation was added for the emulator path.
- Rust all-features test contention was bounded without skipping the test suite.
- Android SDK setup in the Block 46.3 staging workflow was bounded to avoid the
  previous 90-minute silent prerequisite hang.
- The workflow now fails fast when noninteractive `sudo` is required but missing.
- PR #58 made `sudo` optional when all required desktop packages are already
  installed.
- Current exact `master` builds both the production Linux bundles and Android APK
  successfully on the previous Ubuntu 24.04 staging host.

Do not regress these fixes merely to change the execution venue.

## Historical staging failures that explain the current workflow shape

For context when reading git history:

- An earlier self-hosted staging attempt stalled for the full 90-minute job
  timeout inside prerequisite setup.
- Fail-fast instrumentation then exposed that the self-hosted runner account did
  not have passwordless noninteractive `sudo`.
- The user manually installed the prerequisite packages and Android SDK licenses.
- PR #58 changed staging to skip `sudo` when prerequisites are already present.
- The next exact-`master` staging run (`35068123405`) passed all build/setup steps
  and failed only because no physical Android phone was connected to that home
  runner.

This is the point from which Claude Code should continue.

## Definition of done for this handoff

Do not stop at "the build works". Block 46.3 is complete only when all of the
following are true:

1. A supported Ubuntu amd64 laptop is used for the final acceptance run.
2. Desktop and Android artifacts are built/staged from one exact `master` SHA.
3. The Samsung Galaxy A54 is physically connected and authorized over ADB.
4. Exact artifact hashes and host/device metadata are preserved.
5. The staged APK is installed on the A54.
6. `validate-block46-fresh-machine.sh` is run interactively in the real graphical
   desktop session.
7. Every required GUI/physical-device prompt is genuinely observed and passes.
8. Diagnostics/evidence are preserved.
9. The Block 46.3 TODO, parent TODO, and current handoff documentation are
   reconciled to the real evidence.
10. The final documentation changes pass repository gates and are merged to
    `master`.

Do not claim Block 46.3 complete before those conditions are satisfied.
