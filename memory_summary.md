# Memory summary

_Generated: 2026-10-14T17:49:07Z — derived from memory.md; do not edit by hand._

## Current state
- Latest work (2026-10-14, Ubuntu 22.04.5 packaging path, Block 47 / 46.3 acceptance): one physical Samsung Galaxy A54 (SM-A546E, Android 16/API 36) has verified mDNS discovery, host approval, sync lock, "Playing in sync", pause/resume, stop, and a packaged-AppImage session-history row that survives profile reopen. Host rejection now confirms correctly, and the QR invitation expires in the packaged UI. Latest AppImage SHA-256 recorded: `63929088fe1a2e5edda74231a07fda2f0d9ef5be932da0cec8e9bef1767084ca`; `.deb` `90fb149300ba1a6278fb4ba56635f99a9b47a554cc89a3375016eb82e8041b77` (last entry, Block 31 QR readability fix).
- Last code change in the log: startup recovery of interrupted host history rows (marked `desktop.host_session.interrupted`), focused tests pass, but no packaged runtime observation yet.
- Blocked on hardware/infra: second physical Android listener (Blocks 29/47), clean Ubuntu graphical VM for Block 46.3, privileged firewall inspection (Block 24).
- Acceptance host is `arisu` (Ubuntu 22.04.5 amd64) per 2026-09-16; `kawa` is Ubuntu 24.04.4 and was used for 2026-10-13 performance runs.
- Handoff docs referenced by the log: `docs/CLAUDE_CODE_HANDOFF_2026-09-16_BLOCK_52.md` (Blocks 49–52), `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`, `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`, `docs/SILENT_DISCO_NONPHYSICAL_CLOSURE_CHECKPOINT_2026-08-19.md`.

## Timeline
Entries are listed chronologically by their recorded dates. Model names are as recorded.

**June–July 2026: early Android PoC and Ralph Loops**
- 2026-06-07 (Claude Sonnet 4.6): session start; created CLAUDE.md and memory; documented Ralph Loop workflow; completed TODO 11.4 (stable playback callback/write behavior); completed `docs/CODE_REVIEW1_TODO.md` (all 16 tasks) after a UI/UX review.
- 2026-06-28 (Claude Haiku 4.5, Sonnet 4.6): CODE_REVIEW2 Ralph Loop, FIX3, FIX4, FIX5 loops each reported complete (8–9 blocks each); lint, unit, and on-device tests recorded.
- 2026-07-25 (GPT-5.6 Thinking): Rust migration ownership inventory (Block 1.2), compatibility baseline, Block 3 Android Rust integration code, Blocks 4–5, Block 6 synchronization code (physical acceptance not run; no device attached), Block 7 SQLite worker, Block 8 ordered/checksummed migrations, Block 9 Rust-owned Android persistence (instrumented test not run; no device).
- 2026-07-27 to 07-28 (GPT-5.6 Thinking): Tauri desktop Block 1 baseline; Block 10 core ownership.
- 2026-07-30 to 07-31: desktop Blocks 13 (authority closure), 15 (platform-effect runner), 16 (secure audio source), 17 (atomic staging), 18 (decoder decision), 19 (bounded streaming decode), 20 (shared transport runtime), 21 (network bind policy), 22 (manual endpoint host), 23 (listener management).
- 2026-08-01: Block 24 physical Android control interoperability complete; shared Block 14.2–14.4 packetizer, 15.1–15.5 jitter buffer and concealment/scheduler (Block 15 closed), 16 SPSC render ring, 17 real-time C ABI, 18 Oboe adapter (verified on device); BLE advertise and Wi-Fi Direct bugs fixed on a fresh install.

