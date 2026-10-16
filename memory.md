## Read first — Luna automation handoff (2026-10-14)

See [Current handoff for Luna — automation access](docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md#current-handoff-for-luna--automation-access-2026-10-14)
at the top of the active TODO. It records the proven Full access → restricted
session transition, the working direct ADB capture/pull commands, current
artifacts, and the remaining desktop permission blocker. Earlier statements
that ADB itself was intermittently unavailable are superseded by this diagnosis.
The user requires autonomous testing and does not want more manual test steps.

## 2026-10-14 — Current host session supersedes restricted-session handoff
- Current environment has GUI access (`DISPLAY=:0`, xdotool and
  `gnome-screenshot` succeeded) and direct ADB access to `R5CW31AX4FL`. The
  top-of-TODO statements about unavailable GUI/ADB describe the earlier
  restricted session and are superseded for this run.
- Launched packaged AppImage and exercised an mDNS join/approval with the A54
  on `192.168.87.0/24`. The 60-second 440 Hz WAV was audible; Android reported
  locked sync and reflected Pause/Resume. The desktop still said it awaited
  first sync, and Stop made the old installed Android app show out-of-sync
  while accepted probes continued. Implemented discovered-listener sync report
  forwarding and fixed Stop to cancel the per-runtime probe loop and return to
  Approved/Stopped. Added Android unit tests for both paths.
- `./gradlew test lintDebug --stacktrace --console=plain` passed. The updated
  `./gradlew assembleDebug --stacktrace --console=plain` passed. Installing
  failed with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`: the installed app's signer
  differs. Replacing it requires uninstalling and erasing app data, so the
  physical retest needs the user's choice before that destructive step.
- Current artifacts: AppImage SHA-256
  `dad27afb838e330951dd589da8c91c1675b7efc2f2ec0e3cb7bd16e2d01f0ffc`, `.deb`
  SHA-256 `e37bcb30733dc0f6bd944e65b727f2c258f456550aa2916de3196a92da5c82a8`,
  debug APK SHA-256
  `8e10000934a88d20f98fdb111407c8919dd9661bff6b1cb11d335fc37f37d710`.
- Current-session Rust gates passed: `bash scripts/check-rust.sh`; Tauri
  formatting, strict Clippy, 333 tests (5 ignored), and check; and
  `cd desktop && npm run check` (141 tests). Linux AppImage/.deb build and
  bundle-content verification passed. Do not commit the broad existing dirty
  worktree as a single change; it contains unrelated pre-existing changes.

## 2026-10-14 — Rebuilt Android APK physically verified after user authorization
- The user authorized uninstalling the A54's existing app. Uninstalled
  `com.ekkus.silentdisco`, which erased its local app data, then installed
  `app/build/outputs/apk/debug/app-debug.apk` successfully. Installed package
  reports versionName 0.1.0/versionCode 1; pulled installed APK SHA-256 matches
  the build at
  `8e10000934a88d20f98fdb111407c8919dd9661bff6b1cb11d335fc37f37d710`.
- Reused packaged AppImage session `QR Probe`; A54 discovered over mDNS on the
  same `192.168.87.0/24` LAN, joined, and was approved. Desktop listener card
  changed to `Connected — sync excellent`, Android showed `Clock sync locked`.
  The selected 440 Hz WAV played and Android showed `Playing in sync`. Pause,
  Resume, and Stop propagated correctly. After Stop, the phone remained at
  `Playback stopped` after 8 seconds while host sync remained excellent.
- Ended the desktop session; Android returned to its home screen and showed
  `QR Probe` in recent sessions, but displayed `Could not cancel the pending
  connection (Wi-Fi Direct is busy)`. Treat playback Stop as passed and clean
  end/shutdown as incomplete pending investigation.
- Screenshots: `desktop/src-tauri/target/a54-astra.png` and
  `desktop/src-tauri/target/a54-desktop-live.png` (these are ignored target
  artifacts). One listener only; QR physical scan, second-listener, disconnect
  recovery, and the remaining Block 47 failure scenarios remain open.

## 2026-10-14 — Scope Android Wi-Fi Direct cleanup to Wi-Fi Direct joins
- Investigated the BUSY warning after the A54 left the mDNS session. Both
  Session Join cancellation and Playback Leave called
  `DiscoveryLifecycleController.cancelJoinTransport` unconditionally, which
  requested Wi-Fi Direct cancellation although the active transport was
  standard-IP mDNS.
- Added `listenerHasPendingWifiDirectJoin` (active ride or pending establish
  operation) and gated the two UI cleanup call sites. Added a unit regression
  for standard-IP, pending Wi-Fi Direct, and active Wi-Fi Direct state.
- Android tests/lint passed again. `./gradlew assembleDebug --stacktrace
  --console=plain` passed in 6 s, then `adb install -r` succeeded using the
  previously user-authorized signing replacement. Physical A54 joined the
  packaged desktop session over mDNS, was approved, then Leave returned to the
  home screen without the Wi-Fi Direct BUSY warning.
- Current debug APK SHA-256:
  `8d9b9a96b1159f66524b585679c22f9a048615399d10ca33baf37a6ba19ed1f8`.
  The final full end-while-connected sequence and the rest of Block 47 remain
  open.

## 2026-10-14 — Physical end-while-connected and manual endpoint follow-up
- Reused the packaged AppImage (`0.1.0`, SHA-256
  `dad27afb838e330951dd589da8c91c1675b7efc2f2ec0e3cb7bd16e2d01f0ffc`) and
  rebuilt A54 APK (`0.1.0`, versionCode 1, SHA-256
  `8d9b9a96b1159f66524b585679c22f9a048615399d10ca33baf37a6ba19ed1f8`) on
  Ubuntu 24.04 x86_64, `wlo1`/`192.168.87.161`, phone
  `192.168.87.215`, same `192.168.87.0/24` LAN.
- The A54 discovered a fresh `QR Probe` host by mDNS, joined, and was
  approved. Ending the host while Android was waiting in `Getting audio in
  sync` returned the desktop to Host Setup. Android displayed `The nearby
  connection failed`; no Wi-Fi Direct BUSY warning appeared. This does not
  establish a clean listener teardown. The phone app process was force-stopped
  without clearing app data afterward.
- Opened Android `Connect manually` and copied the desktop's JSON endpoint
  payload. Direct ADB `input text` emitted malformed JSON because punctuation
  was dropped; the app correctly rejected it before starting a join. Manual
  endpoint acceptance remains open pending actual paste or physical scan.
- Hash of rebuilt `.deb`: `e37bcb30733dc0f6bd944e65b727f2c258f456550aa2916de3196a92da5c82a8`.
  Phone screenshot is at `desktop/src-tauri/target/a54-astra.png`; desktop
  screenshot at `desktop/src-tauri/target/a54-desktop-live.png` (ignored build
  artifacts).
- Prepared a `QR Probe` invitation and opened the A54's in-app QR scanner after
  granting camera permission. The first invite expired before the phone was
  physically positioned, so it was refreshed; the current invite expires at
  11:13:29 PM PDT. The camera still points away from the display, so scan/join
  remains pending and is not counted as a pass.

## 2026-09-16T16:33:37Z - Claude Opus 5 - Desktop shell rework, Settings, CGNAT opt-in (Blocks 49-51); Block 46.3 paused
- Block 46.3 acceptance paused mid-run by user choice. The a3beac2 stage
  (~/silent-disco-block46-stage/a3beac2.../) is now INVALID: every later commit
  touches desktop/src, so desktop bundles and APK must be rebuilt from one
  current master SHA. Recorded as closeout TODO section 3a.
- Acceptance host is `arisu` (Ubuntu 22.04.5 amd64), not `kawa`/24.04 as the
  2026-09-16 handoff claimed; user confirmed. Release path pins Node 22.16.0
  (nvm) and Java 17 (/usr/lib/jvm/java-17-openjdk-amd64); this machine defaults
  to Node 24 / Java 21, which mismatch.
- Validator gotchas hit: it requires selecting its *generated* WAV (byte-identity
  check, not a format check -- MP3 is supported); it refuses to run while
  `silent-disco` is installed, and any failed run leaves it installed.
- Venue Wi-Fi "Revival Hall" hands out 100.64.0.0/10 (CGNAT) to both laptop
  (100.64.64.132) and A54 (100.64.65.39); peers can ping each other (no client
  isolation) but ping jitter was high (mdev ~50 ms).
- 2ec12b5 (Blocks 49-50): revived dead uiSlice as single activeView router,
  first shared components in desktop/src/components, Settings destination,
  fixed storage-gated-on-ready, discarded recoverableAction, unrendered
  retryable. d761921: network policy control lives only on Settings; setup shows
  a readiness summary; rule extracted to networkReadiness.
- 9aa075e (Block 51): CGNAT opt-in. Core migration v3 adds
  app_settings.allow_shared_address_space (default 0). Bugs caught by own tests:
  persist_settings writes the whole row and would have revoked the opt-in;
  automatic selection picks by default route, so class ranking in the sort was
  ineffective until selection was restricted to the best class present.
- Device results 2026-09-16: A54 (R5CW31AX4FL, Android 16/API 36) upgraded a
  real v2 profile to v3 via `adb install -r` -- user_version 2->3, integrity ok,
  row counts unchanged, opt-in 0. Desktop profile (from Aug 1) also migrated to
  v3 at launch 09:31 PDT. Production classifier on this host's real interfaces:
  wlo1 100.64.64.132 = SharedAddressSpace, refused without opt-in, auto-selected
  with it; all Docker bridges stayed Container.
- IMPORTANT: `./gradlew test lintDebug` does NOT validate Rust schema changes --
  buildRustAndroidDebug is wired to mergeDebugJniLibFolders, not test.
- Desktop GUI cannot be automated here: GNOME Wayland session, xdotool sees no
  app windows. Android side is fully drivable via adb/uiautomator.
- LabScreen.test.tsx has a latent timing flake (setLabLinkFaults waitFor); seen
  once in ~7 full runs, not investigated.
- Still unverified: an actual audio session over CGNAT (Block 51.5 last item).
  Rebuilt desktop package from 9aa075e is installed and running on arisu.

## 2026-09-16T17:14:12Z - Claude Opus 5 - Carrier-NAT device session: audio works via harness, GUI host drops listener on Play
- Desktop->A54 audio over carrier-NAT (100.64.64.132 -> 100.64.65.39) confirmed
  by ear via the device-test harness, now opt-in capable with
  SILENT_DISCO_DEVICE_TEST_ALLOW_SHARED_ADDRESS_SPACE=1 (manual_real_android_
  listener_plays_mp3): 7119/7119 frames delivered, sync Excellent, RTT 3.75-6.17
  ms, drift 35 ppm. Recorded as Block 51.5.
- The packaged GUI host (9aa075e) dropped the listener on Play twice (mDNS join
  and manual join). Harness path is fine, so the bug is in the GUI command /
  platform-effect path, not network or core transport. Block 52.1 -- start here.
- Other device findings in Block 52: Android join-before-endpoint-ready race
  (52.2, seen once), mDNS expiry resetting a joined listener (52.3, suspected),
  approval delivery hung >3 min with no visible failure after a listener app
  restart (52.4), stale "1 of 1 succeeded" banner while streaming to nobody,
  and no logcat output from Android's manual-connect path (52.5).
- Clean-close disconnects log nothing on either side; the GUI app's stderr goes
  to the launching terminal. Launch it from a terminal Claude can read next time.
- Phone automation helper: scratchpad a54.sh (tap-by-text via uiautomator).
  Careful: a BACK keyevent on an app root screen exits to the launcher.
- A54 Battery Saver (settings global low_power) was set to 0 for testing and
  must be restored to 1 by the user.
- Fixed a pedantic clippy warning (ip_constant) shipped in 9aa075e's tests;
  earlier clippy run predated those tests.

## 2026-09-16T18:19:31Z - Claude Opus 5 - Block 52.1 root-caused and fixed: silence guard evicted listeners waiting for Play
- Not a GUI bug. Core host's peer_inbound_silence_timeout (8 s) applied from
  registration; listeners only probe once a stream starts; the guard runs lazily
  on the first recipient lookup, which is the stream start. Any listener approved
  >8 s before Play was evicted by the stream start itself.
- Proof: device harness with SILENT_DISCO_DEVICE_TEST_PLAY_DELAY_SECS=120/240
  delivered 0/7122; host TCP ESTAB -> TIME-WAIT (host closed). Fix: silence clock
  armed only by inbound sync/audio datagram or delivered StreamStart, disarmed by
  delivered Stop. After fix, 60 s hold: 7115/7123 delivered, heard by user.
- Harness additions (test-only env vars): SILENT_DISCO_DEVICE_TEST_SOURCE=<file>
  streams a real file; SILENT_DISCO_DEVICE_TEST_PLAY_DELAY_SECS holds before Play.
  Harness now asserts frames_fully_delivered > 0.
- The user's 16 kHz mono 66-min MP3 streams fine; file format was ruled out.
- Diagnostic technique that settled direction: sample State  Recv-Q Send-Q      Local Address:Port     Peer Address:PortProcess
LISTEN 0      4096              0.0.0.0:5432          0.0.0.0:*          
LISTEN 0      128         100.64.64.132:43217         0.0.0.0:*          
LISTEN 0      64                0.0.0.0:46309         0.0.0.0:*          
LISTEN 0      4096              0.0.0.0:54305         0.0.0.0:*          
LISTEN 0      511             127.0.0.1:6463          0.0.0.0:*          
LISTEN 0      4096            127.0.0.1:8008          0.0.0.0:*          
LISTEN 0      128               0.0.0.0:22            0.0.0.0:*          
LISTEN 0      4096              0.0.0.0:111           0.0.0.0:*          
LISTEN 0      4096            127.0.0.1:11434         0.0.0.0:*          
LISTEN 0      4096              0.0.0.0:48391         0.0.0.0:*          
LISTEN 0      4096            127.0.0.1:5002          0.0.0.0:*          
LISTEN 0      128             127.0.0.1:5037          0.0.0.0:*          
LISTEN 0      128             127.0.0.1:631           0.0.0.0:*          
LISTEN 0      4096              0.0.0.0:35083         0.0.0.0:*          
LISTEN 0      64                0.0.0.0:2049          0.0.0.0:*          
LISTEN 0      4096              0.0.0.0:39025         0.0.0.0:*          
LISTEN 0      4096        127.0.0.53%lo:53            0.0.0.0:*          
ESTAB  0      0           100.64.64.132:47322  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:41944  57.144.220.141:443        
ESTAB  0      0           100.64.64.132:49090    104.18.32.47:443        
ESTAB  0      0           100.64.64.132:54106     3.17.45.182:443        
ESTAB  0      0           100.64.64.132:41802   35.186.224.36:443        
ESTAB  0      0           100.64.64.132:40452   160.79.104.10:443        
ESTAB  0      0           100.64.64.132:33836     3.17.45.182:443        
ESTAB  0      0           100.64.64.132:38994  57.144.220.145:443        
ESTAB  0      0           100.64.64.132:33586  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:43122  57.144.221.134:443        
ESTAB  0      0           100.64.64.132:39868  57.144.220.145:443        
ESTAB  0      0           100.64.64.132:43710  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:42764   35.186.224.31:443        
ESTAB  0      0           100.64.64.132:34020  57.144.220.145:443        
ESTAB  0      0           100.64.64.132:54278   140.82.112.26:443        
ESTAB  0      0           100.64.64.132:34476  74.125.137.188:5228       
ESTAB  0      0           100.64.64.132:60728   34.149.66.165:443        
ESTAB  0      0           100.64.64.132:40050     18.97.36.62:443        
ESTAB  0      0           100.64.64.132:52486  57.144.220.145:443        
ESTAB  0      0           100.64.64.132:53526    140.82.113.3:443        
ESTAB  0      0           100.64.64.132:44996   160.79.104.10:443        
ESTAB  0      0           100.64.64.132:41570  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:35544     3.17.45.182:443        
ESTAB  0      0           100.64.64.132:52394 162.159.130.234:443        
ESTAB  0      0           100.64.64.132:50466   160.79.104.10:443        
ESTAB  0      0           100.64.64.132:57744    104.18.39.21:443        
ESTAB  0      0           100.64.64.132:43570  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:39174   35.186.224.39:443        
ESTAB  0      0           100.64.64.132:39392  57.144.220.141:443        
ESTAB  0      0           100.64.64.132:53016  142.251.214.37:443        
ESTAB  0      0           100.64.64.132:44886  172.64.148.235:443        
ESTAB  0      0           100.64.64.132:38714  24.130.173.209:5122       
ESTAB  0      0           100.64.64.132:43166   104.20.30.124:443        
ESTAB  0      0           100.64.64.132:46162   54.190.152.57:443        
LISTEN 0      4096                 [::]:58839            [::]:*          
LISTEN 0      4096                 [::]:5432             [::]:*          
LISTEN 0      50     [::ffff:127.0.0.1]:35087               *:*          
LISTEN 0      50                      *:1716                *:*          
LISTEN 0      4096                 [::]:57625            [::]:*          
LISTEN 0      128                  [::]:22               [::]:*          
LISTEN 0      4096                 [::]:111              [::]:*          
LISTEN 0      128                 [::1]:631              [::]:*          
LISTEN 0      50     [::ffff:127.0.0.1]:33287               *:*          
LISTEN 0      64                   [::]:40027            [::]:*          
LISTEN 0      4096                 [::]:56967            [::]:*          
LISTEN 0      4096                 [::]:60931            [::]:*          
LISTEN 0      64                   [::]:2049             [::]:*           on the
  host port every 0.5 s; host ending in TIME-WAIT means host sent the first FIN.
- Open: re-verify via packaged GUI; guard still armed during pause; 8/7123
  frames reported partially delivered with one listener.

## 2026-09-16T18:33:54Z - Claude Opus 5 - Block 52.1 verified through the packaged GUI
- Package from 4041b95 (deb sha256 4d688994..., binary 4daa0afb...) installed
  and running; A54 joined Test3 via Connect manually; user approved, waited as
  asked, pressed Play, and heard audio. Phone "Playing", host receiving sync
  traffic, no desktop errors. Gap not independently timed on this path.
- Useful: launch the GUI as `silent-disco-desktop 2>&1 | tee ~/silent-disco-desktop.log`
  so its output is readable afterwards.
- Earlier today the user restarted the GUI before installing the new .deb;
  always confirm /usr/bin/silent-disco-desktop matches the new bundle's binary.

## 2026-09-16T19:17:03Z - Claude Opus 5 - Blocks 52.2/52.3 device-verified; 52.6 found and fixed; mDNS path plays end to end
- 52.3 (f58b1ab): joining stops discovery and MdnsDiscoveryService.stopDiscovery()
  clears its list; Android reconciled anyway and expired the joined session.
  Core now ignores expiry of a joined session (diagnostic
  session_expiry_ignored_while_joined); Android reconciles only while the core
  is discovering and resubmits on discovery start.
- 52.6 (uncommitted at time of writing): the mDNS/discovery listener path never
  started the sync probe loop, so it had never played audio from a desktop host
  (packets "dropped before lock", then evicted by the 52.1 guard). Now started
  when the playback runtime starts; the loop's Wi-Fi Direct liveness check is
  scoped to sessions that ride Wi-Fi Direct (listenerTransportRidesWifiDirect).
- Device run 12:15-12:16 (Test5, mDNS Join, manual approval, ~42 s wait):
  sync locked 6 ms after first probe, RTT 5-8 ms, "Playing in sync", heard.
  First ~1 s lost to pre-lock drops/underruns (follow-up: probe from approval).
- The discovery-path checklist marks "Waiting for host approval" complete and
  shows "Connecting" with a spinner while actually approved and waiting for
  Play -- user read it as stuck (52.5).

## 2026-09-16T19:29:44Z - Claude Opus 5 - Block 52.4: a refused effect silently killed core notification delivery
- Root cause: desktop runners return an error from the core observer when they
  cannot accept an effect; run_notification_worker exited on any observer error,
  so all later effects (e.g. join approvals) were silently never delivered and
  current_snapshot() began failing. Now a refused platform/transport/storage
  effect is fed back to the actor as that operation's failure and delivery
  continues. tests/refused_effect.rs locks both cases.
- Desktop: accepted join decisions/removals time out after 15 s
  (DELIVERY_CONFIRMATION_TIMEOUT_MS) into a retryable visible failure.
- LabScreen.test.tsx link-fault test is flaky: 1 of 3 isolated runs failed.

## 2026-09-16T20:00:58Z - Claude Opus 5 - Block 52.5: misleading delivery banner and "connecting" after approval
- Desktop: "Last delivery" was the last *control* message, not audio. Now labeled
  "Last control message (approval, rejection, or removal)"; per-stream audio frame
  counts shown separately; red "Streaming to nobody" alert when buffering/playing
  with 0 connected listeners (DeliveryAndTransportAlerts.test.tsx).
- Desktop listener list shows listenerProgress() (domain.ts) instead of the raw core
  `connecting`, which the core never advances past for an approved listener.
- Android: APPROVED_OR_LATER wrongly contained CONNECTING (reached before the host
  replies), so "Waiting for host approval" ticked before approval. Removed; join
  checklist now follows core order and adds "Waiting for the host to start playback".
  MainViewModelDiscoveryReconcileTest fails with the old set.
- Gates: Android `./gradlew test lintDebug` 966 runs, 0 failures; desktop
  `npm run check` 114 tests. Not device-verified (needs new APK + desktop package).
- Also fixed TODO layout: Blocks 51-52 now sit after Block 50, before "Future phases".

## 2026-09-16T20:26:26Z - Claude Opus 5 - Block 52.5 fixes device-verified
- Build 9a8767f (desktop binary 5b510f36, APK 55859964), A54 over CGNAT, mDNS join,
  session Test6: Android join steps and desktop listener wording correct before and
  after approval; audio played in sync; after the phone left mid-stream the desktop
  showed "Streaming to nobody" plus the per-stream frame line.
- Follow-ups recorded under 52.5: possible ~1 min approval send delay (unconfirmed,
  click time unrecorded), 11 "partial" frames with one listener, frame counter runs
  ~200/s (unit unclear), stale "Playback start requested." banner, jargon empty text.

## 2026-09-16T20:32:59Z - Claude Opus 5 - Block 52.5: broadcast counter partial/undelivered split
- record_delivery counted frames that reached none of their recipients as
  "partial"; new frames_undelivered counter/DTO field separates them. Explains the
  "11 partially" seen with a single listener and the older 52.1 anomaly.
- Counter rate of ~200/s is correct: one count per broadcast packet, audio packets
  are 5 ms (DEFAULT_PACKET_DURATION_MS), counters are per host session. The UI
  reports local broadcast send-call outcomes, not confirmed listener receipt, with
  five distinct outcomes.
- Gates: desktop npm run check (115 tests), clippy --all-targets -D warnings,
  desktop cargo test 233 passed. No core changes. Not device-verified.

## 2026-09-16T20:35:45Z - Claude Opus 5 - Session handoff written
- Handoff for the next session: docs/CLAUDE_CODE_HANDOFF_2026-09-16_BLOCK_52.md
  (status of Blocks 49-52, open items, release path, how to work on arisu + A54).
  CLAUDE.md Ralph Loop section now points at it.
- 52.1 "8 of 7123 partial" item marked explained by ea0dd37.

## 2026-10-13 — Desktop host TODO continuation (Blocks 4, 23, 26, 49–52)
- Repaired the strict all-target desktop Rust build: both `StoredSettings`
  literals in `performance_probe.rs` now supply the shared-address-space field.
  `cargo check --locked --all-features`, strict Clippy, and desktop Rust tests
  passed (330 passed, 4 device tests ignored, plus integration tests).
- Fixed playback control enqueue failure handling. Pause/resume now compensate
  the actor transition if the control frame cannot enter the outbound queue;
  resume commits its offset and unpauses the pump only after enqueue succeeds.
- Shared-address-space changes validate against the live network policy before
  persistence and restore the previous policy on storage read/write failure.
  Updated Block 51.3 to reflect the actual transaction behavior.
- Host Session refresh now ignores non-newer snapshots. Accepted end-session
  requests clear the pending UI state when a newer core revision reports the
  lifecycle left `ending_session`. Disappearing requests/listeners are no longer
  automatically announced as delivered; ambiguous cases surface a retryable
  confirmation error.
- Connected core error entries now include an observation time and stable ID;
  Host Setup renders the time and permits dismissal. Session-name validation is
  shown after the field is touched. mDNS status is displayed beside the active
  host address. Playback request announcements clear when a newer snapshot
  confirms the requested state; empty-listener text is now plain language.
- Android Block 52.6 follow-up: pre-runtime sync probes can now use an active
  mDNS/manual endpoint and begin after admission, before the host starts audio.
  Added a JVM test for a probe at approval. Physical A54 verification is still
  pending.
- Checks run: `desktop/npm run check` passed (118 frontend tests, production
  build); `desktop/src-tauri cargo clippy --locked --all-targets --all-features
  -- -D warnings` and `cargo test --locked --all-features` passed; Android
  `./gradlew test lintDebug --stacktrace --console=plain` passed (323 unit tests
  for each Debug, PocDebug, and Release variant; lint clean).
- Preliminary single-run Block 45 probe on Linux x86_64 / 16 logical CPUs:
  WAV/FLAC decoder 12M frames/s each, MP3 30.05M frames/s, packetizer 176.5k
  packets/s with queue high-water 11/32 and no backpressure; virtual transport
  delivered 500 packets to 1, 2, 5, and 16 listeners without loss; 50-permille
  configured loss measured 52/52 permille at 5/16 listeners; DB reads averaged
  13 us and writes 2,023 us. Raw JSON was written to
  `/tmp/silent-disco-performance-20261013.json`; this is one run, without soak,
  so no performance limits are claimed.

## 2026-10-13 — Block 45 paired performance evidence
- Ran 3 paired release baselines and a 60 s 16-listener soak on `kawa`, Ubuntu
  24.04.4, x86_64, AMD Ryzen 7 5825U, 16 logical CPUs; `CLK_TCK=100`.
- Decoder real-time multiples: WAV 222–250x, FLAC 250x, MP3 626x. Packetizer
  171,428–176,470 packets/s. Transport peer-delivery throughput medians:
  55,555/s (1 listener), 111,111/s (2), 227,272/s (5), 421,052/s (16),
  208,333/s (5 with 50‰ loss), and 421,052/s (16 with 50‰ loss).
- Runtime queue peak 196–198 / 256, no overflow; notification backlog 49 / 64.
  Monitor callback average 390–429 ns; max 1.0–11.1 us; one ring underrun.
  Runtime synchronization reported Excellent confidence, 12 ms offset, 20 ms RTT.
  Core scheduler concealed one packet without rebuffering.
- Soak: 60.86 s wall time, 1.97 s user CPU + 0.15 s system CPU (3%), max RSS
  37,100 KiB, RSS at end 8,428 KiB, 2,774 packets, 16 listeners, configured
  25‰ loss / observed 24‰, 0 transport send failures. Database read median
  16 us (15–16); writes median 2,010 us (2,002–5,658), preserving the outlier.
  Transport shutdown median 19,342 us (18,340–19,512).
- Repeated JSON, runtime JSON, soak JSON, and `/usr/bin/time -v` artifacts live
  at `/tmp/silent-disco-block45-20261013` for this session. Do not set platform
  support limits from this single machine; UI rendered-update rate remains
  unmeasured. Block 45.3's listener limit, cadence selection, and stable timing
  thresholds remain open.

## 2026-10-13 — Final local gate rerun and A54 instrumentation
- Android `./gradlew test lintDebug --stacktrace --console=plain` passed after
  the final Kotlin and instrumentation-test edits.
- Connected instrumentation passed on Samsung SM-A546E / Android 16: 98 tests
  completed, 0 failed; the legacy Bluetooth permission test was skipped because
  that manifest permission is capped at API 30.
- Desktop `npm run check` passed: generated bindings, format, lint, typecheck,
  121 frontend tests, and production build. Biome printed one informational
  configuration deprecation (`linter.recommended`); there were no lint errors.
- `bash scripts/check-rust.sh` passed: shared Rust formatting, strict Clippy,
  and workspace tests. Desktop Rust `cargo check --locked --all-features`,
  strict Clippy, and tests passed (330 passed, 4 ignored).
- `npm run tauri build` produced both AppImage and `.deb` packages; the
  repository's `verify-linux-bundles.py` passed, confirming the package set,
  desktop entry/icon, declared dependencies, and exclusion of performance probes.
- Android now renders opaque 64-hex host identities as `Host <8 hex chars>` in
  listener-facing UI, preserving the complete identity for protocol and trust
  operations. `./gradlew test lintDebug` passed again, and connected tests on
  the A54 passed again (98 completed, one legacy-permission test skipped on
  API 36).
- These local gates do not satisfy Block 46.3/47, which require a clean
  supported graphical Linux machine and physical listener interoperability
  evidence.
- The historical `LabScreen.test.tsx` flake (one failure in three isolated
  runs on 2026-09-16) did not reproduce in ten isolated reruns or the complete
  frontend suite. No related code changed, so leave it tracked for CI rather
  than claiming a root-cause fix.
- Block 24 firewall configuration remains uninspected: `sudo -n ufw status
  verbose` reports that a password is required.
- Repository branch is `master`; the implementation and evidence commits were
  pushed to `origin/master` (latest at this entry: `8f045cc`).
- Remaining work requires either lab resources or product direction: a second
  physical Android listener for Blocks 29/47; a clean Ubuntu graphical VM for
  Block 46.3; profile identity, listener/cadence limits, UI rendered update
  rate, and stable performance thresholds. Block 47 additionally requires the
  real host/join/audio/disruption matrix and exact package/APK/network evidence.

## 2026-10-13 — A54 long-pause acceptance
- Ran `SILENT_DISCO_DEVICE_TEST_ADB_SERIAL=R5CW31AX4FL
  SILENT_DISCO_DEVICE_TEST_PAUSE_SECS=120 cargo +1.97.1 test --locked
  --manifest-path desktop/src-tauri/Cargo.toml --lib
  manual_real_android_listener_plays_a_song_change -- --ignored --nocapture`
  against Samsung SM-A546E / Android 16 on `ElephantStone` (`192.168.87.0/24`).
- The test passed. During a 120-second pause the listener remained in the
  authoritative host snapshot and playback remained paused. After resume the
  host reported Excellent sync confidence and 9.67 ms RTT; all 7,121 first-song
  packets reached the listener, and the song-change run ended with 15,128
  fully delivered packets in the session. The operator confirmed the scale
  notes were audible after resume.
- The manual harness now supports configurable pause duration and optional
  ADB-driven connection. Desktop Rust formatting, strict Clippy, and the full
  locked all-feature suite passed (330 tests passed, 5 manual-device tests
  ignored). The broader two-listener and packaged acceptance matrix remains
  open.
- Implemented the Block 52.5 approval-timing follow-up: the Tauri approval
  command logs receipt time and the transport worker logs send-start time,
  correlated by the accepted operation ID. Formatting, strict Clippy, and the
  full desktop all-feature suite passed again (330 passed, 5 ignored).

## 2026-10-13 — Profile identity and mixed-device follow-up
- Implemented Block 50 profile identity as a startup-selected profile ID. The
  preference defaults to `main`, validates the backend's path-safe ID format,
  and Settings explains the restart needed to close the current profile first.
  `npm run check` passed with 136 frontend tests; `cargo +1.97.1 fmt --all
  -- --check`, strict Clippy, and the full locked all-feature desktop suite
  passed (330 passed, 5 ignored).
- Made the two-listener manual test accept configurable ADB serials and reject
  invalid payload entry before tapping Connect. Small escaped input chunks
  fixed a dropped `n` in `protocolVersion` on emulator-5554.
- Mixed run with A54 R5CW31AX4FL and emulator-5554: both distinct IDs joined
  and were approved. Host reported Excellent sync, 8,120 attempted and fully
  delivered packets, then one listener disappeared. Emulator log showed
  `received=0 accepted=0` plus virtual audio I/O errors. Treat the run as
  inconclusive; two physical listeners and per-device diagnostics are needed
  before Block 29/47 can pass.

## 2026-10-13 — Diagnostics UI commit cadence
- Extended `DiagnosticsScreen.test.tsx` with React Profiler measurement. Across
  three separately advanced two-second poll periods, each diagnostics refresh
  produces at least one committed render (0.5 Hz state-driven updates in the
  controlled jsdom harness). This closes Block 45.2's component update cadence
  measurement; it does not measure native paint time or frame cost, which still
  needs a graphical-machine profile.

## 2026-10-13 — TODO acceptance audit without second Android device
- Reconciled stale Blocks 34, 39, 40, 41, and 42 acceptance prose with their
  implemented code and tests. In particular, receiver-side raw-byte decoding,
  reconnect delays, live link mutation, packet-hash/fault traces, live monitor
  errors, and production Lab tree-shaking were already implemented but still
  described as open. Marked the final shutdown and Lab fault/replay checklist
  items complete based on their detailed checked sections and all-feature test
  results.
- Confirmed static UFW configuration is `ENABLED=no`, but runtime firewall
  state remains unverified because both `ufw status` and nftables inspection
  require privileges unavailable in this session.
- Remaining unchecked work now reflects physical-device/clean-VM evidence,
  runtime firewall inspection, historical flake monitoring, and performance
  limit decisions that are not justified by the available single-machine data.

## 2026-10-13 — Diagnostics cadence decision
- Retained the 2,000 ms diagnostics refresh interval based on the observed
  one-commit-per-poll result (0.5 Hz) in the controlled Profiler test. The
  screen presents status summaries rather than a real-time control path, and
  its 5 s stale threshold tolerates a delayed poll. Listener ceilings and
  performance regression thresholds remain unset until repeated, broader
  machine data supports them.

## 2026-10-13 — Stabilize the Lab fault-editor test
- The historical `LabScreen.test.tsx` failure was reproduced: its numeric
  latency editor submitted `30125` after clearing `30` and typing `125`.
  Changed the mock to return current state instead of consuming one-shot
  replies, and made the test wait for each number input to render empty and
  then the replacement before Apply.
- The focused test passed 20 isolated runs. `cd desktop && npm run check`
  passed all gates and 136 frontend tests. This is a test synchronization fix;
  no production behavior changed.

## 2026-10-13 — Ubuntu 22.04 fresh-VM attempt
- Created a clean Jammy VM, installed the locally built desktop `.deb`, and
  confirmed its SHA-256 matched the host artifact. Launch failed because the
  artifact was built on this Ubuntu 24.04 host and requires `GLIBC_2.39`, while
  Jammy provides `GLIBC_2.35`. The repository's bundle CI builds on Ubuntu
  22.04, so this does not establish a release-bundle regression.
- Profile creation, source staging, diagnostics export, reopen, and uninstall
  were not validated. The disposable VM and its SSH key were removed. Block
  46.3 remains open pending a fresh run with the Ubuntu 22.04-built artifact.
# 2026-10-14 — Partial Block 47 physical host interoperability
- On Ubuntu 24.04 x86_64, ran the release desktop binary directly as the host
  (`A54 Acceptance Oct 14`, `wlo1`, `192.168.87.161`) with a physical Samsung
  Galaxy A54 (`SM-A546E`, Android 16/API 36, `192.168.87.215`) over the same
  `192.168.87.0/24` Wi-Fi LAN. mDNS discovery and host approval worked. A
  generated 60-second WAV played in sync; the A54 reflected pause, resume, and
  stop. On a later fresh join, the A54 reached “Waiting for host approval,”
  but the desktop reported `desktop.operation_confirmation_missing` and “The
  join request disappeared before its decision was confirmed”; a second
  attempt reproduced this. A rejection click was unconfirmed and does not
  count as passing. Sampled UDP delivery counters were
  2,762/2,762 and 5,185/5,185 delivered to every listener, with zero partial,
  no-listener, or failed sends in those samples. No sync offset was captured.
- Desktop `.deb` SHA-256:
  `a48d5188ae54a1d5c8d0725a3fca4b3ab2acbba56d40856b9d2289c2919a8ea4`.
  Existing Android install was version `0.1.0`/code 1; built debug APK SHA-256
  `4108123f4abe7c4e28ec5b09d733e9ee79180bf6bfd231ceac36c1b404180003` was not
  installed due to signature mismatch. Manual endpoint join was retried but
  not completed because ADB text entry strips JSON key quotes. During a
  cancel attempt Android reported Wi-Fi Direct busy; force-stop/relaunch
  restored discovery without clearing data. Host UI continued to report
  connecting/awaiting first clock sync while Android reported locked sync and
  full delivery. Full Block 47 matrix, packaged-app launch, QR, second listener,
  and remaining failure/recovery scenarios remain unverified. The host session
  was ended and the desktop process exited through the window close protocol;
  see the TODO entry for complete evidence and limitations.

## 2026-10-14 — Correlate host rejection confirmation to delivery
- Traced the physical-run false-negative to frontend reconciliation: the host
  snapshot exposed only delivery counts, so a successful rejection looked
  unconfirmed when those counts matched an earlier approval. Added the
  correlated delivery operation ID to the authoritative core/FFI/desktop
  snapshots. A first attempt incorrectly correlated the transport effect ID;
  this differs from the accepted host command ID. The final implementation
  threads the initiating reject command ID through its pending transport
  effect. The desktop confirms rejection only when that matching accepted
  command has a successful recipient delivery; it retains the unconfirmed
  error otherwise. Reconciliation now waits for the command receipt if its
  resulting snapshot arrives first.
- Added Rust actor and frontend regression coverage for matching delivery,
  unrelated earlier delivery, and receipt/snapshot ordering. Physical rerun
  on the Samsung A54 confirmed the complete rejection path: after host reject,
  the request disappeared from the desktop pending list and the phone displayed
  “The host did not approve this request.”
- Checks passed: 55 focused frontend tests, TypeScript typecheck, frontend
  focused lint, Rust host admission integration tests (4), FFI and Tauri Rust
  compilation, Rust formatting, and generated TypeScript binding check.

## 2026-10-14 — Block 47 continuation: AppImage and gate reruns
- Built the current working tree's AppImage on Ubuntu 22.04.5 amd64. The
  matching `.deb` was rebuilt after noticing the existing package predated the
  final host rejection fix. The Linux bundle verifier passed for both packages.
  An isolated Xvfb
  launch smoke kept the AppImage running for 15 seconds; portal/FUSE permission
  warnings appeared in that temporary session, so this is launch evidence
  only. AppImage SHA-256:
  `f89135036c8734dc0deb3a7f2796f14649f61fc741eab7cfca3ddc396c18899d`; `.deb`
  SHA-256: `9c2bd2833d991505b989419220dc059465be176cec0cc735ecfc7d03041c43a4`.
- `scripts/check-rust.sh` passed formatting and strict Clippy. Its all-feature
  test run stalled over six minutes at the known FFI shutdown race; the exact
  test passed alone with one test thread, and the full workspace suite then
  passed with `--test-threads=1`.
- `npm run check` passed bindings, formatting, lint, and typecheck, then failed
  in `LabScreen.test.tsx`: `edits a loaded link fault profile through the
  backend command` expected an empty numeric input but saw the prior value
  `30`. The file passed alone (11/11), while the full suite reproduced the
  failure (139/140). This failure is unrelated to host rejection.
- Only one physical Android device is attached. Block 47's runner requires two
  physical devices and would install the debug APK over the user's existing
  package; installation was not attempted because its signature differs and
  replacing it would require uninstalling user app data. Block 47 and clean
  package lifecycle acceptance remain open.

## 2026-10-14 — Packaged AppImage approval recheck
- Launched the Ubuntu 22.04.5 AppImage (`f89135036c8734dc0deb3a7f2796f14649f61fc741eab7cfca3ddc396c18899d`) in the graphical session and hosted `Test` on `192.168.87.161`.
  The A54 (`SM-A546E`, Android 16/API 36) discovered it over mDNS, sent a join
  request, and passed host approval; the packaged host announced that approval
  was confirmed by the Rust core. The phone moved beyond the approval wait but
  remained in initial sync, with the host listener card at `connecting`.
  Playback was not verified. The app window became blank after a later
  interaction. Session was closed and the phone app force-stopped without
  clearing data. This completes only Block 52.4's approval recheck.
- Block 29 remains blocked on a second physical Android device; the user
  confirmed none is available. Existing performance evidence reaches 16
  virtual listeners on one host, which is insufficient to declare a maximum
  supported physical listener count or portable timing limits.

## 2026-10-14 — Packaged AppImage playback and profile recheck
- On Ubuntu 22.04.5 amd64, launched the rebuilt AppImage (SHA-256
  `f89135036c8734dc0deb3a7f2796f14649f61fc741eab7cfca3ddc396c18899d`) and
  hosted `ResumeTest`. The A54 joined by manually pasting the copied endpoint
  after the AppImage discovery scan did not show the host; approval, sync lock,
  host playback, pause/resume, explicit stop, diagnostics export, and clean
  shutdown were observed.
- Pulled the Android recording: 58.695 s, 48 kHz, stereo, 16-bit, with
  5,632,138/5,634,720 nonzero samples. This establishes captured non-silent PCM,
  not uninterrupted audible quality. Android listener diagnostics included
  ring underruns and silence-filled frames across pause/resume. Desktop
  diagnostics reported 12,004 frames attempted, 12,003 fully delivered, one
  undelivered, no partial deliveries or queue overflows, peak queue depth 122,
  and no last error.
- Reopened profile `main`; SQLite integrity passed, but recent sessions showed
  zero after the prior host run. This gap was subsequently fixed through the
  Rust desktop platform adapter and verified through a packaged AppImage profile
  reopen; see the following dated entries.
- Focused rejection-correlation tests rerun: Rust host admission integration
  4/4 and frontend host screen/domain/App tests 55/55. Rust workspace
  `cargo check --locked --workspace` passed. `git diff --check` still reports
  generated-binding trailing whitespace; do not hand-edit generated output.


## 2026-10-14 — Desktop host session-history write path
- Added a desktop Rust adapter hook: a successful advertising start writes a
  host `SessionStart` row through the existing database worker; a successful
  advertising stop finishes that row with the final connected-listener count
  and `completed` outcome. If the initial history write fails, the adapter
  stops the transport and reports cleanup failure as well if stopping fails.
- Added a focused test that starts the real SQLite worker, begins and finishes
  a host history row, and reads back the active and terminal records. It passed.
  Tauri Rust strict Clippy, formatting, and release AppImage build passed.
- The newly built AppImage SHA-256 is
  `e24f642d97536028426b2b3983febdd3a7a4c934a8ca080ac8a86449164f6e66`. Initial
  `x11grab` window captures appeared black because they omitted the GNOME
  compositor's accelerated window output. GNOME desktop captures showed the UI
  normally; do not treat those X11 captures as a product blank-window failure.


## 2026-10-14 — Packaged host history survives profile reopen
- On the newly built Ubuntu 22.04 AppImage (`0.1.0`, SHA-256
  `e24f642d97536028426b2b3983febdd3a7a4c934a8ca080ac8a86449164f6e66`),
  created WAV-backed host session `SessionHistoryProbe` in profile `main`.
  Storage showed `host / active` during advertising, then
  `host / completed; listeners 0` after ending it. Closed the app, relaunched
  the same AppImage, reopened Storage, and saw the same completed row with
  SQLite integrity `ok`. This verifies the session-history reopen item for Block
  47. Other matrix items remain open.
- GNOME screenshot captures show the desktop UI rendered correctly. Earlier
  `x11grab` captures returned black for the accelerated/composited webview and
  were a capture-path artifact; the prior diagnosis of a blank product UI was
  incorrect.


## 2026-10-14 — Session IDs fixed for persistent host history
- The first packaged mDNS retest failed before advertising. Diagnostics named
  the cause: `UNIQUE constraint failed: session_history.session_id`. The core
  generated `session-1` on every fresh actor, but the SQLite history correctly
  retained the prior row. The failed history write rolled back advertising.
- Changed shared-core session IDs to combine Unix nanoseconds, process ID, and
  a process-wide atomic nonce. Added
  `session_ids_are_unique_across_fresh_actor_states`; targeted core test,
  formatting, and `cargo clippy --locked -p silent-disco-core --all-targets
  --all-features -- -D warnings` pass.
- Rebuilt the Ubuntu 22.04.5 AppImage (`0.1.0`, SHA-256
  `df878722e2f1f1bf1be80cb05f53e276784f9a8f964901599856e07771fb6f48`). With
  existing profile `main` and the same persistent history DB, it hosted
  `MdnSPackageProbe` on `192.168.87.161`. The Samsung Galaxy A54 (Android 16/API
  36, `192.168.87.215`; installed APK SHA-256
  `2616c6eca3236e8b13090d3006f6535c60e8a58fa18d98c8a1514f86fe7198bd`)
  discovered it by mDNS, joined, was approved, reached “Playing in sync” with
  `Clock sync locked`, and reflected desktop pause and resume. Delivery reported
  7,027/7,030 packets delivered to every listener, 0 partial, 3 no-listener,
  0 failed; queue peak 122. Stop left the phone at “Audio is out of sync” /
  “Fix connection,” and the desktop listener card still showed `connecting`
  despite the phone's locked sync/playback. The host session ended and the DB
  row completed; SQLite integrity passed. These are successful mDNS and
  one-listener synchronized-playback results, with stop handling and host
  listener lifecycle status still unresolved.

## 2026-10-14 — Connected host listener projection and gate rerun
- Updated the desktop approval effect to authorize the approved listener's
  reported sync/audio UDP ports and submit the authoritative `ListenerConnected`
  event after approval delivery. The packaged AppImage
  (`319ec73de959e508d7e7f5a3bbc0b3df9c66c203e27a8bab8be7535e6c26c52c`)
  showed the Galaxy A54 as `session_only` / core transport `connected`. Android
  reported `Clock sync locked` and `Playing in sync`; the desktop card still
  lacked its first sync summary.
- The first End Session request did not appear complete at the initial DB
  check. After returning to Host control and submitting End Session again,
  Host Setup appeared and SQLite recorded `MdnSConnectedProbe` as `completed`
  with listener count zero. Count the eventual end path as completed, while
  leaving listener-count persistence unresolved. SQLite integrity remained
  `ok`; the desktop window remains open and idle.
- Refined unique session IDs to use a per-actor namespace in production while
  deterministic Lab runtimes retain fixture IDs. The 8 focused live Lab
  transport tests passed. Full Tauri Rust tests passed (331 passed, 5 ignored);
  strict Clippy passed. `cd desktop && npm run check` passed bindings, format,
  lint, typecheck, 140 frontend tests, production build, and Lab exclusion.
- Remaining device gates include two physical Android listeners and a fresh
  supported Linux machine/VM; only one Android phone is available. Packaged
  Stop behavior and the host sync-summary display remain unresolved.
- `bash scripts/check-rust.sh` then passed Rust formatting, strict workspace
  Clippy, and all Rust workspace unit/integration tests (338 core unit tests).
  `git diff --check` reports only generated binding trailing whitespace;
  bindings are verified by the generator check and are not hand-edited.
- Firewall follow-up: `systemctl is-active ufw` reports `active`,
  `systemctl is-enabled ufw` reports `enabled`, but `/etc/ufw/ufw.conf` has
  `ENABLED=no` and `LOGLEVEL=low`. `nft list ruleset` and `iptables -S` are
  denied without root; effective rules remain unknown. Successful mDNS and
  listener packets show no observed interference, not a firewall-policy pass.

## 2026-10-14 — QR expiration and Lab editor race fixes
- A packaged UI observation showed that an invitation QR remained visible
  past its displayed expiry until some unrelated state caused a render. Added
  a timer in `QrInvitationPanel` to rerender at expiry; the QR then transitions
  to the expired state automatically. Its fake-clock regression passed.
- The same full npm gate reproduced a Lab test race: `FaultLinkEditor`'s
  initialization effect could run after the user cleared its number input and
  restore the initial prop. It now synchronizes only when backend link values
  actually change. The formerly flaky test passed alone, and the full suite
  passed after the correction.
- `cd desktop && npm run check` passed: bindings, formatting, lint, typecheck,
  141 Vitest tests, production build, and production Lab-code exclusion.
  One earlier run failed the existing Lab editor test; the same test passed
  alone, its initialization race was fixed, and the full run passed afterward.
- Rebuilt both Linux packages: AppImage SHA-256
  `d86d030a19d05ed1c05917ec40058e9788089a33b8dae1634ca5706fdf8f04fa`; `.deb`
  SHA-256 `d9cdb5d08a2a5afead67baeeb69312791995c31d5d72a68580bfff87db5d6a1f`.
  `python3 desktop/scripts/verify-linux-bundles.py --bundle-dir
  desktop/src-tauri/target/release/bundle --tauri-config
  desktop/src-tauri/tauri.conf.json --cargo-manifest
  desktop/src-tauri/Cargo.toml` passed.
  The rebuilt AppImage was subsequently launched on this host; its `Silent
  Disco` window is visible and the desktop process remains alive. Profile
  `main` passes SQLite `integrity_check`, and the latest session-history
  entries are completed with zero listeners. This confirms package launch and
  clean persisted state, but not timed QR expiry observed in the packaged UI.

## 2026-10-14 — Virtual audio endpoint removal probe
- Confirmed the user PulseAudio server had no active sink-inputs and kept its
  hardware default output unchanged. Created a temporary `module-null-sink`
  (`sd_remove_probe`, module ID 31), opened it directly with a temporary CPAL
  0.18.1 PulseAudio-host probe and a silent I16 stream, then unloaded the
  module while the stream was active. The CPAL stream error callback did not
  fire; the sink disappeared and the hardware default remained. This is
  evidence that virtual sink removal on this compatibility path does not
  reproduce a physical unplug or production ALSA-default device-loss event.
  The Block 33 physical device-removal checkbox remains open.
- Verified QR timer behavior in the packaged AppImage (`d86d030a19d05ed1c05917ec40058e9788089a33b8dae1634ca5706fdf8f04fa`):
  the `QR-Physical-Oct-14` invitation expired at 04:58:01 PDT; by 04:59:13
  PDT its QR was removed and the visible expired message appeared without
  another interaction. The A54 scanner stayed foregrounded but no scan/join
  request arrived, so physical QR acceptance remains open.

## 2026-10-14 — Recover host-history rows after desktop restart
- Rechecked the current runtime: the desktop process/window was gone, the
  Android scanner remained open, and SQLite showed `QR-Physical-Oct-14` still
  `active`. No crash record was available, so the exit cause is unknown.
- Added a Rust storage-worker transaction to fail prior active host rows as
  `desktop.host_session.interrupted` during profile startup, while preserving
  listener counts and leaving listener-role and already-terminal records
  unchanged. Added storage idempotence/isolation coverage and a profile-open
  test that seeds an active host row and verifies startup recovery. Both
  focused tests pass.
- `cargo fmt --all -- --check` and strict Tauri Clippy passed. The full Tauri
  suite ran 337 tests: 326 passed, 6 loopback transport tests failed because
  the current managed sandbox denies socket binds, and 5 were ignored.
  `npm run tauri -- build --bundles appimage,deb` completed after supplying a
  runtime extracted from cached `linuxdeploy`. The first cached runtime was too
  old for the appimagetool's Zstandard SquashFS, so that AppImage was regenerated
  with XZ. Then the standard Tauri bundler was rerun with the newer cached
  runtime. The frontend's full `npm run check` passed with 141 tests. After
  changing the diagnostics copy to describe local transport send outcomes
  across audio, sync, and control frames, the final AppImage and `.deb` were
  rebuilt. Current AppImage SHA-256
  `5c8bd0b7454b7fe0c57c6aac35cbf8f6d78360638738a9eac16486176c1da8fb`; `.deb`
  SHA-256 `d7685ae9b98336cbab7c1789c03926b487ae2d959b97b4d3c1c8e2317470402f`.
  The bundle verifier passed for both; AppImage extraction passed, and its
  binary contains `host_transport_send_failure` and
  `datagram_failure_categories`.
  Direct graphical launch and Xvfb
  remain blocked by denied display socket access, so the new startup recovery
  has focused tests but no package runtime observation yet.
- Revisited Block 52.5's 11 frames reported after the A54 left mid-stream.
  The historical counter combined audio/sync datagrams with control sends and
  did not preserve the channel or error kind. Local UDP send success does not
  prove Android received the datagram.
  Updated the desktop label and regression test to say that explicitly. The
  focused Vitest file passes (4 tests). Whether the 11 send failures align with
  listener departure/eviction remains unverified without per-packet timing and
  listener-side sequence correlation. Added typed shared-core datagram failure
  counts (`WouldBlock`, other I/O, partial write) and failure-only desktop
  transport stderr records with frame kind/sequence, monotonic time, local send
  outcomes, and those categories, without peer addresses or payload. The two
  focused category tests and pinned-core strict Clippy pass; a physical run is
  still needed to correlate.
- Retried the Block 24 firewall inspection with `sudo -n nft list ruleset`,
  `sudo -n iptables -S`, and `sudo -n ufw status verbose`. All are blocked by
  this container's `no new privileges` restriction and `/etc/sudo.conf` owner
  mismatch; effective host firewall rules remain a genuine privileged-host
  check.
- Reran the complete Tauri gates after the transport diagnostics change:
  formatting, strict all-target/all-feature Clippy, and `cargo check --locked
  --all-targets --all-features` pass. The latest
  `cargo test --locked --all-features` reports 327 passed, 6 socket-bind
  failures, and 5 ignored physical-device tests. `bash scripts/check-rust.sh`
  still has an environment-limited failure: 333 shared-core tests pass, while
  8 socket integration tests cannot bind TCP (`Operation not permitted`).
- Confirmed the managed runner cannot reach the host desktop/device surfaces:
  `xdpyinfo` cannot open display `:0`; ADB daemon startup fails binding its
  local server socket with `Operation not permitted`; `/dev/kvm` is absent.
  One later ADB session did list the SM-A546E and returned Android 16 from
  `getprop`, but subsequent shell commands again could not reconnect. This
  intermittent access does not provide scanner/UI control. Physical-device
  and clean-VM acceptance cannot be exercised from this runner.
- Repeated Block 45 release performance evidence on 2026-10-14. Built
  `performance_probe` and `block45_runtime_probe` with Rust 1.97.1 in release
  mode, then collected three baseline/runtime pairs and a 60 s 16-virtual-
  listener soak on Ubuntu 22.04.5 x86_64, Ryzen 5 5500U / 12 logical CPUs.
  The analyzer passed. Zero-loss transport at 1/2/5/16 listeners had 0‰
  observed loss; configured 50‰ at 5/16 measured 52‰; all had zero transport
  send failures. The soak configured 25‰, measured 24‰, had zero send
  failures, 37,248 KiB maximum RSS, 8,544 KiB ending RSS, 2.68 s user CPU over
  61.32 s elapsed. Across runtime runs, the five-listener desktop path
  delivered 1,000/1,000, had zero queue overflows, transport peak depth
  193–197/256 and notification backlog 49/64. Raw JSON, summary, environment,
  and `/usr/bin/time -v` records: `/tmp/silent-disco-block45-20261014-repeat`.
  These repeated one-host virtual measurements strengthen scaling evidence but
  still do not justify a physical supported-listener maximum; Block 45.3 stays
  open pending physical listener runs.
- Block 31 QR readability follow-up: the saved packaged UI screenshot showed a
  728-character invite rendered around 248 px wide; OpenCV could not decode it
  at that size. QR generation now requests 1024 source pixels and the image
  displays responsively up to 672 px. Re-rendering the exact saved invite with
  those options decoded in OpenCV at 640 px. This is only an offline image
  check; physical Android scan/join remains unverified. The focused host-screen
  suite passes 27/27, and `npm run check` passes 141 frontend tests plus the
  full frontend gates. Rebuilt AppImage and `.deb` pass the repository bundle
  verifier; AppImage SHA-256
  `63929088fe1a2e5edda74231a07fda2f0d9ef5be932da0cec8e9bef1767084ca`, `.deb`
  SHA-256 `90fb149300ba1a6278fb4ba56635f99a9b47a554cc89a3375016eb82e8041b77`.
- A54 ADB recovery on 2026-10-14: Linux enumerated the SM-A546E as USB
  `04e8:6860` with an ADB interface, and the host device node granted `phil`
  read/write access. Three ADB servers were running (default TCP and two custom
  sockets); the original capture helper's server restarts made the checked
  endpoint intermittent. The helper now prefers the existing default server
  and leaves servers running. At 16:27 PDT it listed `R5CW31AX4FL device`,
  queried Android API 36 and installed app 0.1.0/versionCode 1, and saved a
  valid 1080×2340 screen capture of Android USB settings with file transfer
  selected. A second run used `--launch-app`, opened the app to its home screen,
  and confirmed the installed APK SHA-256 is
  `2616c6eca3236e8b13090d3006f6535c60e8a58fa18d98c8a1514f86fe7198bd`.
  Direct GUI launch from the managed runner fails because FUSE is absent and
  GTK cannot initialize in its display namespace. Prepared the ignored host
  launcher `desktop/src-tauri/target/start-qr-acceptance-host.sh`, which uses
  AppImage extract-and-run, avoids duplicate launches, and points Host Setup
  to a generated 60-second 440 Hz WAV at `/tmp/silent-disco-acceptance-tone-60s.wav`.
  Physical QR scan/join remains open pending host-window interaction.

## 2026-10-14 — Confirmed automation permission regression
- The same conversation's rollout records `danger-full-access` with approval
  policy `never` at 01:20:15 PDT, then `workspace-write` with network disabled
  and approval policy `never` at 09:04:28 PDT. Evidence: session
  `01a1397f-4b28-7c31-a09b-a5f75585d14b`, turn-context records at lines 8 and
  12388. Earlier desktop automation was real; the execution permissions changed.
- The apparent intermittent ADB access was caused by command invocation.
  Existing `~/.codex/rules/default.rules` permits direct `adb` commands.
  Shell redirection and Python/bash wrappers do not inherit that command rule.
  Direct `adb shell screencap -p /sdcard/silent-disco-astra.png`, followed by
  direct `adb pull`, worked repeatedly. A direct Back key event worked too.
  Fresh evidence is `desktop/src-tauri/target/a54-astra.png`. Use direct ADB
  commands, with phone-side capture followed by pull; do not restart servers
  or ask the user to change cables based on a sandboxed wrapper's failure.
- Desktop X11, Wayland, and session D-Bus connections remain denied under the
  current policy; no desktop-control tool or saved GUI-control command rule
  is exposed. Full desktop/A54 acceptance remains incomplete. Restoring the
  previous execution permissions is an application/session setting outside
  this agent's current authority; do not edit permission rules to bypass it.

## 2026-10-14 — Packaged desktop QR join and playback retest
- Current session permissions allowed direct ADB and GUI automation, unlike
  the earlier restricted run described above. A fresh signed QR invitation
  from the running packaged AppImage was scanned with the A54. The listener
  connected, the host approved it, and the desktop showed “Connected — sync
  excellent.” Starting `/tmp/silent-disco-acceptance-tone-60s.wav` through the
  host Play button made the A54 show “Playing.”
- At desktop playback time 0:07, broadcast counters showed 1,526/1,530 frames
  delivered to every listener, 4 to none, 0 partial sends, 0 failed sends,
  0 no-listener sends; queue depth returned to zero and peak was 122. Numeric
  synchronization offset and physical audibility were not measured. The
  desktop diagnostics also displayed `Transport worker: control transport
  Unauthorized: control frame was not sent by the authorized peer identity`
  despite the approved/connected listener, sync-excellent status, and active
  playback; investigate this signal before marking diagnostics clean.
- Updated Block 47's QR checkbox and acceptance evidence. Remaining physical
  gaps include end-while-connected teardown, a second listener, reconnect,
  interface/source/monitor failures, desktop restart, diagnostics export,
  clean shutdown, and numeric sync measurement. Artifacts from the run are
  under `desktop/src-tauri/target/` (`phone-check.png`,
  `desktop-after-play.png`).
- At 23:57 PDT, stopped playback and ended the host with the A54 still
  connected. Desktop returned to Host Setup; Android remained on the
  connection screen with `Host disconnected` and
  `runtime transport ShuttingDown: transport event channel is closed`; no
  Wi-Fi Direct BUSY warning occurred. After creating a fresh session, A54
  rediscovered it over mDNS and requested/received host approval. Desktop then
  showed the listener connected but sync poor; Android displayed a playback
  problem during its connection flow. Do not count graceful teardown or
  reconnect as passed. Desktop listener diagnostics captured poor confidence,
  estimated offset `-15,100,405.5 ms`, RTT `93 ms`, drift `0 ppm`, and last
  contact `Not observed`. This is a failed sync measurement, not acceptable
  alignment. Updated the TODO's Block 47 and 52.8 notes.

## 2026-10-15 — Android retry clock reset and graceful close follow-up
- A54 logs on the failed reconnect showed an accepted sync sample with
  `acquisitionElapsedMs=187224`, followed by `Unable to establish a stable
  sync estimate`. The estimator and UI mapping remained from the previous
  connection. Added `resetListenerSyncAcquisition()` and call it on sync
  failure, transport failure/disconnect, explicit leave, and retry. It closes
  the Rust estimator, clears pending correlation, and drops the old offset and
  lock state while preserving the aggregate resync count.
- Added a regression test using the measured old offset
  `-15,100,405.5 ms`; `./gradlew testDebugUnitTest` passed. `./gradlew lintDebug
  assembleDebug` passed. Then `./gradlew testDebugUnitTest lintDebug
  assembleDebug` passed after the close-path update. Android tooling emitted
  an SDK XML v4 compatibility warning from installed CMake/SDK tools, but the
  Gradle tasks completed successfully.
- Physical check using the sync-reset build: A54 rejoined the running packaged
  AppImage via mDNS and was approved. Desktop showed “Connected — sync good,”
  Android showed “Playing in sync,” and host Stop appeared as “Playback
  stopped.” The 60-second test WAV ran to 0:56 before Stop. No numeric sync
  offset was measured on that pass, though the prior failed-attempt offset was
  recorded above.
- Reproduced host End after Stop: Android showed a workflow failure even
  though no Wi-Fi Direct BUSY warning appeared. The transport event identified
  a clean peer close followed by event-channel shutdown. Changed explicit host
  disconnect and clean peer close handling to call `leaveSession()`; errorful
  connection closes still report a recoverable transport failure. Added a JVM
  test for the clean-close routing and included estimator reset in session
  leave.
- Rebuilt and installed APK SHA-256:
  `e68295247226d2e5f5107646ce25e5340dfac338592551ef53b1bc479b9b1957`. After
  that install, the A54 reached “Waiting for host approval” on the new
  `QR Probe` session. The request was below the desktop viewport; scrolling
  down revealed it, and approval succeeded. In a fresh physical retest, the
  A54 reached “Playing in sync.” Ending the host stopped desktop playback;
  Android showed “Playback stopped” without a workflow or Wi-Fi Direct BUSY
  error, but remained on the host-monitor page after 40 seconds. The host
  control port was still listening during that interval. A second End action
  closed the port and Android returned home with `QR Probe` in Recent, without
  an error. An earlier manual Android Leave also returned home cleanly. This
  demonstrates clean close after retry, but one-action teardown remains open.
  The sole physical A54 is insufficient for the two-listener matrix.
- Investigated the retry: the first queued End left the desktop control port
  open while the listener's sync probes continued. A second End closed the
  port and Android returned home/Recent without an error. In the frontend, End
  pending previously cleared on any newer host lifecycle other than
  `ending_session`; playback shutdown can publish intermediate `ready` before
  host stop completes. Tightened completion to wait for `idle` or `error`, and
  added a view-model test for `ready` followed by `idle`. The focused screen
  suite passed (28 tests); full `npm run check` passed (142 tests, bindings,
  format, lint, typecheck, and production build). Built AppImage SHA-256:
  `6d8b8046d50e1359d16618e5746afe8044de48db9e7fc2319f3a61dcab92278f`. The
  updated one-action physical behavior still needs revalidation.

## 2026-10-15 — second host End retest and Android event-loop cleanup

- The latest physical host End retest was not a pass. On the first session,
  one End closed the desktop control port and returned the desktop to Host
  Setup, but Android remained on the host-disconnected screen with
  `runtime transport ShuttingDown: transport event channel is closed` until
  Back was pressed. On a fresh mDNS session with the newly rebuilt Android
  APK, playback and sync were healthy (`Playing in sync`, desktop sync
  excellent, all 1,252 observed audio frames delivered), then one End stopped
  Android playback but left the desktop advertising with worker running and
  control port 34603 open for more than a minute. The desktop process had to
  be terminated. Android then reported a listener-workflow error; explicit
  Leave returned it to home. The AppImage is not running and no host ports
  remain open. Do not mark Block 47 or 52.8 graceful end complete.
- While investigating the event-channel shutdown path, found that
  `ManualListenerTransportController` stopped playback after a post-connection
  poll failure but kept the listener handle open. This also retained the
  standard-IP `networkSessionLock`, so a later manual reconnect could remain
  blocked. It now closes the handle before publishing the disconnected state
  and reports cleanup errors as `Failed` instead of claiming a clean
  disconnect. Added `postConnectionFailureReportsCleanupFailureInsteadOfClaimingCleanDisconnect`.
- `./gradlew testDebugUnitTest lintDebug assembleDebug --stacktrace
  --console=plain` passed (Gradle `BUILD SUCCESSFUL`; SDK XML v4 compatibility
  warning from installed Android tools). Installed APK SHA-256:
  `c0a310862de48108b9ca549bf02ae5ba03a2a3f3074232d9fa1113b6c49955d7`.
  Packaged desktop AppImage used for acceptance SHA-256:
  `cddf011fe8a230142f50ec57fcb4ce9d853da8e9a0f14763ac921bd3ad525589`.
- The desktop `stop_host_inner` change releases the network-state mutex before
  joining the playback pump, with a real-LAN regression test. That prevents
  the confirmed lock/join deadlock in the Rust test path, but the physical
  teardown failure above proves it is not sufficient to close the end-to-end
  issue. More instrumented diagnosis and a fresh one-action physical retest are
  still required. Other acceptance gaps remain: a second physical listener,
  firewall runtime policy verification, and full clean-VM validation need
  hardware/system access not available in the current setup.
- Follow-up diagnostic attempt: the focused real-LAN `ending_host_during_playback_does_not_hold_network_lock_while_joining` test passed again. A temporary AppImage with shutdown-stage `eprintln!` markers was built (SHA-256 `88b7e69f1650505ceb0b49fe15957cb4c245ea24f9605910947091f7d2a90614`), but GUI automation drifted into Settings after the source-file picker closed, before a session was created. It yielded no shutdown-stage evidence. The temporary source markers were removed; the packaged artifact retains them and must not be used for acceptance. That app was terminated cleanly while no host session was active; no host ports remain open.
- After removing the temporary markers, rebuilt the clean-source release AppImage. `cargo fmt --all -- --check` and the production frontend/Lab-exclusion build passed; Tauri reported `Finished 1 bundle`. Clean AppImage SHA-256 is `165fa88baeae18ddfc2167b63dcc282f88dd5baea6d1c4d0c3017af5ba4fddd7`. It was not exercised on device. The desktop app is closed and no host ports are listening.

## 2026-10-15 — discovered-session close mapping follow-up
- Device logcat during the host End retest showed `runtime transport ShuttingDown: transport event channel is closed`. In the discovered-session controller, a thrown `FfiListenerTransportException.Closed` had been forwarded as `ConnectionClosed(message = error.message)`, which entered `transportFailed(...)`; the existing event handler only routes a null-message `ConnectionClosed` through normal `leaveSession()` cleanup. Changed the poll-exception mapping so typed `Closed` becomes a normal close (`message = null`), while other poll exceptions retain their diagnostic. Added tests for both cases.
- `./gradlew testDebugUnitTest --tests com.ekkus.silentdisco.core.rust.ListenerTransportControllerLifecycleTest --stacktrace --console=plain` passed. Full `./gradlew testDebugUnitTest lintDebug assembleDebug --stacktrace --console=plain` passed in 1m24s. Rebuilt APK SHA-256 `b9713eb892d21fdd8fccec8a67e7079547b5255795b51e82f6e053094b80aa13`; installed it on A54 `R5CW31AX4FL` (SM-A546E, Android 16/API 36).
- On-device retest: mDNS discovery found `QR Probe`; join request and host approval succeeded and the host displayed `Connected — sync fair`. I did not restart playback on this attempt. Closing the desktop app while the listener was connected resulted in the Android Join session page showing “The nearby connection failed” during its reconnect attempt. This did not verify the required graceful end-while-playing behavior. I backed out to Nearby Sessions and closed the desktop; no host ports are listening. Block 47 and Block 52.8 remain open.
- User scope direction: skip tasks requiring a second physical Android phone. The two-listener matrix and multi-device skew/scaling checks remain unchecked and are intentionally omitted for this run; all single-phone checks remain in scope.
- Environment probe for the remaining fresh-machine/firewall gaps: this host is Ubuntu 22.04.5 LTS with `/dev/kvm` and `qemu-system-x86_64`, but no `virt-install`, `VBoxManage`, or local ISO was found. `ufw.conf` remains `ENABLED=no`; `ufw status verbose` and `nft list ruleset` require root, and `sudo -n` reports that a password is required. Do not claim runtime firewall policy or fresh-VM physical LAN acceptance from these facts.
- Follow-up documentation/acceptance work: aligned the final summary with existing Block 47/31 evidence for packaged mDNS discovery and physical QR scan/join, and removed the stale note that the updated APK could not be installed. On the packaged AppImage (`165fa88baeae18ddfc2167b63dcc282f88dd5baea6d1c4d0c3017af5ba4fddd7`), opened Diagnostics and used “Export to file.” The UI reported “Diagnostics export saved”; `/tmp/silent-disco-diagnostics-20261015.json.json` was 1,182 bytes and parsed as a JSON object with the expected diagnostics sections. Marked only the Block 47 diagnostics-export item complete. The desktop close interaction did not terminate the GUI process in this run; do not count it as clean-shutdown evidence.
- Packaged AppImage single-phone disconnect/reconnect attempt: created a
  `QRProbe` host with manual approval, A54 mDNS discovery found it, and the
  desktop approved the pending Android listener. Desktop showed the listener
  `Connected — sync fair`, but the Android join checklist remained at
  `Connecting`. Disabling Wi-Fi alone did not interrupt the host row because
  cellular data was still active. Then disabled Wi-Fi and mobile data together
  for about 23 seconds; the desktop still showed `Connected — sync fair` and
  the phone did not show a disconnect/recovery transition. Restored Wi-Fi and
  mobile data. This is a failed/inconclusive acceptance attempt, not a pass;
  Android disconnect/reconnect remains open. Do not infer the UI checklist
  matches the transport state without resolving the `Connecting` mismatch.
- Cleanup after the interrupted session: one desktop End returned the
  packaged host to Host Setup and `ss` showed no listeners on its advertised
  control, sync, or audio ports. The A54 still displayed the Join session
  `Connecting` spinner afterward. An attempted Alt+F4 with no active host did
  not exit `silent-disco-desktop`; no clean-shutdown claim is made.
- After the failed Alt+F4/window-close attempts, stopped the idle packaged
  desktop process with SIGTERM while no host session or host ports remained.
  This is process cleanup only and is not clean-shutdown acceptance evidence.
- Desktop interface disruption attempt with packaged AppImage SHA-256
  `165fa88baeae18ddfc2167b63dcc282f88dd5baea6d1c4d0c3017af5ba4fddd7`:
  connected the A54 over mDNS, approved the listener, confirmed the phone
  displayed `Playing in sync`, and started the 60-second WAV. Disconnected
  desktop Wi-Fi interface `wlo1` for about 8 seconds, then reconnected to
  `ElephantStone`. The phone capture during outage still showed `Playing in
  sync`; after restoration it showed `Playback stopped` and `Unknown host`.
  The host process remained alive, but the desktop reported no connected
  listeners and its transport worker reported `synchronization transport
  Unauthorized: datagram source is not an authorized peer route`. Broadcast
  diagnostics showed 6,350/12,002 frames sent to every connected listener,
  5,649 with no listener, and zero failed before send; the WAV reached
  `Finished`. End did not release the session after retries, so I terminated
  the desktop process with SIGTERM. This confirms host-process survival but
  not listener recovery or clean shutdown; keep both acceptance rows open.
- After process termination, Android prompted to leave the disconnected
  session. Confirmed Leave session; the A54 returned to the Silent Disco home
  screen. The desktop's host control port closed with the process; remaining
  unrelated listeners on other ports had no owning process shown by `ss`.
- Restart acceptance follow-up: relaunched the same packaged AppImage after
  the forced process termination. It opened to Host Setup with no host control
  port listening; the prior session was not restored. Marked the Block 47
  desktop restart row complete as process-restart recovery evidence. This does
  not count as graceful shutdown acceptance.

## 2026-10-15 — second instrumented End reproduction

- Repeated the one-action End acceptance on the packaged AppImage SHA-256
  `165fa88baeae18ddfc2167b63dcc282f88dd5baea6d1c4d0c3017af5ba4fddd7` with
  the A54 `R5CW31AX4FL` on the same Wi-Fi LAN. Created `Interface Probe`,
  approved the listener, confirmed Android showed “Playing in sync,” and
  started `/tmp/silent-disco-acceptance-tone-60s.wav`. One End click remained
  pending after more than 40 seconds. Desktop still showed lifecycle
  `streaming` and TCP control port `192.168.87.161:38717` remained open. Android
  showed Playback stopped only after desktop SIGTERM, followed by its
  “could not continue the listener workflow” state. The process had to be
  terminated; this is another failed one-action teardown attempt, not a pass.
- Attempted to attach GDB for thread backtraces, but ptrace was denied by the
  host Yama policy (`Inappropriate ioctl for device`). Added temporary
  `shutdown_probe` stage markers to the host effect, host stop, and transport
  worker paths and built an AppImage (`bbee4702cf835e8833d775b6fed41b489eb27a2a50e3e50e45834406a7cf271c`).
  Automation successfully created `Shutdown Probe` and connected the A54, but
  did not produce a controlled End-with-playback run or stage output. Removed
  the temporary markers from source. Do not use the instrumented package for
  acceptance. No production fix was established. Need a debug run with captured
  shutdown-stage output or a permitted thread backtrace before changing shutdown
  ownership.
- `ending_host_during_playback_does_not_hold_network_lock_while_joining`
  passed in the prior turn, but the physical one-action failure still
  demonstrates that this test does not cover the failing end-to-end path.
- After removing the temporary markers, rebuilt the production AppImage.
  `npm run tauri build -- --bundles appimage` completed successfully and
  generated clean-source SHA-256
  `f20cfaa678354c7ca96989299429c1ea484848ead3529c63470c8eb36db796f1`.
  Confirmed no `shutdown_probe` markers remain in Tauri source, no Silent Disco
  host ports remain open, and no desktop host process remains running. The
  clean AppImage was built but not revalidated on the phone.
- Current-build physical retest on the clean AppImage SHA-256
  `f20cfaa678354c7ca96989299429c1ea484848ead3529c63470c8eb36db796f1`:
  created `Interface Probe`, discovered and approved the A54 over mDNS, verified
  desktop `Connected — sync excellent` and Android `Clock sync locked`, and
  started the 60-second WAV. One End action remained `End-session request
  queued` after more than 35 seconds; desktop lifecycle stayed `streaming` and
  TCP control port `192.168.87.161:33021` remained open. Android stopped
  playback only after desktop SIGTERM and showed “Silent Disco could not
  continue the listener workflow.” Explicit Leave returned Android home. The
  current clean package therefore reproduces the one-action teardown failure;
  no acceptance checkbox is changed.

## 2026-10-15 — Android peer-close handling and instrumented End retest

- The A54 log from a prior shutdown-stage probe showed Android's final
  `sendDisconnect` throwing `control transport Delivery: failed to write
  control frame: Broken pipe` after the host closed the peer. Updated
  `ListenerTransportController.closeExistingHandle` to treat only broken-pipe,
  connection-reset, and connection-aborted delivery-write failures as an
  expected peer-close during cleanup. The handle shutdown still runs, and
  other errors remain reportable. Added lifecycle regression tests for
  expected broken pipe and non-expected permission-denied/closed failures.
- `./gradlew testDebugUnitTest lintDebug assembleDebug --stacktrace
  --console=plain` passed (`BUILD SUCCESSFUL in 1m47s`). Installed APK SHA-256:
  `a83b8f875be9dac53b7df5d4a9f47ee83f9e93acef6ba169c6c20a3e7485e8d6`.
- On the instrumented AppImage, the A54 joined `ShutdownTrace`, was approved,
  reached “Playing in sync,” and played the 60-second WAV. One End during
  playback stopped phone playback and changed the desktop lifecycle snapshot
  to `ready`, but after more than 45 seconds End remained queued, TCP
  `192.168.87.161:46469` stayed open, and Android continued monitoring the host
  connection. No listener workflow error appeared during the wait. This is not
  a teardown pass; it proves at least one stage progressed and exposes a
  mismatch between lifecycle state and effect completion.
- Removed temporary `shutdown_probe` Rust `eprintln!` lines after the run.
  Rebuilt the clean-source AppImage successfully with `npm run tauri build --
  --bundles appimage`. Clean package SHA-256:
  `c28a891eb401131d602d04019cab13f510b7d878c8baf168f1313270a51d6cb8`;
  instrumented package SHA-256 was
  `34b72f3f0fd77c3f65d418684875a46987b35fb033cd2ba6b7bf56352b9a9f9b`.
  Confirmed no probe string in the clean package, launched it to Host Setup,
  and confirmed there was no idle host port. The A54 explicitly left after the
  instrumented host was stopped and returned to app home. Block 47 graceful
  teardown and Block 52.8 full teardown remain open; no one-action host-end
  pass is claimed.

## 2026-10-15 — End error correlation and latest physical attempt

- The earlier instrumented End run stayed queued with no newer snapshot. The
  actor validates `expected_revision` asynchronously and reports stale
  revisions as operation-correlated errors; the Tauri error DTO had been
  dropping that operation ID. This is consistent with the observed stall, but
  the rejecting notification was not directly captured. Implemented operation
  ID propagation and one bounded retry after refreshing the snapshot. Tests
  cover the DTO identity, latest-revision retry, and keeping End pending through
  an intermediate `ready` state. `npm run check` passed with 143 tests;
  Tauri all-feature tests and strict Clippy passed.
- Built and launched AppImage SHA-256
  `fc4691f4aea8a2db68484b49233ee7c7f12b7d74821e640ecd9306396c47c0d7` for
  `FinalEndCheck`. A54 Nearby Sessions did not discover the mDNS advertisement.
  `adb shell input text` could not enter the copied JSON connection payload
  correctly. Generated a fresh QR invitation and opened the scanner, but no QR
  was decoded and no listener joined. End with zero listeners returned to Host
  Setup and closed the host port; phone returned to app home. This does not
  satisfy connected End acceptance. The desktop process remains idle in Host
  Setup; no host port is open.
- Keep Block 47/52.8 single-phone graceful teardown open. Second-phone rows,
  root-only firewall inspection, fresh Ubuntu 22.04 graphical VM workflow,
  physical hot-unplug, and physical listener scaling remain unavailable or
  skipped as documented.

## 2026-10-15 — Connected End trace isolated before StopAdvertising

- Temporarily added shutdown markers to the desktop platform effect executor,
  host network stop path, and playback pump. Traced AppImage
  SHA-256: `0fef7c34c4bde1cb09033fcaffb4992c7e6679a0f0c9e75c4ce1aa9f660a66a1`.
- On A54 `R5CW31AX4FL`, `Trace End` was discovered over mDNS, approved, reached
  “Playing in sync,” and played the 60-second WAV. One End changed the desktop
  host lifecycle to `ready`, but the UI remained “End-session request queued”
  and the control TCP listener `192.168.87.161:45461` stayed open.
- The pump trace completed all cleanup markers: pump exit, packetizer cancel,
  Stop broadcast (`ok=true`), and Stopped actor event (`ok=true`). There was no
  `StopAdvertising` executor entry in the captured output before terminating
  the traced app. Observer-boundary markers were added to source after that run,
  then removed without rebuilding; no observer-boundary output was captured. The
  next diagnostic should rebuild with those markers around
  `DesktopCoreObserver::on_notification` and effect dispatch.
- An unconnected End on that same traced build completed every trace stage and
  closed the port, confirming the stall is specific to End while playback is
  active. The traced AppImage must not be used again.
- All temporary `shutdown_probe` lines were removed from source. A clean-source
  AppImage rebuild was started; its result/hash must be recorded separately.
  Block 47 and Block 52.8 remain open. The two-phone items remain skipped by
  the user's instruction.

## 2026-10-15 — End observer trace build and Wayland input limit

- Added a second temporary trace at the actor notification worker and
  `DesktopCoreObserver`/platform effect dispatcher boundary, then built the
  release executable. The connected End repro was not run on that build: the
  Wayland window was visible and inspectable through AT-SPI, but the text field
  had no callable AT-SPI EditableText interface and Mutter rejected synthesized
  keyboard events. This automation path cannot create the next host session.
- Removed every temporary `shutdown_probe` marker and ran `cargo fmt --all` plus
  `cargo build --release --locked` successfully. Confirmed the clean AppImage
  artifact hash is
  `499d3aa34d33a88770e5f2a140ec3225e9b56b6194129c032e4e3eb2d0de35d7`; it
  contains no trace marker. The prior connected trace remains the latest valid
  physical evidence: pump cleanup completed, but no StopAdvertising executor
  entry was captured. Block 47 and Block 52.8 one-action teardown remain open.

## 2026-10-15 — End revision race identified and core fix

- Reproduced connected End on an instrumented release executable under the
  X11 backend. A54 `R5CW31AX4FL` (Android 16/API 36) discovered `Actor Trace 4`
  over mDNS, was approved, reached “Playing in sync” / “Clock sync locked,”
  and received 1,606/1,606 frames during the observed sample.
- The actor log captured the exact failure: operation `command-6` expected
  snapshot revision 110 while the actor's serialized current revision was 112.
  It emitted only `CoreError::InvalidStateTransition`; the actor never enqueued
  `StopAdvertising`. The phone stopped playback as the platform teardown
  partially progressed, then remained monitoring while the host control port
  stayed open. This confirms the revision race as the direct cause of this
  repro, rather than a stalled StopAdvertising executor.
- Changed core command handling so `EndHostSession` acts on the actor's
  currently active session even when its UI-sampled revision has advanced.
  Other commands still enforce expected-revision checks. Added
  `stale_revision_end_command_ends_the_current_host_session`, which verifies
  the stale request reaches `EndingSession` and emits `StopAdvertising`.
- Validation: focused lifecycle integration test passed; all 5 tests in
  `host_block12_actor_lifecycle` passed; Tauri `cargo test --locked
  --all-features` passed (335 passed, 5 ignored); strict Tauri Clippy passed;
  Tauri format check passed after formatting. `cargo build --release --locked`
  passed. Clean executable SHA-256:
  `0c32e9e26b1a74553de5d2d64a4adecdd6ef4a6bde55b21a41f23d924ce53199`, with
  no `shutdown_probe` string.
- With the clean fixed binary, created `End Fix Acceptance`; one End with zero
  listeners returned to Host Setup and closed the host port. The A54's Nearby
  Sessions scan did not discover this session, so this is not connected-end
  acceptance. The old connected-listener failure evidence remains, while the
  connected case must be rerun on the fixed packaged build. Keep Block 47 and
  Block 52.8 open. Second-phone tasks remain skipped per user direction.
- Packaged the fix as AppImage SHA-256
  `3e18179038321a68dfb474f8ffed03b341a11289bf6fb94dc1f176285b79ab99`;
  `python3 desktop/scripts/verify-linux-bundles.py --bundle-dir
  desktop/src-tauri/target/release/bundle --tauri-config
  desktop/src-tauri/tauri.conf.json --cargo-manifest
  desktop/src-tauri/Cargo.toml` passed. The AppImage opened after closing the
  earlier executable instance. It initially surfaced the expected single
  profile-lock error while that instance was still open; closing the older
  instance released the profile and the AppImage then opened to Host Setup.
- On the packaged AppImage, created `End Fix Packaged` with the WAV, confirmed
  the A54 Nearby Sessions scan still did not show that mDNS advertisement, and
  used one End with zero listeners. The app returned to Host Setup and
  `ss -ltnp` showed no host control listener. This validates packaged
  zero-listener End, not the connected listener teardown row.


## 2026-10-15 — Connected End verified on fixed packaged AppImage
- Reused the single A54 (`R5CW31AX4FL`, Android 16/API 36) and clean packaged AppImage SHA-256 `3e18179038321a68dfb474f8ffed03b341a11289bf6fb94dc1f176285b79ab99`. Manual endpoint payload connected the A54; the host approved it, and the phone showed Connected then Playing after starting the 60-second source.
- One End action returned the desktop UI to Host Setup and closed host ports. The phone displayed Host disconnected with `runtime transport ShuttingDown: transport event channel is closed`; because this was the manual-connect page, it remained there with Try again. This confirms the fixed host End path with a live listener but does not verify the discovered-session return-to-home path or the coupled clean listener teardown acceptance.
- Updated Block 47 and Block 52.8 with this result. No second phone used. Previous test suite/build evidence is recorded in the TODO; no new tests were run in this retest. Worktree remains broadly dirty from earlier work, so do not stage/commit it wholesale.

- Follow-up UI cleanup: manual endpoint polling maps typed `Closed` (including native `ShuttingDown`) to a plain disconnected state with no low-level shutdown detail; other transport and cleanup failures retain their diagnostics. `./gradlew assembleDebug --stacktrace --console=plain` passed and the rebuilt APK was installed on the A54 (SHA-256 `bb859c8af22308b9f8c55708ae510fd9c24408af136124f7d766cdc8488cd0ec`). The updated message was not separately re-observed on device.


## 2026-10-15 — Android clean manual disconnect and packaged shutdown recheck
- Updated `ManualListenerTransportControllerTest` to require `Closed` poll errors to become `Disconnected(null)`, while preserving detail for other errors and reporting cleanup failures. Focused Android test `./gradlew testDebugUnitTest --tests com.ekkus.silentdisco.core.rust.ManualListenerTransportControllerTest --stacktrace --console=plain` passed.
- On the installed A54 APK SHA-256 `bb859c8af22308b9f8c55708ae510fd9c24408af136124f7d766cdc8488cd0ec`, repeated manual endpoint join, host approval, playback, and one-action End on packaged AppImage SHA-256 `3e18179038321a68dfb474f8ffed03b341a11289bf6fb94dc1f176285b79ab99`. Desktop returned to Host Setup and closed the host ports. Android showed `Host disconnected` / `The connection was closed.` without the low-level event-channel detail. Its manual-connect screen offered retry/back.
- Closed the idle AppImage window; the desktop process exited. Relaunched the same AppImage; it reopened Host Setup with no host ports listening. Screenshots are ignored target artifacts `desktop/src-tauri/target/a54-manual-end-disconnected.png` and `desktop/src-tauri/target/a54-manual-end-host-setup.png`.
- Block 47 connected End, clean listener teardown on the manual endpoint route, clean shutdown, and Block 52.8 manual end-while-connected are now checked. Discovered-session return-home remains distinct and unclaimed. Remaining acceptance gaps are environmental or other single-device scenarios; two-physical-phone cases remain skipped at the user's direction.

### Desktop-host TODO follow-up — A54 network drop and Android recovery affordance (2026-10-15)

- Continued `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` with packaged AppImage/A54 reconnect attempt. `ReconnectProbe` reached `Playing in sync`; turning off both Wi-Fi and mobile data stopped playback and cleared the host identity. Wi-Fi restoration did not resume playback in 15 seconds. Explicit phone Leave and one desktop End cleaned up; host control port closed. Physical disconnect/reconnect acceptance remains open.
- The lost-host UI showed `Playback stopped` / `Unknown host` with no recovery action. `listenerConnectionHealthSummary()` now treats a stopped listener on the listener route with no selected session as critical, which exposes the existing `Fix connection` route. Added `AppPresentationTest.stoppedListenerWithLostHostOffersConnectionRecovery`.
- Verification: focused `AppPresentationTest` passed; `lintDebug` passed; `assembleDebug` passed; APK installed on A54. APK SHA-256 `dd0e4df80f6eab32b8f449c9b45bf6dd8f2b2fe912c65ee0fdc0afc521bd0413`. Recovery button itself has not yet been exercised in another physical interruption run.

- 2026-10-15 follow-up: installed APK SHA-256 `ea3890dc4856deb92b29dd832403fb0b05756cf59feee0b8d3e70795ef38afe7`. Reconnect-action physical retest could not be reached because Nearby Sessions stayed empty while desktop mDNS status read Advertising; generated QR could not be decoded from the automated phone camera view. Ended the zero-listener session and confirmed ports closed. No claim made for the recovery action.
- Clarified Block 47's host-source and local-monitor physical acceptance rows with existing production-path test references (`a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming`, `source_read_failure_is_visible_and_never_publishes`, and `a_locally_rejected_monitor_datagram_fails_visibly_and_disable_propagates_teardown`). These tests cover software fault behavior; they do not replace physical packaged acceptance.
- Committed and pushed the isolated Android lost-host recovery UI/test as `2609475` (`fix(android): expose lost host recovery action`). The TODO and memory files remain uncommitted because they contain broader prior-session updates.
- Follow-up route check found Connection Help hid `Reconnect` for the stopped/cleared-host state. Added shared `AppUiState.canReconnectToHost()` and wired the help screen to it; expanded the focused unit assertion. `testDebugUnitTest --tests com.ekkus.silentdisco.app.AppPresentationTest`, `lintDebug`, and `assembleDebug` pass. APK installed on A54, SHA-256 `dd0e4df80f6eab32b8f449c9b45bf6dd8f2b2fe912c65ee0fdc0afc521bd0413`. Physical action path remains unverified due mDNS/QR limitation documented above.
- Shared listener recovery predicate and Connection Help Reconnect visibility follow-up committed and pushed as `4641fa3` (`fix(android): show reconnect action after host loss`).

### Ubuntu clean-container AppImage smoke (2026-10-15)

- Pulled Ubuntu 22.04.5 and ran the current AppImage SHA-256 `3e18179038321a68dfb474f8ffed03b341a11289bf6fb94dc1f176285b79ab99` with GTK/WebKit, AppIndicator, FUSE compatibility, and Xvfb packages installed. `--appimage-extract-and-run` launched under Xvfb and remained alive through the 12-second timeout (exit 124); no startup errors were emitted.
- This is basic clean-userspace startup evidence only. The container had no physical audio device or Android connectivity; graphical profile/source/diagnostics flows and uninstall remain unverified. Block 46.3 stays open.

### Android lost-host recovery route follow-up (2026-10-15)

- Repeated single-A54 network interruption on the packaged AppImage. The phone reached `Playing in sync`; after disabling Wi-Fi and mobile data it showed `Playback stopped` / `Unknown host` and `Fix connection`. Restoring Wi-Fi and tapping `Reconnect` started discovery (`listener.retry` logged) but left the UI on Connection Help, preventing session selection.
- Updated `SilentDiscoApp`: Reconnect now navigates to Nearby Sessions, where the route avoids a duplicate scan when retry already started it. Focused `AppPresentationTest` and `ConnectionHelpPresentationTest`, `lintDebug`, and `assembleDebug` passed. Installed APK SHA-256: `a18ab2844fff505e5f56b9f0418d24e10f9e7350f7f55752863fe13e5b3faeca`.
- From Nearby Sessions the A54 found `ReconnectProbe`, but the subsequent join did not finish: the desktop transport worker reported `control transport io: failed to read control connection: Connection reset by peer`. End returned Host Setup and closed host ports; Android returned home. The new route has not yet been physically exercised through a second interruption. Firewall rules still require root inspection; no external USB audio endpoint is attached for hot-unplug.
- Final retest on APK SHA-256 `a18ab2844fff505e5f56b9f0418d24e10f9e7350f7f55752863fe13e5b3faeca` and packaged AppImage SHA-256 `3e18179038321a68dfb474f8ffed03b341a11289bf6fb94dc1f176285b79ab99` (14:21–14:25 PDT): after a second Wi-Fi/cellular interruption, `Fix connection` → `Reconnect` routed to Nearby Sessions; mDNS found the host, join and host approval completed, and Android reacquired `Playing in sync` / `Clock sync locked` after the host restarted the finished 60-second source. Desktop showed `Connected — sync good`. End returned Android home and closed host ports. This verifies explicit user-driven recovery and resynchronization; it does not verify automatic reconnection.
- Committed and pushed the connection-help route and duplicate-scan guard as `9d31ec0` (`fix(android): route host reconnect to nearby sessions`). Documentation/memory remain uncommitted because those files already contain broad unrelated pending changes from earlier loop turns.

## 2026-10-16T05:53:29Z - Claude Opus 5.5 - Gapless handoff exact-head gates and Android failure Stop timestamp coverage
- Exact-head reruns at `af8e090` (API 36 emulator `emulator-5554`, no physical devices): full Android gate `./gradlew testDebugUnitTest lintDebug assembleDebug assembleRelease connectedDebugAndroidTest` passed (120 instrumentation cases, 119 passed, 1 skipped); desktop `npm run check` passed (157 frontend tests, build); desktop Tauri strict Clippy clean and `cargo test --locked --all-features` 347 passed, 5 ignored. Rust workspace not rerun (no Rust changes since `007c48e`, where it passed).
- Added production seams so JVM tests can drive the Android host stream loop: `MainViewModel` constructor now takes `hostTransportController`, `hostElapsedRealtimeMs`, `hostStreamDispatcher`, `closeHostMonitorOutput`; `HostTransportController` takes `callDispatcher` and `bindHandle` (event poll loop stays on IO); `HostPlaylistStreamController` constructor is `internal`. UniFFI-generated classes are `open` with a `NoHandle` fake constructor — subclass them for fakes instead of mocking.
- New `HostStreamFailureStopTest`: late Rust `Decode` failure broadcasts Stop exactly at the last sent packet's audible end (and after a pause-shifted offset). Mutation check (removing the tail wait) fails both tests. TODO section 4 failure item evidence updated; item left unchecked pending section 9 device evidence.
- After the change, the full Android gate passed again (120 cases, 119 passed, 1 skipped); after a final import/doc-only tidy, JVM tests, lint, and debug/release assembly were rerun and passed. Source-line check (642 files) and `git diff --check` pass.

## 2026-10-16T06:15:19Z - Claude Opus 5.5 - Listener pause/resume/reconnect fixes and host failure-boundary race
- Fixed: Android listener with a re-anchored (same-stream resume) runtime stayed PAUSED forever (poller latched PLAYING once per runtime), blocking all later boundary titles. Resume now sets BUFFERING before rescheduling; `shouldReportListenerPlaying` promotes BUFFERING->PLAYING, never overrides PAUSED.
- Fixed: paused reconnect/late-join replay (StreamStart with pre-pause anchor, boundaries, then Pause) presented the upcoming track. Pause now withdraws boundaries applied after hostPauseTimeMs (`appliedListenerTrackBoundaries`, bounded 100) and restores the title at the pause instant.
- Host: decode failure after a boundary but before next-track audio now cancels that boundary immediately (`hostBoundaryJobs` keyed by sample index) instead of tying with the tail deadline.
- Found, not fixed: boundary commit-point edit race (worker commits boundary ~640 ms before hosts report TrackBoundaryScheduled; edits in that window are accepted but the removed track still plays). Reopened TODO §3 item. Options presented to user: actor-approved commit (recommended), immediate commit report, actor end-of-track lock margin.
- Full Android gate passed (120 instrumentation, 119 passed, 1 skipped); source-line check (643 files) and diff-check pass.
