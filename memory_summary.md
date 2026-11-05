# Memory summary

_Generated: 2026-11-05T15:18:22Z — derived from memory.md; do not edit by hand._

## Current state
- **Active track: code review remediation** (`docs/SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md`, findings F1–F6). Latest recorded work (2026-11-05 15:17Z entry, Sonnet 5.5): F3 item 5 checked on the Rust transport boundary, citing exact-head Rust CI on `acc29010`; F3 item 3 partly evidenced by a new desktop stale-approval test. (The matching git commit is `0a59b28a`, taken from git history, not from memory. Memory does not record CI on it.)
- Earlier the same day (15:02Z entry): duplicate-join counter added and `CounterState` split out of `socket/shared.rs` (796 → 648 lines). Its exact counts are asserted, and removing the production call fails the test.
- The 800-line source limit is now a **guideline**, not a hard rule (see Decisions). The CI gate still enforces 800 until changed.
- **Physical acceptance track** (Blocks 46.3, 47, 52.8, 31, 33, 45.3): mostly single-phone results. Remaining gaps are listed under Open items.
- Acceptance host history: `arisu` (Ubuntu 22.04.5) per 2026-09-16; `kawa` (Ubuntu 24.04.4) used for the 2026-10-13 performance runs.
- Handoff docs: the 2026-11-05 entry records that `CLAUDE.md` now points to the 2026-10-28 remediation handoff (`docs/SILENT_DISCO_CODE_REVIEW_REMEDIATION_HANDOFF_2026-10-28.md`). That pointer was assumed current and not confirmed by the owner. The older `docs/CLAUDE_CODE_HANDOFF_2026-09-16_BLOCK_52.md` and `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` are still referenced by the physical-acceptance entries.

## Timeline
Chronological by recorded date. Model names are as recorded. Entries with no model name are from before the model convention began in this log.

- **2026-09-16 (Claude Opus 5), Blocks 49–52:**
  - Desktop shell rework, Settings, and CGNAT opt-in (Blocks 49–51). Block 46.3 acceptance paused. The `a3beac2` stage is invalid because later commits changed desktop code.
  - Carrier-NAT (100.64.0.0/10) audio confirmed through the device-test harness. The packaged GUI host dropped the listener on Play.
  - Block 52.1 root-caused and fixed: the 8 s silence guard evicted listeners approved before Play. Verified through the packaged GUI.
  - Blocks 52.2 and 52.3 device-verified. 52.6 fixed: the mDNS path never started the sync probe loop.
  - Block 52.4: a refused effect silently stopped core notification delivery. Fixed, with tests.
  - Block 52.5: misleading delivery banner and "connecting" after approval fixed. Device-verified. Broadcast counter split into partial and undelivered.
  - Session handoff written to `docs/CLAUDE_CODE_HANDOFF_2026-09-16_BLOCK_52.md`.
- **2026-10-13 (no model recorded):**
  - Desktop host TODO continuation (Blocks 4, 23, 26, 49–52): strict Clippy fixes, playback control enqueue compensation, shared-address-space validation.
  - Block 45 performance: a single-machine probe, then three release baselines and a 60 s 16-listener soak on `kawa`. Limits not set.
  - Final local gate rerun and A54 instrumentation (98 connected tests passed).
  - A54 long-pause acceptance: 120 s pause, resumed in sync, 7,121 packets delivered.
  - Profile identity, mixed-device run (inconclusive: one emulator listener dropped). Diagnostics cadence fixed at 2,000 ms.
  - Lab fault-editor test race fixed (test synchronization only, no production change).
  - Ubuntu 22.04 fresh-VM attempt failed on GLIBC 2.39 vs 2.35. Block 46.3 open.
