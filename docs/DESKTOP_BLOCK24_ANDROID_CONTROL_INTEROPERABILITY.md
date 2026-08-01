# Desktop Block 24 — First Physical Android Control Interoperability

**Status:** Complete (control-plane only; no audio interoperability claimed)
**Date:** 2026-08-01
**Handoff document:** `docs/CLAUDE_CODE_HANDOFF_DESKTOP_BLOCK24_2026-08-01.md`
**Starting commit:** `7e6bdc15a33a2b3266f7a7954e927d263be53190`
**Completion commit:** `2b9be53` (exact hash recorded by `git log` at completion; the two defect
fixes landed at `f0fff45`, immediately before it)

## Acceptance condition

> One Android listener completes a real control-plane session with the desktop host over the LAN.

**Met.** A physical Samsung Galaxy A54 completed real, delivery-confirmed join/approval,
rejection, listener-initiated disconnect, host-initiated removal, and desktop end-session
control-plane exchanges against a real desktop host binary over a real Wi-Fi LAN, using the
shared Rust protocol encoder/decoder on both sides (not the legacy Kotlin Wi-Fi Direct codec).

## What was implemented (code, prior to physical testing)

1. **`ManualHostEndpoint`** (`rust/silent-disco-core/src/transport/manual_endpoint.rs`) — parses
   and validates the desktop host's exact "Connection payload" JSON shape (unicast address,
   nonzero ports, bounded session ID, protocol-version match, expiration).
2. **`FfiListenerTransportHandle`** and **`parse_manual_host_endpoint`**
   (`rust/silent-disco-ffi/src/listener_transport/`) — a new, narrowly-scoped UniFFI object
   wrapping the existing production `SocketListenerTransport`, exposing typed connect/send/poll/
   shutdown operations and a distinguishable error enum to Kotlin.
3. **`ManualListenerTransportController`**, **`ManualEndpointScreen`**, and wiring in
   `MainViewModel`/`AppState`/`SilentDiscoApp`/`NearbySessionsScreen` (Android) — a new "Connect
   manually" entry point with live paste/input validation and typed connect-state UI, kept
   deliberately independent of the existing Wi-Fi Direct `SessionInfo`/`ControlMessage` path.

Architecture note recorded at implementation time: the shared `CoreActorRuntime` already models
listener-role commands (`SelectSession`/`SubmitJoin`/`CancelJoin`) but has no event path for a
connected transport to report `JoinApproval`/`JoinRejection` back into the actor —
`TransportEvent::JoinRequested`/`ListenerConnected`/`ListenerDisconnected` are host-role-only
(`require_role_event(AppRole::Host)`). Per the handoff's explicit fallback guidance, the new
handle stays transport-oriented rather than attempting to complete that actor wiring in this
block; Kotlin observes the handle's typed events directly and performs no protocol parsing of its
own. Completing the shared actor's listener join-lifecycle remains separate follow-up work.

## Defects found and fixed during physical testing

Both were discovered only by driving the real UI end-to-end; neither was visible from source
review or from the automated test suite, since automated tests exercise the underlying Rust/
Kotlin logic directly and bypass the UI-level gates involved.