**August 2–3, 2026: audio pipeline bring-up and quality work**
- 2026-08-02 (Sonnet 5): Block 13 listener lifecycle and UniFFI surface; Wi-Fi Direct peers no longer shown as joinable sessions; Block 20 networking moved to Rust transport; clock sync and audio delivery restored over Rust transport; desktop host real streaming (Blocks 25–27); Battery Saver root-caused as the cause of an ECONNREFUSED blocker; first audible live playback; app-crash race fixed; send-ahead horizon added; clipped-tail and skew-estimation bugs fixed.
- 2026-08-02 (Fable 5): Kotlin audio-quality fixes device-validated over 8 runs; root-caused popping as hard-edged silence concealment of normal Wi-Fi loss; wrote plan to move all audio into Rust core.
- 2026-08-03 (Opus 5, several entries): listener playback migrated to Rust; Kotlin owns no listener audio after Phases 4–5; burst concealment and seam fixes; 5 ms packets; ring drain and concealment gain schedule; one alignment attempt regressed on device and was reverted. Audio quality accepted by user at 16:26:57Z; focus moved to Android and desktop app work.
- 2026-08-03T18:40:46Z (Sonnet 5): session pause and handoff; HEAD `2c7a76e`.

**August 8–10, 2026: device sessions, sync fixes, and the Ralph Loop A/D items**
- 2026-08-08 (Sonnet 5): LG G6 temporarily unavailable; Block 27 closed (two product decisions and two real bugs); Block 28 prep; two-emulator dry run found shared Android device identity bug.
- 2026-08-09 (Sonnet 5): LG G6 available; Block 28 live session; minSdk lowered to 26 (user's explicit call); two transport bugs and pause/resume timeline fixed.
- 2026-08-09 (Opus 5): track-switch scratchiness traced to repeated hard resyncs, not Exclusive/Shared mode; Rust now submits audio directly ("much better" per user); rebuffer target separated (Android tuning measured worse and reverted).
- 2026-08-10 00:10–00:40 (Opus 5): 12.5% "tail" identified as startup stall; `recv_event(&self)` fix raised sync acceptance 19% → 98%.
- 2026-08-10 03:23–08:05 (Sonnet 5, Ralph Loop A/D items): A1 did not reproduce, BLUETOOTH permission crash fixed; A2 t4 stamped at socket receipt; A3 pending sync probes now expire; A4.1 variance check; A5 FLAC clean, MP3 works but trails; D1 device-independent failure regression tests; A6 real-device host-blindness gap found and fixed; D2 three-stack UI visibility fix confirmed; A4.4 offset-driven hard resyncs counted; A4.2/A4.3 `STARTUP_BUFFER_MS` and `REBUFFER_TARGET_MS` lowered 1000→400 ms (controlled 4-vs-4 A/B); A6 silent-host-partition test found listener-side blindness (not fixed, logged as item 7).
- 2026-08-10 06:57:56Z (Sonnet 5): D4 bookkeeping closed Block 28.1/28.2; 28.3 left honestly partial.
- 2026-08-10 13:20–18:40 (Sonnet 5): Blocks 35 (desktop diagnostics screen and export), 36 (deterministic shutdown), 37 (Lab Mode feature-gated build and isolation), 38 (deterministic virtual clocks), 39 (virtual transport and fault injection; reconnect delay deferred).
- 2026-08-10 19:10 (Sonnet 5): Block 21 (Android `MainViewModel` reduction) partially done; dead code removed; host audio packetization left in Kotlin pending Block 23.
- 2026-08-10 19:39–22:40 (Sonnet 5): Block 40 (scenario schema, runner, assertions; JSON not YAML); Block 25 fuzz/property testing (one bug fixed); Block 41 recording and replay; Block 42 Lab Mode UI; shared Block 24 FFI hardening; Block 43 security and Tauri capability audit; Block 21 follow-up rewiring pre-runtime sync to `RustCoreBridge.openSyncEstimator()`; Block 21.4 effect-runner fake infrastructure; Block 44 silent-failure audit (7 bugs fixed).

**August 11–19, 2026: refactors and non-physical closure**
- 2026-08-11 (Sonnet 5): Round-2 oversized-file split refactor, Tasks 1–9 (listener_playback, playback_pump, network, app_state, scheduler, HostSessionScreen, transport host, transport tests, lab_commands); closed out.
- 2026-08-19: non-physical closure checkpoint; Blocks 26/28/34/36/39–45/48 software-only work reconciled; Android listener playback Item 5.5 migrated. Rust 1.95.0 recovered (below CI pin 1.97.1); full compiler-backed gates not claimed. User: do not monitor CI; they report failures.

**September 16, 2026: CGNAT, Settings, and Blocks 49–52 device debugging (Opus 5)**
- 16:33Z: Block 46.3 paused by user; a3beac2 staging invalid. Acceptance host set as `arisu`. Venue Wi-Fi gave both devices 100.64.0.0/10 (CGNAT); Block 51 opt-in migration v3 (`allow_shared_address_space`, default 0) device-verified on A54.
- 17:14Z: desktop→A54 audio over CGNAT confirmed by ear through the device-test harness; packaged GUI host dropped listener on Play.
- 18:19Z: Block 52.1 root-caused and fixed: the 8 s silence guard evicted listeners approved before Play; fix arms the clock only on inbound sync/audio or delivered StreamStart.
- 18:33Z: Block 52.1 verified through packaged GUI.
- 19:17Z: Blocks 52.2/52.3 device-verified; 52.6 (probe loop never started on mDNS path) fixed; mDNS path plays end to end.
- 19:29Z: Block 52.4: a refused effect silently killed core notification delivery; now fed back as operation failure.
- 20:00Z–20:32Z: Block 52.5: delivery banner relabeled as control-message timing; "Streaming to nobody" alert added; listener progress wording fixed; broadcast counter split into partial vs undelivered. Device-verified at build 9a8767f.
- 20:35:45Z: session handoff written to `docs/CLAUDE_CODE_HANDOFF_2026-09-16_BLOCK_52.md`.

**October 13, 2026: continuation, performance evidence, and acceptance gaps**
- Desktop host TODO continuation (Blocks 4, 23, 26, 49–52): strict clippy restored; playback pause/resume enqueue failures compensated; shared-address-space changes validated before persistence; Host Session snapshot ordering fixes; Android pre-runtime probes after admission (JVM test added; physical verification pending).
- Block 45 performance: single-run probe, then paired release baselines on `kawa` (Ubuntu 24.04.4, Ryzen 7 5825U) and a 60 s 16-listener soak; virtual numbers only, no physical listener limit claimed.
- Final local gates green: Android `test lintDebug`, connected tests on A54 (98 completed, 0 failed), desktop `npm run check`, `scripts/check-rust.sh`, AppImage and `.deb` built and verified.
- A54 long-pause acceptance (120 s pause): passed; listener stayed in host snapshot; 7,121 first-song packets delivered.
- Mixed run A54 + emulator-5554: both joined and approved; one listener vanished; treated as inconclusive.
- Diagnostics refresh interval kept at 2,000 ms (0.5 Hz committed renders measured).
- TODO acceptance audit: several stale open items reconciled with code/tests; remaining items require physical devices, clean VM, runtime firewall inspection.
- Ubuntu 22.04 fresh-VM attempt: artifact built on 24.04 needs GLIBC_2.39; Jammy has 2.35; launch failed; not a bundle-CI regression; Block 46.3 open.

**October 14, 2026: packaged-host acceptance (Ubuntu 22.04.5 AppImage)**
- Partial Block 47 physical run (Ubuntu 24.04 host, A54): mDNS discovery and approval worked; 60 s WAV played in sync; a later join hit `desktop.operation_confirmation_missing`; rejection-click result unconfirmed at that time.
- Rejection confirmation correlated to the actual delivery operation ID (core/FFI/desktop); physical rerun confirmed the full rejection path.
- AppImage Block 47 continuation: AppImage and `.deb` built on 22.04.5; one-listener playback, pause/resume, stop, diagnostics export, clean shutdown observed (recording 58.695 s, non-silent PCM).
- Session IDs were colliding with persisted history (`UNIQUE constraint failed: session_history.session_id`); fixed with nanosecond + PID + nonce IDs; host session history rows now written and completed; verified across profile reopen.
- Connected host listener projection fixed (approval authorizes UDP ports; `ListenerConnected` submitted). Stop/end-session lifecycle needed a second request to complete in one run.
- QR invitation auto-expires in the packaged UI (verified 04:58–04:59 PDT); physical QR scan/join not yet verified (scanner did not send a request).
- Virtual audio sink removal probe: did not reproduce physical device loss; Block 33 physical removal item still open.
- Startup recovery: host rows left `active` by a crashed desktop are marked interrupted; focused tests pass; packaged runtime not yet observed.
- Block 45 repeated virtual evidence (Ryzen 5 5500U, 22.04.5); still no physical supported-listener maximum.
- Block 31 QR readability fix: generation at 1024 source pixels; decodes at 640 px (offline check only).

## Decisions & preferences
- **Audio quality**: accepted by user 2026-08-03; do not reopen unless asked.
- **Ask before starting the next block** when the log's established pattern is "per-block approval" (stated 2026-08-10 for Blocks 36–40; do not start unilaterally).
- **Personal phone** (LG G6) must not be used for physical runs unless user confirms it is free (2026-08-08); use emulators meanwhile.
- **minSdk** lowered to 26 to support the LG G6 (user's explicit decision, 2026-08-09).
- **Do not monitor CI**; user reports CI failures (2026-08-19).
- **Device evidence rule**: rebuild and reinstall the APK before any real-device test (a stale install once invalidated a result). Never claim device validation without the exact command, device model, Android version, and ABI.
- **Verify the packaged binary**: confirm `/usr/bin/silent-disco-desktop` matches the bundle being tested. Launch GUI as `silent-disco-desktop 2>&1 | tee ~/silent-disco-desktop.log` to keep output.
- **`./gradlew test lintDebug` does not validate Rust schema changes**; run the Rust gates separately.
- **Checking a box must not hide a known problem** (MP3 quality in Block 28; desktop clippy gate gap noted).
- **Lab identities** must never be selected automatically for production; production identity failure is a hard error (Block 37).
- **Scenario format is JSON**, not YAML (Block 40).
- **Commit attribution**: memory records a rule against `Co-Authored-By:` lines in commit messages (2026-06-28 FIX5 entry, consistent with CLAUDE.md). Current session attribution guidance differs; the project rule takes precedence per CLAUDE.md.
- **Do not hand-edit generated bindings**; `git diff --check` trailing whitespace in generated TypeScript is expected.
- **Diagnostics**: the local UI reports local send outcomes, not confirmed listener receipt.
- **Shutdown**: a timeout never frees callback-visible memory; `ShutdownFailed` is deliberately not reopen-safe (requires app restart).
- **Shared-address-space (CGNAT) is opt-in**; automatic selection only uses the best class present.

## Open items
- Second physical Android listener: Blocks 29 and 47 (and D3, multi-listener capacity); the two-listener sync measurement (the project's success criterion) has never been done on hardware.
- Clean Ubuntu 22.04 graphical VM / fresh-machine acceptance: Block 46.3.
- Full Block 47 matrix: packaged-app launch, QR scan/join on physical Android, second listener, failure and recovery scenarios, exact package/APK/network evidence.
- Block 24 firewall: effective rules unknown (UFW `ENABLED=no` in config but service active; nft/iptables need root).
- Block 33: physical device-removal checkbox open.
- Packaged Stop behavior: listener card lingered at `connecting` after Stop; host listener lifecycle status unresolved; listener-count persistence unresolved.
- Host sync-summary display on the desktop card unresolved.
- Listener-side silence watchdog (A6 item 7): a silent host partition is not visible to the listener; not fixed.
- Listener limits, UI rendered-update rate, and stable performance thresholds: unset pending broader machine data.
- Block 45.3 cadence and listener limit: open pending physical listener runs.
- Desktop clippy is outside the enforced `npm run check` gate; ~8 pre-existing deny-level errors on master.
- Unrelated flaky-test history: `LabScreen.test.tsx` link-fault test (fixed 2026-10-13 and 2026-10-14; watch CI).
- Startup recovery of interrupted host rows: focused tests pass; not yet observed in packaged runtime.
- Approval send delay (~1 min) and "11 partial frames" from Block 52.5 unconfirmed; per-packet correlation not yet done.
