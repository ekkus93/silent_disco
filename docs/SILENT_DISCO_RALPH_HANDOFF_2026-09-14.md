# Silent Disco Ralph handoff — 2026-09-14

This handoff is the authoritative resume point for the next ChatGPT session.

## Repository and session rule

Repository: `https://github.com/ekkus93/silent_disco`

Pre-handoff `master` head:

```text
1dd3c4d5127d518f41726007a2311a78a4f3b563
```

That commit added the self-hosted Android device probe workflow.

**Important for the next session:** use **Ralph Bridge** for GitHub and GitHub Actions operations. Do not fall back to the default GitHub connector unless the user explicitly authorizes that in the new session. If a required Ralph Bridge capability is unavailable, state the missing capability rather than silently switching tools.

---

# 1. Current project state

The project is in **release acceptance / physical validation**, not core feature development.

The major desktop, shared-Rust, Android-listener, transport, staging, decoding, diagnostics, Lab Mode, fault-injection, replay, packaging, and CI implementation work is substantially complete.

The remaining work is dominated by release-candidate qualification and evidence collection on real environments/devices.

## Block 46 status

### Block 46.1 — Linux packaging

Complete.

The project has selected and implemented Linux packaging for the release candidate.

### Block 46.2 — package install / upgrade / uninstall qualification

Complete through CI evidence.

Existing package qualification covers construction plus clean install / upgrade / launch / uninstall behavior and preservation of user data as documented in the repository.

### Block 46.3 — fresh-machine validation

**Still open. This is the immediate next milestone.**

The authoritative runner already exists:

```text
desktop/scripts/validate-block46-fresh-machine.sh
```

The validator is intentionally interactive because it contains physical / GUI acceptance points that must not be faked by blindly piping `PASS` into the process.

The agreed execution model is:

1. use GitHub Actions to build/stage the exact-head release candidate artifacts on the self-hosted environment;
2. use a normal graphical terminal on that same machine to execute `desktop/scripts/validate-block46-fresh-machine.sh` manually;
3. use the attached physical Samsung A54 for the Android-listener portion;
4. answer the validator's manual acceptance prompts based on actual observed behavior;
5. preserve the resulting evidence and diagnostics;
6. if Ubuntu 24.04 exposes a failure, treat it as a product/packaging/validation bug and fix it rather than declaring 24.04 unsupported.

---

# 2. Self-hosted runner and physical Android device

A GitHub Actions self-hosted runner is available.

Host operating system:

```text
Ubuntu 24.04 amd64
```

A physical Samsung Galaxy A54 is attached to the runner machine over ADB.

A smoke workflow was added at:

```text
.github/workflows/self-hosted-android-device-probe.yml
```

Commit:

```text
1dd3c4d5127d518f41726007a2311a78a4f3b563
```

The first probe run passed:

```text
Workflow: Self-hosted Android device probe
Run:      34888621182
Job:      104125408261
Result:   SUCCESS
```

The probe established that GitHub Actions running on the self-hosted runner can reach an authorized **physical** Samsung Android device via ADB and execute `adb shell` commands.

Observed device metadata from that successful run:

```text
manufacturer: Samsung
model:        SM-A546E
product:      a54xnsxx
Android:      16
API level:    36
ABI:          arm64-v8a
physical:     yes
```

The probe was intentionally non-destructive: it did not install/uninstall apps or clear device state.

This establishes that the self-hosted runner + A54 combination is viable for physical Android acceptance automation and for the device-dependent portion of Block 46.3.

---

# 3. Ubuntu 24.04 compatibility decision

The user explicitly decided that the supported Block 46.3 Linux baseline is:

```text
Ubuntu 22.04 amd64 OR Ubuntu 24.04 amd64
```

If the application, packaging, desktop integration, D-Bus/keyring interaction, or validation tooling fails on Ubuntu 24.04, that is a bug to be fixed.

Do **not** require the user to replace the Ubuntu 24.04 self-hosted host with 22.04 merely to satisfy Block 46.3.

## Known validation-harness bug

The current `master` version of:

```text
desktop/scripts/validate-block46-fresh-machine.sh
```

still assumes Ubuntu 22.04 only. It currently hard-fails if `/etc/os-release` does not report 22.04 and its evidence metadata hardcodes the baseline as `ubuntu-22.04-amd64`.

This must be corrected before authoritative Block 46.3 evidence is collected on the Ubuntu 24.04 host.

The intended behavior is:

- accept Ubuntu 22.04 amd64;
- accept Ubuntu 24.04 amd64;
- reject unsupported distro/version/architecture combinations explicitly;
- record the **actual detected OS version and architecture** in evidence rather than hardcoding 22.04.

The Block 47 Android interoperability validator has the same 22.04-only assumption and should be reconciled at the same time so the acceptance policy is consistent.

Relevant file:

```text
desktop/scripts/validate-block47-android-interoperability.sh
```

Documentation that still describes fresh-machine validation as 22.04-only should also be updated to state 22.04 or 24.04 where the text is normative/current.

---

# 4. Exact next task for the new session

Resume with **Block 46.3 fresh-machine validation**.

The first implementation task is to reconcile the validation harness and current documentation with the supported Ubuntu 22.04/24.04 policy.

Recommended sequence:

1. Connect to `ekkus93/silent_disco` with **Ralph Bridge**.
2. Reload current `master`; do not assume the SHA in this document is still HEAD after the handoff commit.
3. Inspect:
   - `desktop/scripts/validate-block46-fresh-machine.sh`
   - `desktop/scripts/validate-block47-android-interoperability.sh`
   - current README / acceptance TODO language that defines the Linux baseline.
