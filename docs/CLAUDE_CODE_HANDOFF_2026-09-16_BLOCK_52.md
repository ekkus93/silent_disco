# Claude Code Handoff — Desktop UI and Device Defects (Blocks 49–52) — 2026-09-16

**Repository:** `ekkus93/silent_disco`
**Branch:** `master` (work is committed straight to master)
**Master at handoff:** `ea0dd37` plus the commit that adds this file
**Machine:** laptop `arisu`, Ubuntu 22.04.5 LTS amd64 (not `kawa`)
**Phone:** Samsung Galaxy A54, Android 16, ADB serial `R5CW31AX4FL`
**Primary TODO:** `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`
(Blocks 45–52, placed after Block 50 and before "Future phases")
**Paused TODO:** `docs/BLOCK_46_3_FRESH_MACHINE_CLOSEOUT_TODO_2026-09-15.md`
**Earlier handoff:** `docs/CLAUDE_CODE_BLOCK_46_3_HANDOFF_2026-09-16.md`

Read `CLAUDE.md`, `memory.md` (newest entries are at the bottom), and Block 52
of the primary TODO before starting.

## Where things stand

The user paused Block 46.3 (fresh-machine release validation) on 2026-09-16 to
fix the UI first. Since then:

| Block | Scope | State |
|---|---|---|
| 49 | Desktop shell, navigation, shared components | Done except 49.4 |
| 50 | Desktop Settings screen | Done except two 50.2 items |
| 51 | Shared address space (CGNAT, 100.64/10) opt-in, persisted (migration v3) | Done, device-verified |
| 52 | Join and playback defects found on the A54 | 52.1–52.4 and 52.6 fixed and device-verified; 52.5 partly done |

Block 52 fixes so far, all on master:

- **52.1** — Approved listeners were evicted when playback started. The core's
  inbound-silence guard is now armed only by a datagram or a delivered
  StreamStart, and disarmed by Stop.
- **52.2** — Android sent the join request before reporting the endpoint as
  ready.
- **52.3** — An mDNS expiry reset a listener that had already joined.
- **52.6** — The discovered-session (mDNS) path never started clock sync.
- **52.4** — A refused effect stopped core notification delivery. Unconfirmed
  join decisions and removals now time out after 15 s into a visible,
  retryable failure. This fix is not device-verified, because a refused
  effect cannot be provoked on demand.
- **52.5** (done parts):
  - The desktop no longer shows the approval delivery as if it were audio
    delivery. It now shows a "Streaming to nobody" alert, and the approval
    line is labeled "Last control message (approval, rejection, or removal)".
  - The desktop listener row no longer says "connecting" after approval.
  - Android no longer ticks "Waiting for host approval" before the host
    approves: `CONNECTING` is no longer treated as approved.
  - The broadcast counter now separates "reached no listener" from "reached
    only some".

**Device-verified on 2026-09-16:**
- Carrier-NAT network: desktop `100.64.64.132`, phone `100.64.65.39`.
- The mDNS join path plays in sync end to end: RTT 5–8 ms, and "Playing in
  sync" held for more than 13 minutes.
- The approval, playback, and streaming-to-nobody wording was checked on build
  `9a8767f`.

**Not yet device-verified:** the `ea0dd37` counter split. It needs a new desktop
package.

## Latest gate results

The results below are from this session; gates were not re-run for this
docs-only commit.

- Desktop:
  - `npm run check`: passed, 115 tests.
  - `cargo clippy --all-targets -- -D warnings` in `desktop/src-tauri`: clean.
  - `cargo test` in `desktop/src-tauri`: 233 passed.
- Android: `./gradlew test lintDebug` passed, with 966 unit-test runs and 0
  failures (after `9a8767f`; there were no Android changes afterwards).
- Shared Rust (`bash scripts/check-rust.sh`): last run during 52.4, with 447
  tests. The core has not changed since.
- Known flake: `LabScreen.test.tsx`, "edits a loaded link fault profile through
  the backend command", failed 1 of 3 isolated runs. It is recorded under 52.4
  and not investigated.

## What still needs to be done

### Block 52 (current work)

1. **52.5 approval delay (unconfirmed).** The host socket shows the approval was
   sent at about 13:22:02, but the user reported clicking Approve before
   13:21. Nothing records the click time. Log both the decision time and the
   send time, then reproduce.
2. **52.5 packets that missed the only listener.** About 11 packets went to the
   single listener and reached nobody. The likely cause is the window between
   the phone leaving and eviction, but the host does not log which packets
   failed or when.