1. **Desktop: `local_network_available` was hardcoded `false`.**
   `desktop/src-tauri/src/platform/capabilities.rs` — `desktop_capabilities()` still reported
   `local_network_available: false`, a leftover from before Desktop Block 21 ("network bind
   policy complete") actually implemented real interface enumeration and binding.
   `HostNetworkPolicyCard` gates its own Refresh button and the entire network-interface UI on
   this flag, which meant the desktop host's real "Create session" button was permanently
   unreachable through the UI — automated tests never caught this because they call the backend
   directly, bypassing the frontend capability gate. Fixed by flipping the flag to `true` and
   updating the two Rust tests (`capability_tests.rs`, `app_state_tests.rs`) that asserted the
   stale value. Commit `f0fff45`.
2. **Android: post-connection transport errors always mapped to `Failed`.**
   `ManualListenerTransportController`'s event-poll loop mapped every
   `FfiListenerTransportException` — including `Closed`/`ShuttingDown` — to
   `ManualConnectUiState.Failed`. A real desktop "End session" therefore rendered as "Couldn't
   connect" on Android instead of a distinct "Host disconnected" state, failing the requirement
   that end-session be distinguishable from a transient network error. Fixed: since the poll loop
   only runs after a connection has already succeeded, any exception it observes is the
   connection ending, not a fresh configuration failure, so it is now always mapped to
   `Disconnected`. The mapping was extracted into a standalone `mapPostConnectionFailure`
   function and covered by a new `ManualListenerTransportControllerTest`, which asserts every
   `FfiListenerTransportException` subtype — including `Closed` — maps to `Disconnected` and
   never to `Failed`. Commit `f0fff45` (fix), follow-up commit (extraction + test).

Both fixes were re-verified against the physical device after applying them (see Scenario E
below); the full automated Rust/Android/desktop gate was re-run afterward and remained green.

## Physical topology

| | |
|---|---|
| Desktop OS/hardware | Ubuntu 22.04.5 LTS, x86_64, physical machine (hostname `arisu`) |
| Desktop connection | Wi-Fi, interface `wlo1` |
| Desktop IP | `192.168.88.109` |
| Phone | Samsung Galaxy A54, model `SM-A546E`, serial `R5CW31AX4FL` |
| Phone OS | Android 16, API level 36, ABI `arm64-v8a` — physical device confirmed via `adb devices -l` (not an emulator) |
| Phone IP | `192.168.88.107` |
| Network | Same private-LAN subnet (`192.168.88.0/24`), no guest-network isolation; connectivity confirmed via `ping` before application testing |
| Firewall | Not independently verified in this session (no interactive `sudo` available); no firewall interference was observed — all scenarios below connected/failed exactly as the protocol dictated |
| App build | `com.ekkus.silentdisco` debug build, installed via `adb install -r`, at commit `f0fff45` (after both defect fixes) for the final Scenario E re-run; commit `a0c7205`..`adc7b6b` state for scenarios A–D, F, G |
| Desktop build | `silent-disco-desktop` debug binary, built from commit `f0fff45` for the final run |

## Automated validation (re-run after all Block 24 changes, including the two fixes)

- `cd rust && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features` — pass (18 new tests: 12 `manual_endpoint` unit tests, 6 `listener_transport` real-loopback-socket integration tests).
- `./gradlew :app:testDebugUnitTest :app:lintDebug :app:assembleDebug :app:assembleAndroidTest` — pass (includes new `ManualEndpointModelsTest`, `ManualEndpointScreenTest`, and the `nearby-connect-manually` addition to `WorkflowScreenStateTest`).
- `cd desktop && npm run check` (bindings-check, Biome format/lint, `tsc`, Vitest, production build) — pass.
- `cd desktop/src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features` — 104 of 105 tests pass. One pre-existing, environment-specific failure
  (`platform::network_tests::port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport`)
  was confirmed present on a clean `git stash` of all Block 24 changes and is unrelated to this
  block; not fixed here.
- `bash scripts/check-source-file-line-counts.sh` — pass (360 tracked files, all below 800 lines).

Also found and fixed as a prerequisite: `desktop/src-tauri` had no `rust-toolchain.toml`, so
`npm run bindings:check` and any direct `cargo` invocation under `desktop/` silently used the
machine's default toolchain (1.95.0) instead of the pinned 1.97.1 required by both crates'
`rust-version`. Added `desktop/rust-toolchain.toml` (commit `66a3272`), placed at `desktop/` so it
resolves for both the npm-script invocation (cwd `desktop/`) and direct `cd desktop/src-tauri`
usage.

## Physical acceptance scenarios

All scenarios were run interactively against the real desktop binary and the real installed APK;
screenshots and a full `adb logcat` capture for the session are retained locally as evidence.

### A. Approval success — PASS

Desktop host session `Block24Test` created (audio source: a locally-generated 1-second WAV test
tone; playback not started, per scope). Android entered the exact copy-paste connection payload
via `Connect manually`, tapped Connect. Desktop showed the real pending join request
("This Android Listener", device `listener-device`, request `desktop-join-1`). Desktop approved.
Android showed "Connected — the host approved this device" only after the real approval message
arrived; desktop showed the listener as connected with "Last delivery: 1 of 1 succeeded; 0 failed
(ok)."

### B. Rejection — PASS (separate run)

Fresh join request (`desktop-join-2`) from the same manual-endpoint flow. Desktop clicked
Reject. Android displayed "Host declined this connection — host_rejected", distinctly worded
from the disconnect scenarios below. Desktop did not show the listener as connected.

### C. Listener disconnect — PASS

Established approval, then used Android's screen back-arrow (mapped to the same cancel action
that sends a real `Disconnect` control message before shutting down the transport). Desktop's
"Connected listeners" reverted to "No listeners have completed delivery-confirmed approval" —
no stale connected-listener row remained. Android returned to Nearby Sessions with a "Manual
connection cancelled" confirmation.

### D. Host removal/disconnect — PASS

Established approval, then used desktop's listener-detail "Remove listener" action. Android
displayed "Host disconnected — host_removed_listener" and did not remain in an approved/connected
state. Desktop's connected-listener list returned to empty.

### E. Desktop end-session — PASS (after fix)

Established approval, then used desktop's "End session" action. **First attempt** (before the
Kotlin fix above) incorrectly rendered as "Couldn't connect" (`Failed` state) with detail
`runtime transport ShuttingDown: transport event channel is closed` — logged as the defect above.
**After the fix**, a fresh end-to-end run (new session `session-2`, fresh join/approval) showed
"Host disconnected" with the same underlying technical detail, now correctly distinguished from a
connection failure. Desktop's lifecycle returned to `idle`.

### F. Invalid endpoint — PASS

Entered a payload with an unreachable host address (`192.168.88.200`, not present on the LAN)
and a closed low port. Android reported "Couldn't connect — control transport Connect: failed to
connect TCP control channel: No route to host (os error 113)" within seconds (bounded by the
transport's connect timeout); no success state was ever emitted.

### G. Wrong protocol version — PASS

Entered a payload identical to a real, working endpoint except `protocolVersion: 999`. Live
client-side validation (via `parse_manual_host_endpoint`) rejected it immediately, before any
connection attempt: "control transport Protocol: unsupported protocol version 999; this build
supports version 2." No connection or join success was claimed.

## Known limitations

- Audio interoperability is explicitly **not** claimed or tested. Playback remains disabled on
  the desktop UI pending later packetization/streaming blocks.
- The shared actor's listener-role join-lifecycle (`AwaitingApproval`/`Approved` states inside
  `CoreActorRuntime`) remains unfinished; this block's Android manual-connect path is
  intentionally transport-level rather than actor-integrated, as recorded above.
- Firewall configuration on the desktop machine was not independently inspected (no interactive
  `sudo` in this session); no evidence of firewall interference was observed in any scenario.
- The pre-existing `port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport` Rust
  test failure on this machine remains open; it is environment-specific (reproduces on a clean
  checkout) and out of Block 24's scope.