4. Change the validators to accept Ubuntu 22.04 or Ubuntu 24.04 amd64 and to report the actual detected baseline in evidence.
5. Update only the current normative documentation needed to make the acceptance contract consistent.
6. Run shell syntax checks and any relevant repository validation locally/through CI.
7. Commit/push the fix through Ralph Bridge.
8. Run exact-head CI and fix failures until green.
9. Obtain/stage the exact-head desktop Linux release bundle and Android release-candidate APK produced by CI.
10. On the Ubuntu 24.04 self-hosted machine, from a normal graphical terminal, execute the Block 46.3 validator manually against those exact-head artifacts.
11. Use the attached A54 as the real Android listener.
12. Preserve the validator evidence JSON, diagnostics export, environment information, launch logs, package metadata/hashes, and any other generated evidence.
13. If the validation fails, diagnose and fix the real failure, rerun exact-head CI, restage artifacts, and repeat Block 46.3 until it passes or a genuine user-dependent blocker is identified.
14. Once Block 46.3 genuinely passes, update the authoritative TODO/acceptance ledger with the exact commit/run/evidence and move immediately to the remaining physical/release gates.

Do not mark Block 46.3 complete from CI build success alone. The manual graphical/physical acceptance portion is part of the block.

---

# 5. Artifact staging for Block 46.3

Existing CI already publishes the release-candidate artifacts needed for this work. The current artifact names observed in the project are:

```text
desktop-linux-bundle
android-release-candidate
```

These should come from the exact candidate SHA being validated.

Do not mix a desktop package from one commit with an APK from another commit.

The normal release-candidate flow should record enough information to prove:

- source commit SHA;
- desktop package version and SHA/hash;
- APK version and SHA/hash;
- host OS / architecture;
- Android device inventory;
- pass/fail status for each manual acceptance point;
- diagnostics/evidence paths.

---

# 6. Block 46.3 environment expectations

The self-hosted Ubuntu 24.04 machine must function as a genuine fresh-machine desktop acceptance environment.

Before running the validator, confirm the items the runner actually depends on, including:

- `x86_64` / amd64 host;
- graphical desktop session and usable `$DISPLAY`;
- user D-Bus session;
- Secret Service/keyring support expected by the desktop app;
- ability to install/remove the `.deb` as required by the validator;
- no stale Silent Disco install/profile state that invalidates the intended fresh-machine step, except where the validator explicitly tests persistence/reopen behavior;
- network topology that permits the desktop host and A54 listener to communicate as intended;
- ADB authorization remains valid for the A54.

If the GitHub Actions service account itself cannot drive the GUI session cleanly, that is not a reason to fake interaction inside CI. The agreed plan is to stage artifacts with CI and execute the interactive validator from a real terminal in the graphical user session.

---

# 7. What remains after Block 46.3

After Block 46.3 is complete, the remaining release work includes the following major areas.

## Block 47 — final Android interoperability acceptance

The repository already contains:

```text
desktop/scripts/validate-block47-android-interoperability.sh
```

Block 47 covers end-to-end packaged desktop-to-physical-Android interoperability such as joining, discovery/QR/manual paths as applicable, playback/control/reconnect behavior, diagnostics, restart/persistence, network/source/output failure handling, and synchronization evidence.

The attached A54 can cover one physical listener.

The full two-listener synchronization acceptance still requires a **second distinct physical Android device** unless the authoritative acceptance specification is deliberately changed.

## Physical playback review

Some remaining acceptance items require actual physical listening / playback observations and cannot be honestly closed from loopback/emulator-only evidence.

Of particular importance is real two-phone listener-to-listener synchronization measurement after the implemented alignment work.

## Block 45 — measured performance evidence

The implementation/instrumentation exists, but authoritative measured evidence still needs to be collected for the required release metrics and limits, including applicable CPU/RSS/throughput/queue/synchronization/underrun/shutdown/database behavior.

Do not invent performance limits or promote CI-host measurements to universal product limits without the evidence required by the acceptance plan.

## Block 48 — final qualification / release ledger

The final candidate must reconcile the repository's complete gate set and evidence ledger. This includes the applicable Rust, Android, frontend/Tauri, Linux packaging, Lab Mode, loopback/integration, and physical Android acceptance gates.

Once all required evidence is complete, reconcile the authoritative TODO and final project/release memory/ledger documents.

---

# 8. Known bookkeeping issue

A final completion checklist was observed to still mention a supposedly unfixed `stop_playback` bug even though the corresponding earlier block documents that bug as fixed with regression coverage.

When reconciling the final acceptance ledger, verify the current code/tests and remove or correct stale completion-checklist language rather than treating that stale sentence as a newly discovered defect.

---

# 9. Working principles for the next Ralph Loop

- Use **Ralph Bridge** for repository and CI operations.
- Work from current `master` and reload it after merges/direct publications.
- Qualify the exact head being promoted.
- Keep physical/manual checks honest; do not synthesize `PASS` input to bypass them.
- Treat Ubuntu 24.04 failures as bugs because 24.04 is an intended supported baseline.
- Use the attached A54 wherever a single physical Android device is sufficient.
- Preserve evidence artifacts and exact commit/run IDs.
- Do not stop for ordinary CI/test failures; diagnose and fix them.
- Stop only for a concrete blocker that genuinely requires user action, credentials, hardware, authorization, or a product decision.

---

# 10. Immediate resume command / intent

The next session should begin with the equivalent of:

> Use Ralph Bridge with `https://github.com/ekkus93/silent_disco`. Read `docs/SILENT_DISCO_RALPH_HANDOFF_2026-09-14.md`. Resume Block 46.3. First reconcile the Ubuntu 22.04/24.04 acceptance policy in the validators and current docs, then qualify exact-head CI and proceed to the manual fresh-machine validation on the Ubuntu 24.04 self-hosted runner machine with the attached Samsung A54.

That is the current project resume point.