3. **52.5 remaining misleading or missing diagnostics:**
   - Android's manual-connect path writes nothing to logcat.
   - The host name is shown on Android as a raw 64-hex device ID.
   - The desktop network card says only private-LAN addresses can be
     advertised while bound to a CGNAT address.
   - "Session name is invalid" is shown on an untouched empty field.
   - "Playback start requested." stays under the session header after
     playback has started.
   - The empty listener text ("…delivery-confirmed approval.") is jargon.
4. **52.6** — The first ~1 s of audio is lost (140–180 packets dropped before
   sync lock, plus render-ring underruns). Start clock-sync probing at approval
   instead of at stream start.
5. **52.1** — Confirm the Android listener keeps probing through a long pause,
   or disarm the silence guard on pause.
6. **52.4** — Re-check in the GUI when possible, and look at the LabScreen
   flake.

### Blocks 49 and 50

- **49.4** — Record an observation timestamp for core errors, and let the user
  dismiss them.
- **50.2** — Show the mDNS advertising status next to the address in the
  connection panel.
- **50.2** — Profile identity is still hardcoded.

### Release path (after the UI work)

- **Block 46.3** — fresh-machine validation, paused.
  - Rebuild and stage from current master.
  - The user runs the graphical validator
    (`desktop/scripts/validate-block46-fresh-machine.sh`).
  - The closeout TODO has 73 open items and 5 done.
  - Section 3a lists the desktop UI changes that affect the acceptance steps
    (network policy moved to Settings, and others).
- **Block 47** — final Android interoperability acceptance. The two-listener
  cases need a second Android device.
- **Blocks 45 and 48** — performance and soak measurements, then the final
  documentation and completion audit.

### Open question for the user

`CLAUDE.md` says the initial packet duration target is 20 ms, but the
packetizer uses 5 ms (`DEFAULT_PACKET_DURATION_MS`, 200 packets per second).
Nothing has been changed; ask the user which one should be authoritative.

## How to work on this machine

**Environment:**
- Node: run `nvm use 22.16.0`.
- Java and Android SDK:
  `JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64`,
  `ANDROID_SDK_ROOT=ANDROID_HOME=$HOME/Android/Sdk`.

**Build and install:**
- Desktop package: run `cd desktop && npm run tauri build`.
- Verify the bundles with `python3 desktop/scripts/verify-linux-bundles.py
  --bundle-dir desktop/src-tauri/target/release/bundle --tauri-config
  desktop/src-tauri/tauri.conf.json --cargo-manifest
  desktop/src-tauri/Cargo.toml`.
- The user installs the desktop package themselves (it needs sudo):
  `sudo apt install --reinstall "./desktop/src-tauri/target/release/bundle/deb/Silent Disco_0.1.0_amd64.deb"`.
- Confirm the install by comparing the sha256 of `/usr/bin/silent-disco-desktop`
  with the binary extracted from the `.deb` (`dpkg-deb -x`).
- APK: run `./gradlew assembleDebug`, then
  `adb -s R5CW31AX4FL install -r app/build/outputs/apk/debug/app-debug.apk`.

**Driving the apps:**
- Ask the user to launch the desktop GUI as
  `silent-disco-desktop 2>&1 | tee ~/silent-disco-desktop.log`, so its output
  can be read. The session is GNOME Wayland, so `xdotool` cannot drive the
  GUI. The user operates the desktop and shares screenshots.
- The phone can be driven over ADB with `uiautomator dump` plus `input tap`.
  - Sending BACK on an app root screen exits to the launcher.
  - "Cancel and go back" may exit the app.

**Headless device harness** (desktop host to the A54):
- Run from `desktop/src-tauri`:
  `cargo test --lib manual_real_android_listener_plays_mp3 -- --ignored --nocapture`.
- Set `SILENT_DISCO_DEVICE_TEST_ALLOW_SHARED_ADDRESS_SPACE=1` on this network.
- Optional: `SILENT_DISCO_DEVICE_TEST_SOURCE=<file>` and
  `SILENT_DISCO_DEVICE_TEST_PLAY_DELAY_SECS=N`.

**Host-side evidence:** `ss -tni` shows the control connection to the phone,
with `lastsnd`, `lastrcv`, and byte counts.

**Working rules for this repository:**
- The user commits with `/commit-push`, straight to master. Never add a
  `Co-Authored-By` line.
- Do not claim device results the user did not observe. Record device runs in
  the TODO and in `memory.md`, with build SHAs and binary hashes.
- The Android gate does not load native Rust code, so it cannot validate
  UniFFI or schema changes. Those need a device run.
