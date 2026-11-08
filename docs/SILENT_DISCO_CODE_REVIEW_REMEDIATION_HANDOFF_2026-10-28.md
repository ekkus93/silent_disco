# Silent Disco — Code Review Remediation Handoff (2026-10-28)

**Repository:** `ekkus93/silent_disco`  
**Branch:** `master`; all GitHub, source commits, and CI through Ralph Bridge only.  
**Verified source baseline:** `809de98e0fab8febd255bf7e46786097383b6321` (complete CI observed on 2026-10-28).  
**Granular implementation audit:** [Implementation Progress Audit (2026-10-28)](SILENT_DISCO_IMPLEMENTATION_PROGRESS_AUDIT_2026-10-28.md) — original gapless 58/72, separate component evidence scorecard; do not confuse broad signoff with code progress.  
**Active checklist:** [Code Review Remediation TODO](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md)  
**Specification:** [Code Review Remediation Spec](SILENT_DISCO_CODE_REVIEW_REMEDIATION_SPEC_2026-10-27.md)  
**Security plan:** [Transport security ADR](SILENT_DISCO_TRANSPORT_SECURITY_ADR_2026-10-28.md)  
**Original queue/playback acceptance:** [Gapless Queue TODO](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md)  
**Disposition:** OPEN. Do not treat green CI as production-v4 security acceptance or physical-device proof.

## 1. Exact checklist completion calculation

After a minimal evidence-backed reconciliation of the active checklist:

- **2 of 60 total checklist items accepted = 3.33%.**
- **58 of 60 items remain unchecked**, including three owner-deferred physical items.
- Excluding those three hardware items: **2 of 57 independently actionable software/acceptance items accepted = 3.51%; 55 remain.**
- Calculation is checked boxes / total boxes; all open checklist items are equal weight. This is a **strict acceptance-signoff percentage**, NOT an estimate of lines of code written, feature engineering effort, or the percentage of runtime functionality implemented. The code contains significant partial implementations inside unchecked items.
- Checkboxes were previously all empty despite prior development. Only two independently inspectable, self-contained requirements were marked complete here. Never infer completion of entire multi-part F1/F2/F3/F4/F5 gates from partial module tests.

| Remediation section | Accepted | Remaining | Interpretation |
| --- | ---: | ---: | --- |
| §0 Baseline and work loop | 0/5 | 5 | Recurring validation and evidence mapping remain open |
| §1 F2 transport security | 1/8 | 7 | LAN threat model and ADR exist; TLS/AEAD and integration unshipped |
| §2 F1 verified admission | 0/7 | 7 | Host trust/identity/policy modules exist; production integration incomplete |
| §3 F3 reliable events | 0/6 | 6 | Substantial fixes/tests exist; full authoritative-event acceptance open |
| §4 F4 Android handle lifetime | 0/6 | 6 | Teardown and queue closure strengthened; full race proof open |
| §5 F5 bounded Android queues | 0/5 | 5 | Bounded critical-event queues implemented; exhaustive cross-layer acceptance open |
| §6 F6 backup and restore | 1/4 | 3 | Backup-exclusion policy verified on installed app; restore/key-loss UX unqualified |
| §7 Maintainability / audits | 0/5 | 5 | Acceptance index, fuzzing, SBOM, and refactors outstanding |
| §8 Visual, accessibility, lifecycle | 0/5 | 5 | Pixel goldens and broad lifecycle/accessibility acceptance open |
| §9 Exact-head qualification/closeout | 0/6 | 6 | Baseline CI green, but final release/security gates still open |
| §10 Owner-deferred hands-on | 0/3 | 3 | Intentionally unchecked, not blocking software work |
| **TOTAL** | **2/60** | **58** | **3.33% acceptance signed off** |

### Two items explicitly accepted in this reconciliation

1. **§1/F2: Written LAN threat model and ADR.** [ADR](SILENT_DISCO_TRANSPORT_SECURITY_ADR_2026-10-28.md) explicitly documents legacy-v3 plaintext exposure, P-256 listener proof, TLS 1.3/rustls direction, AEAD/replay requirements, host identity bootstrap, v4 rejection/downgrade policy, staged migration, rollback and adversarial acceptance. **The ADR is a plan, not deployed encryption.**
2. **§6/F6: Backup exclusion policy.** `app/src/main/AndroidManifest.xml` sets `allowBackup=false` and references `full_backup_exclusions.xml` and `data_extraction_exclusions.xml`. Both legacy and cloud/device-transfer policies exclude the private root. `app/src/androidTest/java/com/ekkus/silentdisco/app/BackupExclusionInstrumentedTest.kt` checks the installed merged manifest and XML on the API29 emulator. This does NOT prove real backup/restore recovery or key re-enrollment.

## 2. Previous run completed: replay-protection scaffold and green CI

The immediately preceding task added `rust/silent-disco-core/src/transport/secure_replay.rs` and five focused tests. It contains:

- Bounded **128-packet replay windows** supporting out-of-order traffic while rejecting duplicates and too-old sequences.
- Replay isolation by verified listener fingerprint, traffic-key generation and datagram purpose (audio or synchronization).
- Retired/rotated generation rejection, bounded **512-entry** peer/channel map and sequence/counter handling.
- A separate candidate check and *post-authentication* commit operation, so a failed tag must not poison the replay window.

**Critical limitation:** These are APIs for a future authenticated datagram receiver. They do not implement TLS, traffic-key derivation, AEAD authentication, encrypted UDP sockets, or production authenticated admission. The current `production_transport_factory()` still selects the legacy socket transport. Do not close the F2 datagram item merely because replay-window unit tests pass.

The previous session had left Android CI running. It is **now confirmed fully passed**, on the exact tested baseline `809de98e0fab8febd255bf7e46786097383b6321`:

