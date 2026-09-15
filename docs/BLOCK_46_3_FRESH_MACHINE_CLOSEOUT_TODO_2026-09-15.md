# Block 46.3 Fresh-Machine Closeout TODO — 2026-09-15

**Repository:** `ekkus93/silent_disco`  
**Parent TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`  
**Validator:** `desktop/scripts/validate-block46-fresh-machine.sh`  
**Staging workflow:** `.github/workflows/block46-self-hosted-stage.yml`  
**Current PR:** #52 (`ralph/block46-ubuntu24-baseline`)  
**Current physical topology:** self-hosted Ubuntu 24.04 amd64 runner + Samsung Galaxy A54 over ADB

This file is the detailed execution/evidence checklist for Block 46.3. It does
not replace the parent TODO. Block 46.3 remains open until the real graphical
desktop and physical-device validation is complete and the resulting evidence
is reconciled back into the project documentation.

---

## 0. Non-negotiable acceptance rules

- [ ] Use exact-head evidence only; do not combine artifacts or CI results from
      different source SHAs.
- [ ] The desktop package and Android APK used for the physical validation must
      be built/staged from the same exact post-merge `master` SHA.
- [ ] Ubuntu 22.04 amd64 and Ubuntu 24.04 amd64 are both supported Block 46.3
      acceptance baselines. A Ubuntu 24.04-specific failure is a bug to fix,
      not a reason to narrow support back to 22.04.
- [ ] The self-hosted Ubuntu 24.04 amd64 runner is the current fresh-machine
      validation host.
- [ ] The Samsung Galaxy A54 is the current physical Android listener for this
      block and must be connected/authorized over ADB when the validator runs.
- [ ] One physical Android device is sufficient for Block 46.3. Block 47's
      two-listener cases still require two distinct physical Android devices.
- [ ] The staging workflow may build, verify, hash, and stage artifacts, but it
      must not fabricate GUI observations or pipe/synthesize `PASS` into the
      manual validator.
- [ ] Manual prompts must be answered only after the user has actually observed
      the requested GUI/physical-device behavior.
- [ ] Build success, CI success, and staging success alone do not complete
      Block 46.3.

---

## 1. PR #52 exact-head qualification

Implementation head `09bf8b0280b354cd4c9bdc13e752095065adfe9c` was
qualified before this documentation reconciliation with all three exact-head
workflow groups green:

- CI run `34942355662` — success, including Rust quality gates, Android
  build/unit/lint, hosted-runner disk reclamation, snapshot-free API-29
  emulator boot, and `connectedDebugAndroidTest`;
- Source file line limit run `34942355770` — success;
- Desktop CI run `34942355768` — success.

The documentation commit that updates this checklist creates a newer PR head,
so the merge gate must use fresh CI for that final exact SHA.

- [x] Replace the unstable managed-device snapshot path with a snapshot-free
      API-29 emulator path.
- [x] Reclaim unused hosted-runner Android SDK disk before emulator creation.
- [x] Preserve the full Android instrumentation gate; do not skip or fake it.
- [x] Serialize the complete Rust all-features test suite with
      `--test-threads=1` to eliminate the documented scheduler-contention hang
      without skipping any tests.
- [x] Obtain a fully green exact-head implementation run at `09bf8b02...`.
- [ ] Obtain fully green CI for the final documentation-adjusted PR head.
- [ ] Confirm PR #52 is still based on the intended `master` and contains only
      the reviewed Block 46.3/CI/documentation changes.
- [ ] Merge PR #52 through the normal protected merge path.
- [ ] Record the exact post-merge `master` SHA.

---

## 2. Self-hosted exact-SHA staging

After PR #52 is merged, `.github/workflows/block46-self-hosted-stage.yml` must
run against the exact post-merge `master` SHA on the self-hosted Ubuntu 24.04
amd64 runner. The first merge that introduces/changes this workflow is expected
to trigger it automatically because the workflow file itself changed.

- [ ] Confirm the staging workflow run's `head_sha` exactly equals the
      post-merge `master` SHA.
- [ ] Confirm the runner reports Ubuntu 24.04 amd64.
- [ ] Confirm checkout `HEAD` equals `GITHUB_SHA`.
- [ ] Build the Linux Tauri release candidate.
- [ ] Build the Android debug APK from the same checkout.
- [ ] Confirm at least one physical Android device is visible over ADB.
- [ ] Stage the exact release candidate under
      `$HOME/silent-disco-block46-stage/<MASTER_SHA>/`.
- [ ] Confirm the stage contains at least:
  - `desktop-bundle/deb/*.deb`;
  - `desktop-bundle/appimage/*.AppImage`;
  - `android/app-debug.apk`;
  - `validate-block46-fresh-machine.sh`;
  - `STAGE_MANIFEST.txt`;
  - `SHA256SUMS`;
  - `RUN_BLOCK46.txt`.
- [ ] Preserve the staging workflow run ID/job ID and any uploaded staging
      artifact metadata.
- [ ] Treat any Ubuntu 24.04 staging failure as a defect to diagnose/fix and
      requalify; do not switch the acceptance baseline to 22.04 to avoid it.

---

## 3. Pre-manual integrity and A54 checks

From a normal graphical terminal on the same self-hosted runner, use the exact
stage directory generated above.

- [ ] `cd $HOME/silent-disco-block46-stage/<MASTER_SHA>`
- [ ] `cat STAGE_MANIFEST.txt` and verify the recorded source SHA.
- [ ] `sha256sum -c SHA256SUMS` passes for every staged artifact.
- [ ] `adb devices -l` shows the physical Samsung Galaxy A54 as an authorized
      device, not `offline` or `unauthorized`.
- [ ] Record the A54 model/product/device identity and Android/API version.
- [ ] Install the exact staged APK with
      `adb install -r android/app-debug.apk`.
- [ ] Confirm the installed Android app corresponds to the staged release
      candidate; do not substitute an older local APK.

---

## 4. Manual graphical fresh-machine validator

Run:

```bash
bash ./validate-block46-fresh-machine.sh ./desktop-bundle
```

Do not pipe answers into the script. Complete the prompted checks against the
real desktop window and the physical A54.

- [ ] Validator confirms supported Ubuntu version and amd64 host/package
      architecture.
- [ ] Install the staged desktop package successfully.
- [ ] Launch the packaged desktop application without a development server.
- [ ] Create a fresh profile.
- [ ] Select/stage a real supported audio source.
- [ ] Start a real desktop host session.
- [ ] Use the Samsung Galaxy A54 as the physical Android listener.
- [ ] Confirm the required join/approval/session behavior requested by the
      validator.
- [ ] Confirm required playback/GUI behavior requested by the validator.
- [ ] Export diagnostics successfully.
- [ ] Shut the application down cleanly.
- [ ] Reopen the packaged application/profile successfully.
- [ ] Verify the required persistence/reopen behavior.
- [ ] Exercise package uninstall behavior.
- [ ] Verify the documented profile-local user-data preservation policy.
- [ ] Do not claim a step passed if it was not actually observed.

---

## 5. Evidence preservation

Preserve exact, auditable evidence for the successful run:

- [ ] post-merge `master` SHA;
- [ ] PR #52 merge identity;
- [ ] exact-head PR CI run IDs;
- [ ] post-merge staging workflow run/job IDs;
- [ ] `STAGE_MANIFEST.txt`;
- [ ] `SHA256SUMS` plus successful verification output;
- [ ] desktop `.deb`/AppImage hashes;
- [ ] Android APK hash;
- [ ] host distribution/version and amd64 architecture;
- [ ] package architecture;
- [ ] Samsung Galaxy A54 model/product/device identity;
- [ ] Android version/API level;
- [ ] validator `environment.txt`;
- [ ] validator evidence JSON;
- [ ] packaged-app launch logs;
- [ ] exported diagnostics;
- [ ] exact commands used;
- [ ] explicit pass/fail result for every manual validator prompt.

Evidence must be tied to the exact release candidate SHA. Do not reuse evidence
from an earlier branch head or earlier package/APK.

---

## 6. Reconciliation and closure

Only after Sections 1-5 are satisfied:

- [ ] Update `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` Block 46.3 boxes
      with the real evidence.
- [ ] Update this closeout TODO with exact run IDs, hashes, host/device details,
      and evidence paths.
- [ ] Update the current Ralph handoff with the completed Block 46.3 state and
      the exact next task.
- [ ] Re-run applicable documentation/source-size gates after reconciliation.
- [ ] Confirm no manual acceptance result was inferred solely from CI.
- [ ] Mark Block 46.3 complete only when the real graphical/physical validator
      has passed.

**Block 46.3 acceptance:** an exact-SHA packaged Linux desktop release
candidate and Android APK have been staged from the same post-merge `master`
commit on a supported Ubuntu 22.04 or Ubuntu 24.04 amd64 host; the physical
Samsung Galaxy A54 has participated in the real validation; the manual
graphical validator has passed without fabricated input; install/profile/source/
listener/diagnostics/shutdown/reopen/uninstall behavior has been observed; and
the complete evidence set has been preserved and reconciled.

---

## 7. Boundary with Block 47

Block 46.3 proves one packaged Linux desktop host with one physical Android
listener on a supported fresh-machine baseline. It does **not** close the full
interoperability matrix. Block 47 remains responsible for the complete
production matrix, including two-listener physical acceptance and its separate
mDNS/QR/failure/reconnect/synchronization evidence.
