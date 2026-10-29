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