| GitHub-hosted workflow | Run | Exact SHA | Result |
| --- | --- | --- | --- |
| Rust + Android CI (Rust quality, Android build/JVM/lint, API29 instrumentation, publication gate) | [40781020309](https://github.com/ekkus93/silent_disco/actions/runs/40781020309) | `809de98e0fab8febd255bf7e46786097383b6321` | **Completed: success** |
| Desktop CI (frontend, Rust quality, Linux bundle smoke) | [40781020211](https://github.com/ekkus93/silent_disco/actions/runs/40781020211) | same | **Completed: success** |
| Desktop Lab live transport | [40781020385](https://github.com/ekkus93/silent_disco/actions/runs/40781020385) | same | **Completed: success** |
| Source-file limit | [40781020278](https://github.com/ekkus93/silent_disco/actions/runs/40781020278) | same | **Completed: success** |

**CI SHA caveat:** The above outcomes certify `809de98e0fab8febd255bf7e46786097383b6321`, NOT the subsequent documentation-only commit containing this handoff and TODO box reconciliation. Check the new `master` SHA and its own workflows independently before reporting new exact-head success.

## 3. Existing partial engineering progress (do not confuse with closed gates)

- **Playback core / regressions:** Production WAV decode -> playlist packetizer -> TCP/UDP -> listener scheduling/render ring -> native C-ABI callback tests now cover two consecutive track-boundary Pause/Resume cycles and same-identity reconnect during Pause with exact virtual PCM continuity and no native output reopen. Earlier real-socket multi-listener Stop/replay/late-join tests also exist. These are NOT physical DAC/speaker measurements, and the security migration must repeat end-to-end tests under encryption.
- **F1 identity/admission:** `transport/secure_identity.rs` verifies single-use, domain-separated P-256 proofs bound to a *supplied* authenticated TLS exporter and derives stable key-fingerprint routing identities. `transport/secure_trust.rs` supplies host-scoped versioned trust records, revocation and offline enumeration; `runtime/secure_host_admission.rs` classifies verified joins and refuses revoked identities. These are **standalone scaffolding**; no production TLS channel provides the required exporter yet. No OS Keystore/desktop credential-store integration, FFI end-to-end admission, or enrollment/revocation UI proof.
- **F2 datagrams:** `transport/secure_replay.rs` accounts for replay after hypothetical successful AEAD verification. No implemented authenticated datagram transport.
- **F3:** Rust real-socket handling gained bounded authoritative admission-event delivery, visible failure/rollback under saturation, join deduplication and reconnect route protections, and associated loopback regression tests. Still need full control-event permutations and cross-layer stress with reliable error/UI outcomes.
- **F4/F5:** Android native teardown, close/cancellation, bounded event queues and critical-producer cancellation have dedicated JVM tests. Still need conclusive generation-lifetime stress of blocked in-flight native calls, rebind, approval, and multiprocess restoration.
- **F6:** Installed backup policy is excluded and instrumented; restore detection/key mismatch, corruption, identity rotation, and re-enrollment UX remain open.
- **UI/visual:** Android Compose layout/navigation/accessibility and desktop actual pixel-golden acceptance remain open; DOM tests are not pixel captures. Material 3 Expressive, focus, reduced motion and insets need explicit qualification.

## 4. What remains and next implementation sequence

The canonical checklist is the source of truth for every still-open task (55 software + 3 deferred). Sequence by production value:

1. **F2/F1 secure host and listener control transport:** Implement a real TLS 1.3 socket boundary using a vetted stack, host pinning/first-use identity, exporter-bound single-use listener proof, explicit v4 wire negotiation and no plaintext downgrade. Wire `VerifiedJoinAdmission`/host-owned trust through Rust actor/FFI and Android/desktop host views. Add attacker-negative and host-key-mismatch tests on actual loopback TLS sockets. **Do not present legacy-v3 as secure.**
2. **F2 authenticated datagrams:** Derive separate per-peer/direction/purpose secrets from the actual TLS session; use vetted AEAD, unique nonces, authenticated metadata and `SecureDatagramReplay` *only after* tag verification. Cover tampering, replay, old generation, seq exhaustion, Stop/replay/reconnect and teardown. Repeat exact-PCM TCP/UDP -> scheduler -> native callback acceptance and benchmark overhead.
3. **F1 trust lifecycle and platforms:** Provide Android Keystore and desktop protected-key adapters, durable host-scoped trust load/save/migration, Approve once/Always allow/revoke UX, first-use/manual fingerprint confirmation, profile-switch/restart and lost-key recovery. Verify Android/desktop parity; reject spoofed IDs and unknown/revoked keys in approved-only mode.
4. **F3/F4/F5 pressure and native safety:** Exercise actual socket + Rust actor + Android native bridge with admission/control saturation, reconnect, Stop, stale approvals, blocked fake native calls, concurrent close/rebind and 100+ iteration lifecycle stress. Assert bounded queues, finite visible errors, no cross-generation FFI, and no output reopen on same-stream resume.
5. **F6 restore and UI:** Add corrupt/missing-key restore simulation and re-enrollment UX; finish Android Compose navigation, process/config restore, API29 screen/font/landscape/system inset/focus tests and Android screenshot/golden artifact workflow. Add GitHub-hosted desktop *pixel* screenshot tests rather than claiming DOM evidence is equivalent; cover reduced-motion and focus/keyboard semantics.
6. **Hardening and closeout:** Refactor narrow ownership boundaries where test coverage requires it; produce concise acceptance index and append-only evidence ledger for the original 167 KB gapless TODO; add fuzz/property testing, pinned dependency/SBOM/security audits, and a final F1–F6 evidence matrix. Repeat **all** Rust, Android API29, desktop, hosted bundle, security and source-policy CI on the same final master SHA before checking §9 or closing software acceptance.

### Explicitly deferred; leave every §10 box unchecked

Physical speaker/headphone/DAC/Bluetooth recordings, cross-physical-device Android playback, packaged desktop-to-physical-Android listener, hands-on screen-reader and manual accessibility, and owner-network/device testing require separately authorized evidence. Do **not** block independent software tasks on them; do **not** simulate acceptance or check them off.

## 5. Safe continuation protocol

At the next iteration, refresh `master` SHA, **entire** active TODO/spec, `AGENTS.md`, relevant source, and **exact-head** CI using Ralph Bridge; inspect actual failing job logs if required. Reconcile concurrent master updates before every non-force compare-and-swap commit. Work directly on `master` with **no per-task branches/PRs**, exclusively via Ralph Bridge; use GitHub-hosted runners. Prefer integrated production behavior and substantial negative/end-to-end tests over extra comparator hardening. Keep checklist checkmarks and this handoff evidence grounded in exact-head behavior and known limitations.

The security migration is intentionally not declared complete. This handoff is a snapshot, not permission to stop the hourly loop while actionable software work remains.

## 6. 2026-10-28 follow-up — fail-closed legacy trust UX

**Code head:** `a41df7eaec9ad34f6ab00fd62c5a803222ae8b24`. This is a concrete production UX safeguard while v4 authenticated identities remain unavailable; **not** a claim that F1/F2 secure admission is complete.

- Android Host Access now disables the **Approved devices only** selection on legacy plaintext v3, describes the missing verified identity prerequisite, and refuses Next for a restored draft with that unsupported mode. Host Ready also refuses to start such a restored draft. Compose API29 instrumentation now covers disabled semantics, saved-state restoration, recovery by switching to manual approval, and the Host Ready block. A user is not silently moved into manual mode.
- Desktop Host Setup disables the `trusted_devices` option and prevents **Remember approved devices** from being newly enabled. An existing saved draft with either legacy flag remains visible, can reach Access for repair, but cannot proceed to Ready or start until the user explicitly chooses manual/invite-code approval and clears remembered trust. A React integration test exercises that recovery; the accessibility-outline golden was updated for the intentional warning and disabled checkbox.
- These changes do **not** provide v4 TLS/AEAD, authenticated key persistence, or secure approved-device auto-admission. The insecure mode is disabled rather than falsely advertised as working. The broader original gapless playback work remains unchanged; owner-deferred hardware boxes remain unchecked.
- **CI on code SHA:** Desktop CI [40788508768](https://github.com/ekkus93/silent_disco/actions/runs/40788508768) **passed** (frontend, strict Rust, Linux bundle). Source policy [40788508772](https://github.com/ekkus93/silent_disco/actions/runs/40788508772) **passed**. Main Rust/Android CI [40788508753](https://github.com/ekkus93/silent_disco/actions/runs/40788508753) was **still in progress** at this checkpoint: Rust quality completed successfully, Android build and API29 instrumentation still running. **Recheck this exact code SHA before marking Android qualification successful.** Previous intermediate code SHAs failed frontend formatting and a recovery-flow/golden regression; both were corrected before the current code head. No active TODO checkbox has been accepted from this partial qualification.

## 7. 2026-10-28 follow-up — Android bridge generation cleanup

On the current remediation baseline, `BoundedTransportEventQueue` already filtered retired events at collection time but retained their bytes/slots until a collector resumed. With a stopped collector and a full queue, a new native host/listener session could have its authoritative join/Stop/approval backpressured behind old-session notifications. This change drains queued retired events while the native handle gate serializes close/rebind, after cancelling the retired poller and before starting the new one; it also restores the best-effort quota. Host and listener controllers both invoke the cleanup on rebind and close/disconnect, including when native retirement throws. Already-delivered events remain subject to the existing generation check.

New JVM coroutine tests exercise a full retired queue containing critical and audio notifications, a replacement session's join/audio/Stop delivery without a collector during rebind, and cancellation of a suspended old-session critical producer. This is **not** proof of physical Android output, cross-layer F3/F5 saturation, or secure-v4 admission. Keep the F5 checklist open pending full bridge/native loopback qualification and exact-head Android CI.

## 8. 2026-10-28 follow-up — join-send failure and terminal error visibility

- Android listener join sends now fail explicitly if Stop/disconnect/rebind retires the native handle before the IO dispatcher delivers the request. Previously `withHandle` returned `null` and `sendJoinRequest` returned normally; the caller logged “sent” and could wait indefinitely for approval. The listener network effect now clears its connected flag, retires the failed socket, and reports a retryable Rust actor failure; coroutine cancellation remains cancellation rather than being mistaken for a transport failure.
- Android host treats `PeerAccepted` as an authoritative event at the bounded bridge rather than dropping it with heartbeat/telemetry. Rust host and listener TCP readers now carry validation errors into the authoritative `PeerDisconnected` event even if the separate diagnostic `Rejected` event is dropped under UDP pressure. No real-time audio callback is blocked by these changes.
- New JVM regression tests cover join attempts before connection and after disconnect; existing 128-iteration host/listener native-generation gate tests cover stale expected handles. This is targeted F3/F4 progress, **not** full F3/F4/F5 cross-layer pressure acceptance or secure-v4 transport. Keep their checklist items open pending exact-head CI and real-socket qualification.

## 9. 2026-10-28 follow-up — fail-closed durable trust on plaintext v3

The Android host native transport now rejects `trustedForFuture=true` in both the direct approval and combined approval/route-authorization APIs, before touching any native handle. Manual one-session approvals remain possible; a self-declared listener ID cannot be promoted to persistent trusted identity. The Android Rust storage-effect adapter also reports `PersistTrustedDevice` as a nonretryable failure instead of writing a claimed ID to the legacy `domainStore` and falsely acknowledging durable trust. The Rust actor remains authoritative and receives an explicit failure rather than a fabricated success. JVM tests cover both native approval entry points and the allowed session-only case. This is a fail-closed **v3 interim guard**, not v4 authenticated enrollment or a claim that F1/F2 is complete. The guard must be replaced only when v4 cryptographic proof, OS-backed key storage, and platform acceptance are implemented. CI evidence for this follow-up must be checked on its exact code SHA; no checkbox is closed by source changes alone.

## 10. 2026-10-28 follow-up — bounded pre-join TCP admission lifetime

The production host TCP reader now expires an accepted socket that has not forwarded its first JoinRequest within the configured operation timeout. A silent peer or a partial-header trickle can no longer occupy a host peer slot indefinitely. The timeout is absolute from reader start, is surfaced as a typed terminal disconnect, and does not apply after a valid join is forwarded (manual approval remains asynchronous). The listener TCP reader is unaffected. A real loopback regression uses a one-peer host, silent and partial-header clients, then a legitimate listener whose manual approval occurs after the pre-join deadline. This advances F3 capacity/terminal reliability but does not provide v4 authentication, anti-DoS rate limiting, or close the full F3 gate. Exact-head CI must be verified before acceptance.

## 11. 2026-10-28/29 follow-up — manual listener viewport and saved-state action reachability

**Code SHA:** `9ff63776ea7d6404641e26530e1252620a224a69` (direct `master`, Ralph Bridge exact-head CAS). Android `ManualEndpointScreen.kt` previously placed Connect and Cancel/Back beneath the entire vertically scrolling payload/warning/progress area. At 320×400 dp and enlarged font, the user could lose the cancel action below the visible viewport while the IME was open or connection status was lengthy. The screen now scrolls just the data/warning/status area while keeping the primary action and Cancel/Back in a fixed, at-least-48-dp action area above the keyboard. Approved/Streaming states retain their existing no-local-action layout, so playback ownership is unchanged.

**New Android API29 Compose instrumentation:** `ManualEndpointViewportTest.kt` checks that Connect (disabled without a valid payload) and Back remain displayed at 320×400 dp with 1.8× font after scrolling the LAN warning; and that an in-progress connection preserves a visible Cancel through saved-state restoration at 1.6× font, then returns to disabled Connect and Back upon recovery to Idle. The fixtures use Compose UI semantics, not pixel snapshots or physical accessibility measurements.

**Qualification snapshot:** Source-file policy [40832940969](https://github.com/ekkus93/silent_disco/actions/runs/40832940969) **passed** on the code SHA. Rust/Android build, JVM/lint and API29 instrumentation [40832941240](https://github.com/ekkus93/silent_disco/actions/runs/40832941240) were **still in progress** when this documentation was written; do not infer success. The older `5371919b` Rust job failed a strict Clippy nested-conditional lint and a congestion test after adding the pre-join timeout; a concurrent session corrected those in `dc8f6291`. The later code CI must be checked directly before upgrading this acceptance. F1/F2 secure-v4, visual goldens, complete process/navigation/keyboard/landscape matrix and all physical-device acceptance remain open.

## 11. 2026-10-29 follow-up — real packaged desktop WebKit pixel capture

**Code commit:** `145af4b5e4fc515f4df3a20945aae0ab2585de3d` (`test(desktop): capture packaged Tauri WebKit pixels on hosted Xvfb`). This adds `desktop/scripts/capture-tauri-webview.sh` and a dedicated GitHub-hosted Linux bundle workflow step. The script launches the actual packaged AppImage inside a clean XDG profile and Xvfb/DBus session, finds the mapped `Silent Disco` WebKit2GTK window, requires at least 720×520 visible pixels and more than two distinct colors, and saves the rendered PNG plus diagnostics. It does **not** substitute a Chromium browser render or a DOM/accessibility snapshot.

**Exact code-SHA desktop evidence:** [Desktop CI run 40833468173](https://github.com/ekkus93/silent_disco/actions/runs/40833468173) **completed successfully** on `145af4b5e4fc515f4df3a20945aae0ab2585de3d`. The packaged WebKit screenshot step itself completed successfully and printed **1000×700 pixels, 3,083 colors**, PNG SHA-256 `4ed8c83e8e5eaa2b716361d09c23a714989ca14be73ba288226743b42dd72d9c`. GitHub Actions uploaded the `desktop-tauri-webview-pixels` artifact (ID `12879772124`, 79,606 bytes), with a seven-day retention policy. The same desktop workflow passed frontend, desktop Rust, and Linux bundle jobs; source-file policy [40833468164](https://github.com/ekkus93/silent_disco/actions/runs/40833468164) passed. Main Rust/Android CI [40833468227](https://github.com/ekkus93/silent_disco/actions/runs/40833468227) was **still running** at this checkpoint and is not counted as passed.

**Remaining acceptance:** Pixel capture is initially `continue-on-error` to expose hosted-runner issues without hiding unrelated bundle regressions. This first successful capture proves a real packaged WebKit window can be rendered in CI, **not** approved visual parity or the full desktop golden workflow. Follow up by making pixel capture a required gate after repeatability checks, exercising reviewed host setup/live/error states, adding review-controlled pixel baselines and diff artifacts, and recording human baseline approval. The remediation §8 screenshot item and original gapless visual acceptance remain unchecked. Android screenshots and physical-device evidence are separate and deferred only where explicitly hardware-dependent.


## 12. 2026-10-29 follow-up — required packaged WebKit default/compact pixel smoke

The Linux bundle job now treats actual packaged Tauri/WebKit2GTK pixel capture as a **required** CI step rather than advisory. It captures both the initial 1000×700 viewport and a resized 760×560 compact viewport in the same clean Xvfb/XDG profile, validates geometry and a nontrivial pixel-color threshold, and uploads both PNGs and capture diagnostics. A failed launch, resize or blank render now fails the Linux bundle gate; artifact upload remains best-effort. This extends the earlier single-startup capture without claiming a visual golden, human-approved baseline, complete host/client navigation, or physical accessibility. **Keep §8 and §9 acceptance boxes open** until full reviewed-state visual evidence and exact-head qualification exist.

## 13. 2026-11-05 — F4 native-handle call-site and ownership map

Read from current master (`d3c18283` plus local test additions); this is a map of existing behavior, not new code.

**Ownership model.** `HostTransportController` and `ListenerTransportController` each own one `HostNativeHandleGate` (`core/rust/HostNativeHandleGate.kt`; the listener uses the `ListenerNativeHandleGate` alias). The gate holds one command monitor, an `AtomicReference` current handle, identity-keyed poll leases and a table of deferred closes.

- **Commands** (send, approve, authorize, broadcast, sync, attach/detach): `withHandle` captures `currentHandle()` on the caller's thread, hops to the controller's call dispatcher (`callDispatcher`, default `Dispatchers.IO`, injectable on the host; listener uses `Dispatchers.IO`), then runs under `withExpected(captured)`. A command whose captured generation was retired or replaced returns `null` instead of running on the replacement. Callers must treat `null` as failure; the listener join/Leave senders now throw on it.
- **Polls**: one `scope.launch(Dispatchers.IO)` loop per bound handle. `withExpectedPoll` takes an identity lease outside the command monitor so shutdown can interrupt it. Events are tagged with their source handle in `BoundedTransportEventQueue` and rechecked at the consumer (`isCurrentGeneration`, `collectCurrentEvents`), so a retired poll cannot mutate the next session.
- **Lifecycle**: bind/connect use `replaceIf` (caller token check, retire old, open, start poller, all under the monitor; a failed start retires the new handle). Close/disconnect use `close`/`closeIf`. `retireNativeHandle` runs shutdown, awaits poll quiescence (5 s), then closes; if a poll will not exit, the close is deferred to the final lease and the failure is surfaced, never closing a handle still in use. `close()` is idempotent because the gate clears the current handle before retiring it.
- **Approval + route authorization** is one operation on one generation (`deliverAndAuthorize`), using the route's recorded `sourceHandle` as `expectedGeneration`. Pending routes are keyed by actor request ID (`PendingHostJoinRoutes`), not the claimed device ID.

**Host call sites (app layer).** bind: `MainViewModelRustHost.kt` (`bindIfRequested`); event collection and generation check: same file; approve + authorize, rejection, disconnect: same file (`executeRustTransportEffect`); join rejection on projection failure: `MainViewModelRustHostEvents.kt`; broadcasts: `MainViewModelHostActions.kt` (start, pause, stop) and `MainViewModelHostPlayback.kt` (audio, stop, boundary); sync response: `MainViewModelRustHostSync.kt`; resync replay: `MainViewModelRustHostResync.kt`; close: `MainViewModelRustHostRecovery.kt` and `MainViewModel.kt` (`onCleared`).

**Listener call sites.** connect, join request and disconnect: `MainViewModelRustListenerNetwork.kt` (join-failure cleanup attaches a cleanup failure as suppressed to the original error and reports it to the actor); sync requests: `MainViewModelSynchronization.kt`; sync report: `MainViewModelRustListener.kt`; playback attach/detach: `MainViewModelRustListenerPlayback.kt`; event consumption: `MainViewModelRustListener.kt` (`collectCurrentEvents`); close: `MainViewModel.kt`.

**Test map.** Blocked in-flight send vs close/rebind: `HostNativeHandleBlockedSendTest`, `HostNativeHandleGateTest` (including 128-iteration rebind loops and hung-poll quarantine), `CapturedNativeHandleGenerationTest`. Superseded binds: `HostNativeHandleConditionalBindTest`, `ListenerConnectionGenerationGateTest`. Stale approval and retired events: `HostTransportGenerationAdmissionTest`, `HostTransportEventGateTest`, `PendingHostJoinRoutesTest`, `MainViewModelRustHostEffectTest`, `MainViewModelStaleApprovalTest`. Idempotent close: `HostTransportControllerIdempotentCloseTest`. Real native rebind (API29 instrumentation): `HostTransportSessionRebindInstrumentedTest`, `HostAdmissionIsolationInstrumentedTest`, `HostTransportLateJoinInstrumentedTest`. **Limits:** the unit tests use fake or mocked native handles, so they prove ownership logic, not native behavior; repeated real-native rebind stress is covered by `HostTransportRebindStressInstrumentedTest` (25 cycles with retired-event ghost joins, stale-approval and bounded-time checks), which has been run locally on a physical A54 and on the API29 emulator, not yet in hosted CI. Do not run `connectedDebugAndroidTest` with a phone attached unless that is intended: it targets every attached device (pin `ANDROID_SERIAL`) and uninstalls the app and its data afterward.

## 14. 2026-11-05 — F5 Android bridge queue and memory inventory

Read from current master. A search of Android main code finds **no `Channel.UNLIMITED`**: earlier work already removed them. This is the inventory of every remaining buffer between native/Rust producers and Android consumers, and of peer-controlled collections.

| Buffer | Producer | Consumer | Bound | Overflow policy |
| --- | --- | --- | --- | --- |
| `BoundedCoreNotificationQueue` (host core: `HostCoreController`; listener: `ListenerCoreNotificationMailbox`) | Rust actor observer thread | Android core-notification collector | 64 entries | Snapshots are conflated into a `StateFlow` outside the queue. Diagnostics are lossy and limited to a quarter of the slots, counted as `droppedDiagnostics`. Critical effects (transport/approval/Stop) apply bounded backpressure on the observer thread and fail after 2 s (`criticalTimeouts`), never silently. |
| `BoundedTransportEventQueue` (`HostTransportController`, `ListenerTransportController`) | IO native poll coroutine, one per bound handle | `eventsWithGeneration` / `collectCurrentEvents` | 64 entries | Critical events (join, disconnect, approval, Stop, resync) suspend the IO poll coroutine and keep FIFO order; best-effort telemetry and PCM notifications are limited to a quarter of the slots and counted (`droppedBestEffort`). Entries carry their native handle and are dropped at rebind, close and consumer handoff, so a retired generation cannot deliver. |
| `WorkflowViewModel.effectChannel` | UI workflow calls on the main thread | UI effect collector | `Channel.BUFFERED` (64) | `trySend` failure throws unless the ViewModel is cleared, so overflow is loud. |
| `PendingHostJoinRoutes` | Host join events | Approval/rejection effects | 128 routes, keyed by actor request ID | A full table refuses the new join with a busy refusal; consumed on use; cleared on close/rebind. |
| `MdnsDiscoveryLedger` (new, `MdnsDiscoveryService`) | NSD discovery callbacks (peer-controlled names) | NSD resolve (one at a time), session list | 64 pending resolves, 64 discovered sessions, 256 remembered lost names | The newcomer is refused, never an existing entry. The first refusal is reported once through the `failures` flow ("Nearby session list is full..."), later refusals are counted (`rejectedPendingCount`, `rejectedSessionCount`). Before this change all four collections were unbounded. |
| `MutableSharedFlow(extraBufferCapacity = 8)` failure streams (`MdnsDiscoveryService`, `BleDiscoveryService`) | Discovery callbacks via `tryEmit` | `MainViewModelTransport` collectors | 8 | Failures are lost if the buffer is full or nobody is collecting; both streams do have a collector, so this is a burst-only risk. Not changed. |

**Findings still open.** (1) The queue drop/timeout counters (`droppedDiagnosticEvents`, `droppedAudioEvents`, `droppedDiagnosticNotifications`, `criticalNotificationTimeouts`) exist as properties but nothing outside the queues reads them, and the host core notification queue's counters are not exposed by `HostCoreController` at all, so they never reach a diagnostics screen. (2) The discovery `failures` streams can drop a burst of more than eight failures. (3) `MdnsDiscoveryService` wiring is covered by a mocked-NSD flood test (`MdnsDiscoveryServiceFloodTest`) and the ledger by `MdnsDiscoveryLedgerTest`; neither exercises a real NSD stack.

**Update (counters now surfaced).** The bridge pressure counters are exposed on the core and transport interfaces (`HostCoreController` and `ListenerCoreController` gained `droppedDiagnosticNotifications` and `criticalNotificationTimeouts`; `ListenerTransport` gained `droppedAudioEvents`; `HostTransportController.droppedDiagnosticEvents` already existed), carried as `BridgeQueueCounters` on `HostDiagnosticsSnapshot` and `ListenerDiagnosticsSnapshot`, refreshed by `refreshHostDiagnostics` and `refreshListenerDiagnostics`, and printed in the support report ("Host/Listener bridge queues: telemetry/PCM drops=..., diagnostic drops=..., critical timeouts=..."). `uiStateForSupportReport()` reads the counters again when a report is built, because they change when a collector stalls and not when a refresh happens to run. The counters are lifetime values of the controller instances (reset by process restart), contain no identifiers, and are not shown on a dashboard screen, only in the report.

**Process restoration (reasoned, not tested).** No bridge queue is persisted. Process death takes the native sockets, the Rust actor state and these queues down together; the restored `MainViewModel` builds fresh controllers with empty queues and rebuilds UI state from saved state and the Rust-owned database. A join or Stop that was queued at death is lost with its TCP connection, and the peer observes a disconnect. There is no test that kills and restores the process.

**F5 cross-layer qualification (2026-11-05).** Chain observed on a real native host with the Android event collector stopped: Android transport queue (64 slots, critical events suspend the IO poll) fills, the native poll stops being called, the Rust host event queue (256) backs up, and the first visible refusal (busy join rejection or closed connection) appeared after 83 accepted joins in a connect/join/disconnect loop. Throughout, an already-approved listener still received Stop, and after collection resumed all 83 accepted joins arrived in order with no gaps. Evidence: `HostBridgeSaturationInstrumentedTest` (API29 emulator, local) and `BridgeQueueRetentionTest` (JVM, 64 KiB payloads, measured retained bytes). Not established: UI rendering of the refusal on the listener, and any run in hosted CI.

## 15. 2026-11-05 — F6 Android backup/restore data inventory

Documentation deliverable for TODO §6 item 1. It maps what the code does today; it is not restore proof.

**Policy.** `AndroidManifest.xml` sets `allowBackup="false"`, `fullBackupContent="@xml/full_backup_exclusions"` (API 26-30) and `dataExtractionRules="@xml/data_extraction_exclusions"` (API 31+). Both XML files exclude `domain="root" path="."`; the Android 12+ rules exclude it for both `cloud-backup` and `device-transfer`. Backup and device-to-device migration are therefore denied on every supported API level by two independent mechanisms. Instrumented checks: `BackupExclusionInstrumentedTest`, `BackupPolicyInstrumentedTest`.

| Store | Location | Contents | Migrates? |
|---|---|---|---|
| Rust domain SQLite | `noBackupFilesDir/domain/` (`AndroidDatabasePathProvider`) | settings, trusted device IDs, session history | No (no-backup dir and root exclusion) |
| P2 SQLite | `noBackupFilesDir/domain/` (`AndroidP2Store`) | recent sessions, trusted host public keys and fingerprints | No |
| `device_identity` prefs | app-private SharedPreferences | random per-install `device_id` (an unauthenticated label, not a key) | No (root exclusion) |
| domain legacy prefs | `AndroidRustDomainStore` preferences | pre-Rust tuning and `trusted:` keys, read once for import and then cleared | No |
| audio grant ledger prefs | `AUDIO_GRANT_LEDGER_PREFERENCES` | persisted read-grant bookkeeping for selected audio URIs | No; grants themselves are not restorable |
| Android Keystore | `silent-disco-host-identity-v1` (EC P-256, `HostIdentityManager`) | host QR signing key; non-exportable | Never leaves the device by design |
| Staged host audio | `cacheDir` | bounded copies of selected tracks | No; cleaned at start |
| Diagnostics and callback captures | `getExternalFilesDir("diagnostics")` and `getExternalFilesDir(null)` | debug-only PCM captures, exports | Not covered by the app backup rules; external app storage is not part of Auto Backup |

**Findings (not yet changed).**
1. (Addressed in part, same day: replacement is now detected and reported once; see TODO §6 item 3.) `HostIdentityManager.keyPair()` silently generated a new key if the alias was missing. A replaced key changes the host fingerprint. Listeners that trusted the old key fail closed: `P2UiState.validatedHostIsTrusted()` and `trustedVerifiedSessionIds()` require both fingerprint and public key to match (`P2PresentationTest` covers a mismatched key). The host side shows no "your identity changed" notice.
2. Trusted-device records in the Rust store are keyed by the unauthenticated claimed `device_id` (legacy v3), so there is no key-bound trust record to invalidate on a restore. Durable trust is blocked until the authenticated transport exists (TODO §1/§2).
3. No test simulates a restore (DB present, Keystore key missing or replaced). That is TODO §6 item 4.

## 16. 2026-11-07 — F2 and F3 call-path and protocol map

Documentation deliverable for TODO §0 item 3 (F2 and F3; F4, F5 and F6 are in §13–15). It records what the code does today. It is not a security proof.

**F2 — transport security (current state, verified in code).**
- **Protocol version.** `PROTOCOL_VERSION = 3` (`rust/silent-disco-core/src/protocol/types.rs:8`). Decoding rejects any other version (`protocol/codec/decoding.rs:329`). There is no version negotiation and no downgrade path, because there is only one version.
- **Transport encryption.** No Cargo manifest in the workspace or the desktop crate depends on `rustls`, `native-tls` or `openssl`. Control and audio frames are plaintext on the wire. The TLS 1.3 requirement in §1 item 1 is unimplemented.
- **Identity in frames.** `JoinRequest` and the other control messages carry a self-declared `device_id` (`protocol/types.rs:156`). No signature or key is bound to it on the wire.
- **Secure building blocks exist but are not wired.** `transport/secure_identity.rs` (possession proof), `secure_replay.rs` (replay windows), and `secure_trust.rs` (host-owned trust snapshot) are declared as `pub mod` in `transport/mod.rs`. Their only callers are their own unit and fuzz tests. No desktop (`desktop/src-tauri`) or Android (`app/src/main`) production path references them. Therefore no production join, admission or datagram path uses them.
- **Signed invitations.** The Android P2 flow creates and validates a signed QR (`P2ViewModel` → `P2RustBridge.prepareUnsignedQr` / `finalizeQr` / `validateInvitation`), and the host signs with an Android Keystore key (`HostIdentityManager`). This proves the invitation's origin to the app. It does not authenticate the transport, and join admission does not check it.
- **Consequence for F1.** Approved-devices-only admission cannot be strengthened by a key-bound check until the secure modules are wired into the join path. Current protection is the claimed-ID approval in §0.2 and F1.

**F3 — authoritative transport events (current state).**
- **Join path.** Desktop and Android transport inputs go through bounded queues: `BoundedTransportEventQueue` and `BoundedCoreNotificationQueue` on Android (64 slots, a quarter reserved for best-effort traffic, critical events apply backpressure with a 2 s timeout), and a bounded Rust transport event channel (`DEFAULT_TRANSPORT_EVENT_CAPACITY = 256` in `transport/types.rs`, created with `sync_channel` in `transport/socket/listener.rs`; max 8,192). Busy refusal is a protocol `JoinRejection` with the reason "Host busy" (verified by `real_socket_busy_join_is_refused_then_retry_rejoins_live_pcm`). Duplicate join suppression is per TCP generation (`duplicate_join_tests.rs`, counter `duplicate_join_suppressions`).
- **Counters.** Best-effort drops, busy refusals and authoritative-event failures are exposed through FFI counters (`FfiHostTransportCounters`, `FfiListenerTransportCounters`) and the Android `BridgeQueueCounters` diagnostics (§14).
- **Client retry.** The client does not retry automatically after a busy reply. Retry is user-driven. This is the open owner decision for §3 item 2.
- **Not yet changed by this map.** Nothing in this section changes behavior.

## 17. 2026-11-07 — F2 control-channel TLS 1.3 with pinned host key (rustls)

Implementation slice for TODO §1 item 1. It is the TLS configuration and its proof, not yet wired into the control socket.

**Choice.** `rustls =0.23.46` with the `ring` provider, TLS 1.3 only (`default-features = false`, so TLS 1.2 is not compiled). Pure Rust, no OpenSSL. `rcgen =0.14.2` is a dev-dependency only, for test certificates.

**Design (`rust/silent-disco-core/src/transport/secure_control_tls.rs`).**
- The host's identity is the SPKI of its P-256 key, the same `publicKeyDer` shown in the signed invitation. There is no CA, because the app works offline.
- The listener's verifier pins that SPKI. Authentication comes from the TLS 1.3 `CertificateVerify` signature: it must verify under the pinned key (`rustls::crypto::verify_tls13_signature_with_raw_key`). The presented certificate's chain and names are not checked. That is deliberate: the certificate only carries the key, and only a holder of the pinned private key can produce a valid signature over the handshake transcript.
- TLS 1.2 signatures are refused outright.
- The host configuration takes any rustls `SigningKey`, so a platform-protected key can sign without being exported. A Keystore-backed signer is not built yet.

**Evidence (local).** `secure_control_tls_tests.rs` runs real handshakes over loopback TCP:
- a listener pinned to the host key completes TLS 1.3 and echoes bytes (the negotiated version is checked);
- a listener pinned to a different key refuses a host that presents a valid self-signed certificate;
- five successive handshakes succeed.
- Mutation: replacing `verify_tls13_signature` with an unconditional accept makes the mismatched-key test fail. Restored; passes.
- `scripts/check-rust.sh` exit 0 (23 suites; strict Clippy).
- `./gradlew assembleDebug` builds for all four Android ABIs, so `ring` cross-compiles (`buildRustAndroidDebug`). `./gradlew test lintDebug` does not run that task, so the cross-build needed the separate assemble run.
- `desktop/src-tauri/Cargo.lock` updated (additions only); `cargo metadata --locked` passes, which CI's `--locked` needs.

**Not done (TODO §1 item 1 stays open).**
- The control socket does not use this configuration yet, so protocol v3 plaintext is still the production transport.
- The host signing key for TLS is not Keystore-backed. Keystore can sign ECDSA-SHA256, which matches TLS 1.3 `ecdsa_secp256r1_sha256`, so a rustls `SigningKey` over `HostIdentityManager` is the natural next step, but it is unbuilt.
- No client (listener) authentication: client certificates are disabled. That needs the possession proof in `secure_identity.rs`, or mutual TLS, and is a separate item.
- Hostile-network tests (downgrade, forged host, tampered control) belong to §1 item 4 and are not written.
- No CI record for this commit; the owner watches CI.

## 18. 2026-11-07 — listener possession proof over TLS 1.3 (step 1 of the proof plan)

Implements the possession proof from §17's next steps, in core only, with in-memory keys. It is not wired into a socket and does not use a device key store yet.

**Design (`transport/secure_listener_proof.rs`).**
- After the TLS 1.3 handshake, the host sends a fresh 32-byte nonce from the provider's `secure_random`. The listener replies with its SPKI and a DER ECDSA signature over the canonical transcript from `secure_identity.rs`.
- The transcript binds the host key fingerprint, the listener key fingerprint, session ID, generation, nonce and a 32-byte TLS exporter value (`export_keying_material`, label `silent-disco listener proof v1`). A proof captured on one connection cannot be replayed on another, because both the nonce and the exporter differ.
- The caller computes the exporter value from the connection before the exchange, so the proof code never borrows the TLS connection and the stream together.
- Frames are two length-prefixed fields. A length of zero or above 256 bytes is refused before any allocation of that size.
- The host consumes the challenge on the first verification attempt (`PendingListenerProof::verify_and_consume`), so a failed proof cannot be retried.

**Evidence (local).** `secure_listener_proof_tests.rs`, five tests over real TLS 1.3 loopback connections:
- a listener proving its pinned key is verified, and the verified fingerprint equals the SHA-256 of the pinned host key;
- a listener announcing a victim's key but signing with its own key is refused (`SignatureMismatch`);
- a proof captured on connection one is refused on connection two (`SignatureMismatch`), after the same proof verifies on connection one;
- a valid proof is refused when checked against a different session ID;
- a 65,535-byte length prefix is refused as `MalformedFrame`.

Mutations, all restored and verified identical:
- Binding and nonce both made constant: the replay test fails.
- Signature check removed from `secure_identity`: three tests fail (forged key, replay, cross-session).
- Binding alone made constant: the replay test still passes, because the fresh nonce is enough on its own. So the exporter binding is defence in depth, and nothing in the test suite yet proves it works alone. This is recorded as a gap.

**Not done.**
- Listeners have no persistent P-256 key. Android listeners have a random device ID label (`DeviceIdentityStore`); desktop listeners likewise. Step 2 (key storage, Keystore on Android, owner decision on desktop) comes first.
- The proof is not wired into join admission, and the control socket is still protocol v3 plaintext.
- Nothing here is device evidence. Gate: `scripts/check-rust.sh` exit 0, 23 suites.

## 19. 2026-11-07 — desktop listener proof key: owner-only key file

Step 2 for the desktop listener, as chosen by the owner: a key file in the app data directory with owner-only permissions. This is storage only. It is not yet used by the listener proof.

**Design (`desktop/src-tauri/src/platform/listener_key_file.rs`).**
- File `listener-proof-key.v1`: the raw 32-byte P-256 scalar. Mode must be exactly `0600`, inside a directory with no group or other bits (`0700` when the directory is created).
- A missing file is created with `create_new` (never overwrites), written, synced, then `chmod`ed to `0600` so the result does not depend on the umask.
- Everything else is refused and the file is left unchanged: symlinked path, non-regular file, wrong mode (not repaired), wrong length, zero or out-of-range scalar, or a directory readable by others. A damaged key is never regenerated, because that would silently change the listener's identity, which trust policy must see as a change.
- Unix only. Other platforms fail closed with `UnsupportedPlatform`.
- Randomness comes from `getrandom`, already a dependency. No new crates.

**Relationship to the existing host identity.** `platform/identity.rs` already keeps the desktop host's secret in the OS keyring. The listener key deliberately uses a file, at the owner's choice. Two stores now exist. That is not a problem for correctness, but it is a difference the owner should know about.

**Evidence (local).**
- Seven tests: create and reload the same key (mode `0600`), create a missing directory as `0700`, refuse a `0644` key file and leave it unchanged, refuse a symlink without creating anything behind it, refuse a truncated file without regenerating it, refuse zero and `0xff…` scalars, refuse a `0755` directory.
- Mutations, both restored and verified identical: repairing a wrong mode instead of refusing it fails the permission test; regenerating a truncated key fails the truncation test.
- `cargo fmt --check` passes. `cargo clippy --all-targets -D warnings` passes. Full desktop library suite: 394 passed, 5 ignored (the existing manual device tests).

**Residual limits.**
- Time-of-check and time-of-use: the path is checked, then opened. An attacker who can write into the owner's `0700` directory could swap the file in between. Such an attacker already runs as that user, so the risk is narrow, but it is not closed. `O_NOFOLLOW` would close it, but it needs a platform-specific constant or `libc`, which I did not add.
- The key is stored unencrypted on disk. Protection comes from file permissions only.
- Nothing is wired to the listener proof yet, and the Android Keystore listener key (step 2, Android half) is not built.

## 20. 2026-11-07 — Android listener proof key in Android Keystore

Step 2, Android half. The listener's proof key is a non-exportable P-256 key in Android Keystore. It is not yet used by a listener proof flow.

**Design.**
- `core/identity/KeystoreEcKey.kt`: shared load-or-create for a Keystore EC key (P-256, SHA-256, sign and verify, no user authentication). `HostIdentityManager` now delegates its key loading to it. Behavior is unchanged, and its instrumented tests still pass.
- `core/identity/ListenerIdentityManager.kt`: the listener proof key, alias `silent-disco-listener-proof-v1`, separate from the host key. It exposes the SPKI DER and a DER ECDSA-SHA256 signature, which is the form the core verifier takes.

**Evidence (Samsung A54, Android 16, real Keystore).**
- `ListenerIdentityInstrumentedTest`, four tests: the public key is on the secp256r1 curve; signatures are DER and verify against the public key, and fail for another payload; the same alias keeps its identity across manager instances; the listener key differs from the host key; the private key's `encoded` is null.
- Mutation: a signer that ignores its payload fails the signature test. Restored; byte-identical.
- Cross-language: a signature captured from the phone's Keystore (72-byte DER, 91-byte SPKI) verifies in the core `p256` verifier, and the same signature is rejected for another transcript. Test `a_keystore_signature_from_android_verifies_in_the_core_verifier`.
- All nine identity instrumented tests pass after the refactor. `./gradlew test lintDebug` passes. `cargo clippy -D warnings` passes on the core crate.

**Not done.**
- The listener does not yet sign a proof on the TLS connection. That needs the Android listener's TLS client and the exporter binding wired through, which is a later step.
- The desktop listener proof key (§19) is not yet used by a proof flow either.
- The stored key is protected by Keystore, but the test runs were on one device. No emulator run.

## 21. 2026-11-07 — listener TLS proof flow: core and desktop; Android pending

**Signer abstraction (core).** `ListenerProofSigner` (in `transport/secure_listener_proof.rs`) supplies the listener's SPKI and signs the transcript. The proof code never needs the raw private key. A desktop key file or an in-memory key implements it, and an Android Keystore key can implement it through a callback once the FFI exists. `SigningKey` already implements it.

**End-to-end functions (core).**
- `connect_and_prove(address, host_spki, session, generation, signer)`: connects, completes TLS 1.3 pinned to the host SPKI, reads the host nonce, and returns a proof. It sets a 10 s timeout on connect and I/O.
- `accept_and_verify(listener, host_certificate, host_signing_key, host_spki, session, generation)`: accepts one listener, completes TLS 1.3, and verifies the proof. It returns the verified listener key.
- Errors are typed: `SignerFailed` is reported when the key can't be read or can't sign.

**Evidence (core, local).** `secure_listener_proof_tests.rs`, nine tests over real TLS loopback:
- an end-to-end proof verifies, and the verified fingerprint equals the SHA-256 of the listener's key;
- a listener pinned to another host key never reaches the proof (both sides fail);
- a claimed key the listener does not hold is refused (`SignatureMismatch`);
- a signer that can't read its key surfaces `SignerFailed` on the listener, and no proof verifies;
- a signer that reads its key but refuses to sign surfaces `SignerFailed`, and no empty proof verifies;
- a captured proof is refused on a second connection; a proof for another session is refused; an oversized field is refused as malformed;
- a Keystore-produced signature captured from the A54 verifies in the core verifier.

Mutation: swallowing signer failures (`unwrap_or_default`) passed the first version of the tests, because only the key-read failure was tested. A sign-only failure test was added; with the mutation in place it now fails. Restored, and verified identical.

**Desktop (`desktop/src-tauri/src/platform/listener_proof.rs`).** `prove_to_host` loads the owner-only key file (§19) and runs `connect_and_prove`. Two loopback tests: a desktop listener's key-file key is verified by the host and matches the stored key; a desktop listener refuses to prove to a host it did not pin. Dev-dependencies added to the desktop crate: `rcgen =0.14.2` and `rustls =0.23.46` (ring), the same pins as the core.

**Gates (local).** `scripts/check-rust.sh` exit 0 (23 suites; strict Clippy). Desktop: `cargo fmt --check` passes, `cargo clippy --locked --all-targets -D warnings` passes, and the new desktop tests pass. The desktop lock gained only added lines.

**Not done: Android.** The Keystore key cannot be exported, so the TLS client must run in Rust while each signature is requested from Kotlin. That needs:
1. a UniFFI callback interface in `silent-disco-ffi` for the signer, plus a function that runs `connect_and_prove`, with regenerated Kotlin bindings;
2. a Kotlin implementation that delegates to `ListenerIdentityManager`;
3. a host endpoint for an on-device test. The host side exists only as Rust test code. The options are a test-only FFI export of `accept_and_verify`, or a separate desktop host running on the same network.

**Not done: production wiring.** Neither the desktop nor the Android join path uses the proof yet. Both still run protocol v3 plaintext.

## 22. 2026-11-07 — Android listener TLS proof over the FFI (on-device)

Completes the Android half of §21. The Keystore listener key signs through a UniFFI callback, and the Rust listener runs TLS 1.3 pinned to the host key.

**FFI surface (`rust/silent-disco-ffi/src/listener_proof.rs`).**
- `listener_prove_to_host(address, host_spki, session_id, generation, signer)`: runs the listener side. `signer` is a `FfiListenerProofSigner` callback, implemented by the app.
- `FfiProofHostListener`: binds a port from a PKCS#8 P-256 host key. `host_spki()` is what listeners pin. `accept_and_verify(session_id, generation)` accepts one listener and returns the verified fingerprints. This is the host-side verifier a real host needs later, so it is not test-only.
- Core support: `host_tls_identity_from_pkcs8` derives the SPKI, self-signed certificate and signing key from one key, so they cannot disagree. `rcgen` moved from dev-dependency to dependency of the core for this.

**Android.** `KeystoreListenerProofSigner` implements the callback over `ListenerIdentityManager`. Failures reach Rust as `FfiBridgeException.Callback`, never as an empty proof.

**Evidence (Samsung A54, Android 16, real Keystore and TLS).**
- `ListenerProofOverFfiInstrumentedTest`, three tests: the Keystore listener proves and the host verifies, with the listener and host fingerprints checked; a listener pinned to another key never proves and the host rejects the failed handshake; a signature over the wrong transcript is refused.
- Mutation: breaking `KeystoreListenerProofSigner.signDer` to sign the wrong bytes fails the end-to-end test. Restored and verified identical.
- `./gradlew test lintDebug` passes. The line-count check passes. The Rust gate passes (23 suites, strict Clippy). The desktop lock is consistent under `--locked`.

**Environment note.** The first full Rust gate run hit "No space left on device" during compilation. A later run had one concurrency test (`shutdown_races_with_concurrent_command_submission_and_notification_delivery`) stall for several minutes under memory pressure (about 230 MB free). Run alone it passes five times in a row, and the full gate passed on a rerun. This is a timing-sensitive test that stalls under load, and it is recorded as a flake risk, not a fix.

**Not done.**
- Neither join path uses the proof yet. Both still run protocol v3 plaintext.
- The host object in this step is test-exercised; no production host uses it yet.

## 23. 2026-11-07 — owner-thread TLS frame channel (option 1, step 1)

First step of the owner's chosen design: the host and listener control connections will each be owned by one thread. The existing `TcpStream` reader/writer split cannot hold a TLS stream (it clones the socket), so the TLS record layer moves to one worker per connection.

**Design (`transport/secure_frame_channel.rs`).**
- One worker thread owns the TCP socket and the rustls `Connection`. Callers send frames through a bounded queue (64 slots) and receive through another. Nothing else touches the socket.
- Frames are `u16` length-prefixed, 1..=65,535 bytes. An oversized or empty frame is refused at `send`, before it reaches the wire.
- Bytes the TLS writer does not accept are staged and retried after each flush, so a large frame is never dropped or half-written.
- The worker polls every 10 ms, so a queued frame waits at most one poll interval. A close sends `close_notify`.

**Evidence (local, real TLS 1.3 on loopback, `secure_frame_channel_tests.rs`, four tests).** Frames flow in both directions over a pinned connection, including the largest legal frame, in order; a listener pinned to another key gets no application data; oversized and empty frames are refused and the channel still works; a clean close is observed by the peer.

Mutations, both restored and verified identical: dropping the partially accepted bytes fails the round-trip test (the large frame is lost); removing the send size check fails the refusal test.

**Gate.** `scripts/check-rust.sh` exit 0, 23 suites, strict Clippy.

**Not done (the rest of option 1).**
- The host's control path (`transport/socket/host_workers.rs`) still runs plaintext v3. The next step is to accept control connections as TLS, run the listener proof on the new channel, and only then forward the join to the actor.
- The listener's join path (`listener.rs`) and the Android listener's join are not switched.
- Admission still does not consult durable host trust. That is the F1 work, which follows the wiring.

## 24. 2026-11-07 — TLS control admission with the listener proof on the frame channel (wiring step 1)

**What this is.** `transport/secure_admission.rs` provides the two halves of the admission the host's control socket will use:
- `admit_listener(tcp, host_identity, session, generation)`: completes TLS 1.3 on the accepted socket (exporter binding taken after the handshake), starts the owner-thread channel, sends a fresh 32-byte nonce as a frame, reads the listener's response frame (exactly two length-prefixed fields, no trailing bytes), and verifies it with `PendingListenerProof`. It returns the verified listener and the open channel, only on success.
- `dial_and_prove(address, host_spki, session, generation, signer)`: the listener side. It connects, pins the host SPKI, answers the nonce from its signer, and returns the open channel for join traffic.

**Deadlines.** Connect, handshake and every frame are bounded at 10 s. A listener that never answers is dropped at that deadline.

**Evidence (local, real TLS 1.3 on loopback, four tests).**
- An honest listener is admitted with the expected fingerprint, and a message the host sends after admission reaches the listener over the same channel.
- A listener claiming another key is refused with a proof error before any channel is returned.
- A listener pinned to another host key never reaches admission.
- A listener that stalls the proof is dropped within the deadline (the test checks it does not wait the full stall).
- Mutation: removing the signature check in `secure_identity` makes the forgery test fail. Restored; identical.
- `scripts/check-rust.sh` exit 0, 23 suites, strict Clippy.

**Not done (the rest of wiring step 1).** The host's production accept loop in `transport/socket/host_workers.rs` does not call `admit_listener` yet, and the listener's join path does not call `dial_and_prove`. Both still run plaintext protocol v3. Switching the production accept loop is a protocol cutover: existing plaintext listeners (including the Android and desktop clients today) would be refused, so it needs a decision on whether the host runs a TLS-only port or both protocols side by side.

## 25. 2026-11-07 — option 2 dispatch: TLS and plaintext on the host's control port (wiring step 2)

**What is wired.** The host's accept loop (`socket/host_workers.rs`) now routes each new control socket through `socket/control_dispatch.rs` before the existing registration path:
- A connection whose first record is a TLS handshake (`0x16 0x03`, see `transport/control_protocol.rs`) goes to `admit_listener` on its own thread. Concurrent TLS admissions are capped at `max_peers` and released by a guard. The admission outcome is reported as a visible `Rejected` event: refused with the reason, or admitted with the channel closed, because the join path that would keep it open is not built yet.
- Everything else, plaintext v3 included, goes to the existing path unchanged. Silent, partial or malformed connections are not refused here: the plaintext decoder and the host's own timers decide, as before.
- `HostTransportConfig.secure_control` (optional `HostTlsIdentity`) enables TLS admission. `None` refuses TLS with a visible reason. The FFI host config defaults it to `None`.
- Classification waits at most 250 ms for the first bytes. A silent client is handed to the plaintext path after that, so its delay is bounded.
- Channel fix found by the new tests: dropping a channel stopped the worker before a queued frame was written, so a listener that sent its proof and dropped at once lost it. The worker now drains queued frames before close (regression test `a_frame_queued_just_before_the_channel_is_dropped_still_reaches_the_peer`).

**Evidence (local).**
- Protocol classifier: four tests. Frame channel: five tests. Admission: four. Dispatch on a real host: five tests (honest TLS admitted with the channel closed; forged key refused as `Unauthorized`; TLS with no identity refused with a reason; a silent or malformed connection left to the plaintext path; a silent client classified within the deadline).
- Mutation: routing TLS to the plaintext path fails three of the four TLS dispatch tests. Restored.
- Full core library suite: 606 passed, including every existing plaintext real-socket test. The FFI suite: 127 passed, with one test skipped (below). Workspace strict Clippy and `cargo fmt --check` pass.
- An earlier version refused silent and malformed connections; that broke 15 existing pressure tests, so it was reverted. The behavior-preservation rule is the reason plaintext is routed unchanged.

**Open problem, not fixed in this step: a hang in the FFI shutdown test.** `host_control::handle::tests::concurrency_tests::shutdown_races_with_concurrent_command_submission_and_notification_delivery` hung in 2 of 20 isolated runs (the test has a documented history of stalls). On a hang, every thread (test main, notification worker, actor) is blocked in a futex wait with near-zero CPU, and all submitter threads have already finished. That points to a lost or misordered shutdown in `actor_runtime::shutdown` (it queues `Shutdown`, then joins the actor, then joins the notification worker). It is in the FFI/core actor runtime, not in the socket code this step changes. I did not prove that it predates this step; I observed it in the full gate. The test is skipped only for the validation run in this step, not removed.

**Not done.**
- The host's real join flow does not use the admitted TLS channel yet. Admitted listeners are closed. Joins over TLS, durable trust, and the listener's join path remain.
- The 250 ms classification wait is a small accept-loop delay for silent clients. It is documented in the module.
- The actor-runtime shutdown hang above needs its own investigation before the full gate can be called green.

## 26. 2026-11-07 — fix: shutdown hang in the core actor (resolves the §25 open problem)

**Root cause.** `CoreActorRuntime::shutdown` swaps `accepting` to false, samples the snapshot revision, and queues a `Shutdown` command with that revision. The actor applied the optimistic revision check to every command except `EndHostSession`. If a concurrent command advanced the revision between the sample and the send, the actor rejected the stale `Shutdown` with `invalid_state`. `stop_requested` stayed false, the actor kept waiting on its input channel, and the main thread blocked forever in `join_actor`. The notification worker was waiting in the same state. The `/proc` wait channels from the hang (all threads in futex waits, submitters already finished) matched exactly.

**Fix.** `Shutdown` is treated as terminal, like `EndHostSession`, and is exempt from the optimistic revision check (`actor_runtime/state/mod.rs`). Shutdown has no state precondition, so the exemption is safe.

**Evidence.**
- Deterministic regression test `shutdown_is_honored_even_when_a_concurrent_command_moved_the_revision` (actor state, no threads). It fails on the old code and passes on the fix.
- The FFI test `shutdown_races_with_concurrent_command_submission_and_notification_delivery` passed 40 of 40 isolated runs after the fix. Before the fix it hung in 2 of 20 isolated runs, and in the full gate.
- Full `scripts/check-rust.sh`: exit 0, 23 suites, no failures, no tests skipped.

**Not claimed.** The fix addresses the stale-revision path. Other lost-wakeup or ordering races in the actor are not ruled out by these runs; the 40-run rate is evidence, not a proof.

## 27. 2026-11-07 — TLS join path, first part: protocol frames over the admitted channel

**What is built.**
- `transport/secure_control_session.rs`: `SecureControlSession` wraps an admitted channel bound to one session. Each channel message is one encoded protocol frame, the same bytes the plaintext v3 path uses. Outbound frames must belong to the session. Inbound frames are decoded with the session policy (`DecodePolicy.expected_session_id`), so a frame for another session is refused and the channel stays usable.
- The frame channel's length prefix is now `u32`, and the cap is the largest protocol control frame (`FRAME_HEADER_BYTES + MAX_CONTROL_PAYLOAD_BYTES`). The old `u16` prefix was too small for a control frame.

**Evidence (local, real TLS 1.3 on loopback, five tests).** A join request sent by the listener is decoded at the host with the right device and session, and the host's approval comes back on the same channel. The host refuses to send a foreign-session frame. A foreign-session frame arriving over TLS is refused, and the next valid join still decodes. Malformed bytes are refused as protocol errors. A silent peer times out. Mutations: removing the receive-side session check fails the foreign-frame test; removing the send-side check fails the send refusal test. Both restored.

**Full Rust gate:** `scripts/check-rust.sh` exit 0, 23 suites, nothing skipped.

**Not built yet (the rest of the join path).**
- Feeding these frames into the host actor. The plaintext path emits `PeerAccepted` and `FrameReceived` events with per-peer state; the TLS path does not emit them yet, so a TLS join does not reach the actor. Admitted channels are still closed by the accept loop (§25).
- Route authorization for datagrams (`PeerAuthorized`), and the approval-to-route handoff.
- The listener's own join state machine and its disconnect and reconnect behavior.
- The desktop and Android listeners still join over plaintext v3. Nothing in the app uses this path yet.
- Durable trust: a TLS join is still session-scoped, and the claimed device ID is not yet bound to the verified listener key.

## 28. 2026-11-07 — TLS join path, parts 2–3 (core): actor events, datagram routes, listener join

**What is built (core only; the desktop and Android apps do not use it yet).**
- **Actor events (item 1).** An admitted TLS listener is registered as a normal host peer (`socket/secure_peer.rs`). It gets the same peer-table entry, `PeerAccepted`, and join gating as a plaintext peer. The join frame reaches the actor as `FrameReceived { channel: Control }`, and the actor's approval goes back over the same channel through `send_pending_control`.
- **Shared handling.** The plaintext reader and the TLS pump now run the same code: `socket/peer_registry.rs` (shared handles and the `PeerState` constructor), `socket/peer_inbound.rs` (frame validation, one join per connection, busy refusal, ordered terminal disconnect), and `socket/secure_control_io.rs` (outcome mapping and write drain). The plaintext behavior is unchanged: the full core suite (613 library tests plus integration suites) passed after the refactor.
- **Datagram routes (item 2).** UDP authorization needed no new code. A TLS peer is in the same peer table and route map, so `authorize_peer` and the datagram receiver apply the same checks. Test: a TLS-joined listener's sync is accepted only from its authorized route; before authorization, and from any other socket, it is refused as `Unauthorized`.
- **Listener join (item 3, core).** `ListenerTransportConfig.secure_host` (a `SecureHostTarget`: the invitation's host key and a listener proof signer) makes `SocketListenerTransport::connect` dial TLS, pin the host, and prove the listener key. A pump then carries join, approval, sync and departure through `listener_inbound.rs` (the shared inbound handler). `None` keeps plaintext v3.

**Evidence (local, real TLS 1.3 on loopback).**
- Dispatch: join reaches the actor and the approval returns on the same channel; a foreign-session frame disconnects its peer.
- Datagrams: sync accepted only from the authorized route.
- Listener (`secure_listener_tests.rs`, five tests): a pinned listener joins and gets its approval; a listener pinned to another key never connects; sync over its authorized route reaches the host; a clean departure is seen by the host; a listener that leaves can rejoin with the same device and be approved again.
- Session: queued bytes for another session are refused on the byte path. Mutation (remove the decode check in `send_encoded`) fails that test; the file was restored and checked with `cmp`.
- Gate: `scripts/check-rust.sh` exit 0, 23 suites, nothing skipped. `cargo check` on `desktop/src-tauri` with `--locked --all-features --all-targets` exit 0. `check-source-file-line-counts.sh`: all 861 tracked files under 800 lines.

**Not built (the app half of item 3, and item 5).**
- *Hosts have no TLS identity.* Both the desktop and the Android host still set `secure_control: None`, so a TLS listener is refused with a visible reason. The desktop host can build its identity from the PKCS#8 key it already stores for invitations (`platform/invitation_identity.rs`). The Android host's key is non-exportable, so its TLS signing needs a Keystore-backed rustls signer; that is a design and device task.
- *Listeners do not pin yet.* The invitation carries `host_public_key_der`, so an app listener can pin it. The FFI listener config and the Kotlin controller do not pass `secure_host` yet; the Keystore proof signer exists (`KeystoreListenerProofSigner`).
- *Durable trust (item 5).* A TLS join is still session-scoped. The claimed device ID is not bound to the verified listener key, and no trust record is written.
- *Negative and device tests (§1 items 4–6).* Downgrade and tamper tests across the app, and end-to-end runs on the A54 and the desktop host, are not written.
- *Missing deadline test.* A TLS peer that never sends a join is cut off by the pump's admission deadline, but no test covers it yet. Post-join trickle is also unbounded (the TCP reader's frame deadline does not apply to the TLS channel). Both are noted for follow-up.

**Decision recorded.** Option 2 keeps plaintext working unchanged, so no cutover is needed. A listener should use TLS only when its invitation carries a host key, and a host should accept it only once its own identity is configured. Wiring the listener side before the host side would break joins, so the host lands first.

## 29. 2026-11-08 — Desktop host presents a TLS identity (remediation §1, host side)

**What changed.** The desktop host now presents a TLS identity on its control port, built from the same P-256 key that signs its invitations. A listener that pins the invitation's host key therefore proves the key the host actually holds. Plaintext listeners are unaffected, because the control port still routes non-TLS bytes to the plaintext path.

- `platform/invitation_identity.rs`: `DesktopHostSigningIdentity::host_tls_identity()` derives a `HostTlsIdentity` from the signing key. The PKCS#8 copy is zeroed after use and never leaves the process.
- `platform/network/host_control.rs`: `set_host_tls_identity` stores the identity in the network state. `start_host_with_sink` copies it into `HostTransportConfig.secure_control`.
- `app_state/construct.rs`: profile open derives and sets the identity right after the signing identity loads. Failure fails the open closed (`desktop.host_tls_identity.unavailable`), so the host never runs with TLS silently absent.

**Why this key.** The listener pins the SPKI carried by the invitation. Reusing the invitation key makes the pinned value and the proven value the same by construction. A separate TLS key would need its own distribution and would drift from the invitation.

**Tests (all real code paths).**
- `host_tls_identity_pins_the_same_key_the_invitation_carries`: the TLS SPKI equals the invitation's `public_key_der`.
- `host_tls_identity_is_stable_for_one_key`.
- `host_start_hands_the_host_tls_identity_to_the_transport`: the transport config received at bind carries the identity. Mutation (`secure_control = None`) fails it; restored and verified.
- `a_desktop_listener_proves_to_a_host_presenting_the_invitation_identity`: the desktop listener's `prove_to_host` verifies against the host identity built from the invitation key.

**Gates.** Desktop `cargo fmt --check` exit 0; `cargo clippy --locked --all-features --all-targets -- -D warnings` exit 0; `cargo test --locked --all-features` 407 passed, 0 failed, 5 ignored; `scripts/check-source-file-line-counts.sh` exit 0 (869 files). Core was not changed in this step. `npm run check` was not run.

**Not done (still open).**
- Android host: the key is non-exportable in Keystore, so its TLS signer needs a Keystore-backed rustls signing key. Still `secure_control: None`, so TLS listeners to an Android host are refused with a visible reason.
- Listener app wiring: the FFI listener config and the Kotlin and desktop listener join paths do not yet pass `secure_host` (invitation key plus signer). Listeners still join over plaintext.
- Trust binding (item 5), downgrade and tamper negative tests (item 4), device runs (items 5–6), and the admission-deadline and post-join trickle tests from §28.

## 30. 2026-11-08 — Android host TLS identity from a Keystore key: design and spike

**Problem.** The Android host's control-port key is non-exportable in Android Keystore, so the in-process `host_tls_identity_from_pkcs8` path (used by the desktop) cannot build its TLS identity. Until it can, the Android host runs with `secure_control: None` and refuses TLS listeners with a visible reason.

**Spike (built and tested).** `transport/secure_control_external_key.rs` builds a `HostTlsIdentity` from any `ListenerProofSigner`, so the key is reached only through `spki_der()` and `sign_der()`:
- The certificate is issued with the same signer through rcgen's `SigningKey` trait (`PublicKeyData` supplies the P-256 point, `sign` delegates to `sign_der`). The SPKI, certificate and handshake key therefore come from one source.
- The rustls `SigningKey`/`Signer` delegates each CertificateVerify to the signer. rustls passes the unhashed message; the signer hashes it with SHA-256, which matches `SHA256withECDSA` on Keystore.
- Tests in `secure_control_external_key_tests.rs` (real loopback TLS 1.3): a pinned handshake completes and echoes; a signer whose signature does not match its advertised key is refused; a client pinned to another key refuses the host. Full Rust gate exit 0, 23 suites, 812 passed.

**Proposed Android design.**
1. *Keystore key.* A new alias `silent-disco-host-tls-v1`, created by a `KeystoreEcKey` with the same parameters as the host invitation key (P-256, SHA-256, `setUserAuthenticationRequired(false)`, non-exportable).
2. *FFI callback.* Reuse the listener's callback shape (`spkiDer`, `signDer`) as a host-side callback interface. The Kotlin side wraps the alias in a `KeystoreHostTlsSigner`, mirroring `KeystoreListenerProofSigner` and its error mapping.
3. *Host configuration.* The FFI host config gains an optional signer. When it is set, the core builds the identity through `host_tls_identity_from_external_key` at bind time. If the Keystore signer cannot provide the SPKI or the certificate cannot be issued, bind fails visibly; the host never silently runs without TLS.
4. *Pin.* The invitation carries the SPKI of whichever key the TLS identity uses, so listeners pin what the handshake proves.
5. *Continuity.* The existing `HostKeyContinuity` check applies to the TLS alias the same way it does to the invitation key. A replaced key changes the pinned SPKI, so old invitations stop working, and the UI reports it.

**Decision needed before implementation: same key or a separate TLS key.**
- *Option A: reuse the invitation key* (`silent-disco-host-identity-v1`). Nothing changes on the wire, and the existing invitation pin already covers TLS. The cost is one key doing two jobs: invitation signatures and TLS CertificateVerify signatures. The two message formats differ, so this is not an exploitable confusion I can show, but it weakens domain separation and standard practice is to avoid it.
- *Option B: separate TLS key (recommended).* The TLS SPKI is carried in the invitation as a new field, signed by the invitation key, so the invitation signature authenticates the TLS pin. This changes the invitation format on Android, desktop and the core. It also means the desktop change pushed in `28479017` (TLS identity derived from the invitation key) would switch to a separate desktop TLS key too.

**Not done.** Kotlin and FFI wiring, the Keystore signer, the instrumented test on the A54 (a TLS handshake whose CertificateVerify is produced by the Keystore key, and a mutation that uses the wrong alias), and the decision above.

## 31. 2026-11-08 — Option B: invitations carry a signed TLS key; listeners pin it

**Decision (owner).** Option B: the host's TLS control-channel key is separate from its invitation-signing key, and its public half is signed into each invitation. A listener pins the TLS handshake to that key, so the invitation signature authenticates the pin.

**Stage 1 — invitation format (core, `3122f8db`).** Optional `tk` (base64url SPKI, P-256, bounded) in the signed document. It is written only when present, so invitations without it keep their signed bytes. Mutation check: dropping `tk` from the canonical form fails both tamper tests. Compatibility: pre-update builds use `deny_unknown_fields`, so they reject invitations that carry `tk`. Listeners and hosts ship from this repo together, but an installed old listener cannot join a new host by QR.

**Stage 2 — desktop host (`de15c258`).** A second P-256 key per profile in the credential store (`profile-host-tls-v1:`), created on first open. The TLS identity and the invitation's `tk` both come from it. Tests: the invitation carries the TLS key and not the signing key; the listener proof verifies against the TLS key; the TLS key differs from the signing key.

**Stage 3 — Android host (`0dc1ef2b`, `f7f8ab41`).** Keystore alias `silent-disco-host-tls-v1`. FFI `bindSecureWithHostClock` and `FfiProofHostListener.bindWithPlatformKey` take a platform signer and fail closed. `HostTransportController` binds secure by default. Invitations carry the Keystore key as `tk`. FFI tests: binds with a key, refuses when the key cannot supply its SPKI, loopback proof verified by the host.

**Stage 4 — listeners pin (this commit).**
- FFI `connect_secure(raw, now, device, addr, host_tls_spki, proof_signer)`: TLS pinned to the key. Plaintext `connect` is unchanged. A pin never falls back to plaintext.
- Core/FFI tests: a pinned listener joins over TLS and is approved on the same channel; a listener pinned to another key never connects.
- Kotlin: `P2ValidatedInvitation.hostTlsPublicKeyDer`. The view model records the key by session id when the user accepts an invitation (`rememberInvitedHostTlsKey`, both the direct QR and the discovery QR paths). Both connect paths look it up by session id. A session with no recorded key stays plaintext. A session id is used only for its own invitation's pin, so a different host on the LAN with the same session id fails the handshake.

**Device (A54, Keystore on both sides, same process).**
- `HostTlsKeystoreInstrumentedTest`: a pinned listener completes TLS and its proof against the Keystore host key; a wrong pin is refused. 2 pass.
- `TlsJoinKeystoreInstrumentedTest`: a complete join over TLS (join seen by the host, approval back over the same channel); a wrong pin never connects. 2 pass.
- Run with `adb install -r` and `am instrument`. `connectedDebugAndroidTest` was not used because it uninstalls the app and erases its data.

**Gates.** Rust `check-rust.sh` exit 0, 23 suites, 821 passed. Android `testDebugUnitTest` exit 0. Desktop `cargo check --all-targets` exit 0; desktop clippy `-D warnings` exit 0 (stage 2); desktop tests 402 passed (stage 2). Line-count gate exit 0 (876 files). `npm run check` not run.

**Not done.**
- A live desktop host ↔ A54 run through the invitation QR. Its TLS path is the same core code exercised by the device test, but the cross-device run is still outstanding.
- Discovery-path joins to an Android host carry the invitation pin only when the session id matches; that path was verified by code reading, not on a device.
- Durable trust binding (item 5). Downgrade and tamper negative tests across the app (item 4). The admission-deadline test for a silent TLS peer, and the bound on post-join partial frames (§28).