- **2026-10-14 (no model recorded; many entries):**
  - Earlier entries in this date range say ADB and GUI automation were intermittently unavailable. A later entry says that was a command-invocation issue and the direct ADB commands work. The "unavailable" claims are superseded.
  - Partial Block 47 physical run: mDNS discovery, approval and playback on the A54 (Android 16 / API 36). Rejection false negative traced to missing correlation of delivery IDs, then fixed. Physical rejection confirmed.
  - Session history persists across profile reopen. Session IDs made unique across actors. Startup recovery of interrupted host rows added, tested but not observed in a package.
  - Packaged QR join and playback with a fresh invitation. The QR expiry timer works in the packaged UI. A QR join was verified, after the sync-state issues below.
  - Virtual sink removal probe: does not reproduce a physical unplug. Block 33 device removal stays open.
  - Automation permission regression: a sandboxed wrapper's failure was not a real device failure. The model was told not to edit permission rules to work around it.
  - Block 31 QR readability fixed (decoded offline at 640 px). Firewall (Block 24) remains unverified because root is needed.
  - Stop and end-of-session issues were found during the packaged checks: Stop left the phone out of sync, and the listener card stuck at "connecting".
- **2026-10-15 (no model recorded; many entries):**
  - Android retry clock reset: sync estimator reset on failure, retry and leave.
  - Clean-close handling: a clean peer close is routed to normal leave. Broken pipe on the final disconnect is treated as an expected peer close.
  - End-while-playing investigation: first a stuck state, then a revision race. Root cause: the actor rejected `EndHostSession` when the UI-sampled revision had moved on. Fixed in core. Stale-revision End now ends the current session.
  - Connected End verified on a fixed packaged AppImage, using the manual endpoint route. Discovered-session return-home is **not** verified.
  - Disconnect/reconnect attempts: mDNS and QR limits. Recovery action exposed after host loss (`2609475`, `4641fa3`); reconnect routes to Nearby Sessions (`9d31ec0`). A forced process kill and restart recovered to Host Setup (process-restart evidence only).
  - Diagnostics export verified. Ubuntu 22.04 clean-container startup smoke passed (basic only).
- **2026-10-16 (Claude Opus 5.5):**
  - Exact-head gates at `af8e090` (API 36 emulator, no physical devices): Android 119/120 passed (1 skipped), desktop 157 frontend tests, desktop Rust 347 passed.
  - Listener pause/resume fixes: BUFFERING state on resume, boundaries withdrawn on Pause.
  - Host decode failure now cancels the pending boundary immediately.
  - Boundary commit-point edit race found and left open (see Open items).
- **2026-11-05 (Sonnet 5.5), remediation track:**
  - 12:48Z: `CLAUDE.md` `/init` review. Handoff pointer updated, commit-hook note corrected, gates documented.
  - 13:29Z: F1 reproduction (TODO §0.2). Desktop and Android tests that a claimed device ID never becomes trusted; both fail under mutation. Box left unchecked: no exact-head CI, and the Android test uses a fake controller.
  - 14:48Z: F3 counters exposed over FFI (TODO §3 item 4). Box left unchecked.
  - 14:48Z: 800-line limit relaxed to a guideline (owner decision).
  - 15:02Z: duplicate-join counter and `CounterState` split.
  - 15:17Z: F3 items 3 and 5 audit. Item 5 checked on the Rust boundary. Item 3 left open.

## Decisions & preferences
- **File size (2026-11-05, owner):** 800 lines is a guideline. The rule is no God files. Claude decides when to refactor, splitting by responsibility and not just to hit a number. The CI gate `scripts/check-source-file-line-counts.sh` still enforces 800 until the owner changes it.
- **Remediation checkboxes (2026-11-05 entries):** TODO §0.2 and the F3 items were left unchecked while exact-head CI or the real-actor or Android path was missing. The TODO header says a box is checked only with production code, negative and positive tests, and exact-head evidence.
- **Physical acceptance (2026-10-13 to 10-15):** local gates and simulations are not physical proof. Do not claim a physical pass from simulation, and do not use instrumented or probe builds for acceptance. The two-physical-phone matrix is skipped at the user's direction (2026-10-15).
- **Git hygiene (2026-10-14 to 10-15):** do not commit the broad dirty worktree as one change. Commit isolated slices.
- **Generated code (2026-10-14):** do not hand-edit generated bindings. Their trailing whitespace in `git diff --check` is expected.
- **Performance (2026-10-13 to 10-14):** do not set platform listener limits or timing thresholds from one machine's virtual-listener runs.
- **Tooling (2026-11-05):** run `./gradlew` alone, not in parallel with other Bash calls; a parallel run exited 127 because of a working-directory race.
- **Android UI (2026-10-13):** opaque 64-hex host identities render as `Host <8 hex chars>` in listener-facing UI. The full ID is kept for protocol and trust operations.
- **Screenshots (2026-10-14):** X11 captures of the composited webview can be blank. GNOME captures are reliable. Do not treat X11 blank captures as a product bug.
- **Packaging (2026-09-16):** `./gradlew test lintDebug` does not validate Rust schema changes. Use the Rust gates for those.

## Open items
- **Remediation F1 (TODO §0.2):** exact-head CI for the reproduction commit is not recorded. The Android test does not exercise the approved-only outcome through the real Rust actor via FFI.
- **Remediation F3 item 2:** automatic finite client retry on a busy reply is not implemented. Retry is user-driven. Owner decision needed.
- **Remediation F3 item 3:** the Android stale-approval refusal (`pendingListenerDatagramPorts.take`) has no JVM test because the function is file-private. Planned as part of F4.
- **Remediation F3 item 4:** no Android or desktop UI or diagnostics surface shows the new counters. No exact-head CI yet.
- **Remediation F3 item 5:** checked for the Rust boundary only. Android bridge and permanent-spinner requirements remain under F5.
- **Gapless boundary edit race (2026-10-16):** a worker commits a track boundary about 640 ms before hosts report it scheduled. Edits in that window are accepted, but the removed track still plays. Options (actor-approved commit, immediate commit report, end-of-track lock margin) were presented and not decided. The TODO §3 item was reopened.
- **Connected End (2026-10-15):** verified on the manual endpoint route only. Discovered-session return-home and coupled listener teardown are not verified.
- **Automatic reconnection (2026-10-15):** not verified. Only user-driven recovery (Fix connection → Reconnect) has passed.
- **Unauthorized-peer diagnostic (2026-10-14):** the desktop reported `control frame was not sent by the authorized peer identity` while playback was healthy. The desktop also logged a sync-path `Unauthorized` datagram during a network interruption (2026-10-15). No later entry records these as resolved.
- **Physical hardware gaps:** a second physical Android listener (Blocks 29, 47; two-phone matrix skipped by user direction), a clean Ubuntu 22.04 graphical VM run (Block 46.3), physical device hot-unplug (Block 33), and physical listener scaling (Block 45.3).
- **Firewall (Block 24):** runtime policy unverified. `ufw.conf` shows `ENABLED=no`, but the effective rules need root.
- **Performance (Block 45):** UI rendered-update rate, listener ceilings and stable timing thresholds are unset.
- **Battery saver (2026-09-16):** the A54 `low_power` setting was set to 0 for testing and must be restored to 1 by the user. No later entry confirms this.
- **Historical flake:** the `LabScreen.test.tsx` link-fault test was flaky. Entries from 2026-10-13 and 2026-10-14 record a synchronization fix that passed repeated isolated runs. Those entries describe it as a fix, not a root-cause proof.

## Provenance notes
- The previous summary also covered June–July 2026 entries (early Android PoC and Ralph loops, and the Rust migration blocks). Those entries are not in the current `memory.md`, so they are not carried forward here.
- The "Luna automation handoff" at the top of `memory.md` (2026-10-14) describes a restricted session that later entries supersede.
- Several Block 47 and Block 52 results depend on one A54 and one host machine. Their artifact hashes are in the log and were not re-verified for this summary.
