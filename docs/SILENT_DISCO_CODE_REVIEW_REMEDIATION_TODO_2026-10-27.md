# Silent Disco — Code Review Remediation TODO

**Date:** 2026-10-27 (Pacific)
**Status:** Open — prioritize security and correctness; software-only execution
**Specification:** [Code Review Remediation Spec](SILENT_DISCO_CODE_REVIEW_REMEDIATION_SPEC_2026-10-27.md)
**Current progress handoff:** [Remediation handoff (2026-10-28)](SILENT_DISCO_CODE_REVIEW_REMEDIATION_HANDOFF_2026-10-28.md)
**Current acceptance navigation:** [Concise acceptance index](SILENT_DISCO_ACCEPTANCE_INDEX.md) · [Append-only evidence ledger](SILENT_DISCO_ACCEPTANCE_EVIDENCE_LEDGER.md)
**Existing acceptance:** [Gapless Queue and Workflow Redesign TODO](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md)
**Reviewed baseline:** 5c187acafc9df0ae07816b814ddae38e1c4a2a54
**Branch policy:** direct master, Ralph Bridge exclusively, exact-head CAS commits, no per-task branches/PRs

This is the ACTIVE remediation checklist for the six findings of the 2026-10-27 cross-platform review and its related code-quality/acceptance concerns. Read the linked spec and AGENTS.md at current master before editing. Check a box only when production code, negative and positive tests, and exact-head evidence support it. If a finding is disproved by reproducible evidence, document the proof and update the spec rather than adding speculative code. Mark deferred device tasks only after separately authorized physical evidence.

## 0. Baseline, plan and safe work loop (P0)

- [ ] Refresh the exact current master SHA, full spec, this entire TODO, original gapless TODO, relevant production paths and exact-head CI. Reconcile simultaneous master commits before each CAS write.
- [ ] Reproduce F1 on Android and desktop host production join/projection paths with test fixtures; record current UNKNOWN/SessionOnly state and actual approved-only outcome. **Partial evidence (local gates only, no exact-head CI yet; box stays open):** desktop `approved_devices_only_claimed_identity_is_projected_unverified_and_refused_by_policy` drives a real loopback join through `DesktopHostTransportRuntime` with an approved-only host and asserts the actor receives `TrustState::SessionOnly` and `classify_join_request` returns `AutoReject(DeviceNotApproved)`; Android `HostJoinTrustProjectionTest` asserts a claimed device ID reaches the actor as `FfiTrustState.UNKNOWN`. Both fail under a mutation that projects Trusted. Gap: the Android test uses `FakeHostCoreController`, so the approved-only *outcome* is not yet exercised through the real Rust actor via FFI.
- [ ] Map F2–F6 call paths, existing identity/key storage, protocol versioning, transport frame types, queue capacities, Android native-handle ownership and backup manifest. Record evidence in a short remediation handoff.
- [x] Create a test/CI matrix with Rust, UniFFI, Android JVM/API29 instrumented, desktop/Tauri, real loopback socket, hostile-peer and pressure scenarios. Do not conflate simulation with real device evidence. **Evidence:** [remediation verification matrix](SILENT_DISCO_REMEDIATION_VERIFICATION_MATRIX.md), baseline `dbbfa34f`, hosted [CI 41612379225](https://github.com/ekkus93/silent_disco/actions/runs/41612379225), [Desktop CI 41612379167](https://github.com/ekkus93/silent_disco/actions/runs/41612379167), [Desktop Lab 41612379095](https://github.com/ekkus93/silent_disco/actions/runs/41612379095). This closes the **matrix deliverable only**, not the individual implementation or security acceptance gates.
- [ ] Preserve current green behavior and regressions; use GitHub-hosted runners, pinned formatting/strict Clippy/Android lint, exact-head CI and failure-log inspection. Do not report success from partial, older or skipped CI.

## 1. F2 — Transport security and threat model (P0; prerequisite for durable trust)

- [x] Write an explicit LAN threat model and transport security ADR: unauthenticated legacy-v3 limitations, host identity/bootstrap UX, listener proof, protocol-v4 or equivalent versioning, crypto library selection and rollback rules.
- [x] Specify authenticated enrollment and stored host/listener public-key identity; design reuse of existing signed QR/key material only if handshake-bound. Define manual-address host fingerprint confirmation. **Design evidence:** [v4 secure transport ADR](SILENT_DISCO_SECURE_TRANSPORT_ADR_2026-10-29.md), committed at `c5a89a01`; this is specification only, **not** proof of implemented crypto, enrollment or F1/F2 acceptance.
- [ ] Implement verified encrypted host↔listener control transport using a mature TLS 1.3 or established secure transport stack; enforce host identity and verified peer identity without trusting self-declared device IDs.
- [ ] Protect UDP PCM with vetted AEAD or authenticated secure datagrams; per-direction/per-purpose/per-session keys, unique nonces, replay windows, key rollover/retirement, clock-sync/control authentication and secure teardown.
- [ ] Add protocol-version negotiation and explicit rejection of plaintext/downgrade and mixed incompatible peers; any legacy/lab exception is opt-in, clearly labeled and never automatic in production.
- [ ] Add negative tests: forged device ID, forged host, bad MAC/tag, duplicated/old packet, nonce/sequence wrap, retired stream generation, untrusted QR, invalid certificate/key, tampered control, downgrade and malformed framing.
- [ ] Add encrypted real TCP/UDP (or selected transport) host/listener integration through scheduler/native callback: verify exact sample indices, PCM, no output reopen, acceptable jitter/buffer/CPU cost and independent peer keys.
- [ ] Verify Android and desktop bootstrap, discovery/manual join, reconnect, stop/replay, signed invite, revocation and safe failure UI on exact-head CI. No real-device claims.

## 2. F1 — Repair approved-devices-only admission (P0; requires verified identity)

- [ ] Make Rust admission policy explicit for Manual, Invite code and Approved devices only; unknown/revoked/unverified identities must not auto-approve in approved-only mode. Decide/document separate enrollment UX; no automatic fallback to pending without truthful UI.
- [ ] Implement durable host-side trusted public-key records (versioned storage, migration, revocation and key rotation) with OS-backed private keys and no silent elevation of legacy claimed device IDs.
- [ ] Validate cryptographic proof of listener identity before projecting Trusted/Untrusted. Update Rust FFI admission inputs and Android MainViewModelRustHost join mapping; never hard-code TRUSTED.
- [ ] Update desktop host_join_projection to resolve verified host-owned trust records instead of unconditional SessionOnly; ensure Android/desktop policy parity.
- [ ] Wire Always allow / Approve once / revoke, first enrollment, host restart, listener reconnect, profile switch and lost-key re-enrollment with accurate user-visible status.
- [ ] Add Rust policy, FFI, Android JVM/API29 and desktop tests covering valid trusted auto-admission, unknown rejection, revoked key rejection, spoofed claimed ID with wrong key, manual mode, invitation mode and approval timeout.
- [ ] Prove join success/failure on real loopback authenticated sockets and no unauthorized access to StreamStart, TrackBoundary, audio or synchronization after rejection.

## 3. F3 — Never silently lose authoritative transport events (P1)

- [x] Classify Rust socket transport event types: authoritative admission/control/terminal events vs lossy audio metrics/telemetry; define bounded pressure/acknowledgement semantics. **Evidence:** `rust/silent-disco-core/src/transport/socket/event_delivery_class.rs`, shared send guard and both TCP reader routes, exact-head `513f75f8`, Rust quality job in [CI 41739460845](https://github.com/ekkus93/silent_disco/actions/runs/41739460845) **passed**, [Desktop Lab 41739460850](https://github.com/ekkus93/silent_disco/actions/runs/41739460850) **passed**. The umbrella Android/desktop workflows were not terminal when recorded; this closes only the Rust event-classification requirement.
- [ ] Replace drop-on-full for accepted JoinRequestReceived and critical control events with bounded reliable delivery or an explicit busy/rejected reply and finite client retry. Do not block audio callback/decoder.
- [ ] Add deduplication and generation-aware request IDs to prevent duplicate approvals after retry, reconnect or peer-route replacement; preserve order for Start/Pause/Stop/boundaries. **Partial evidence (local gates; box stays open):** Rust: one accepted join per TCP generation with exact suppression counts (`duplicate_join_tests`), same-ID pending/replacement isolation (`simultaneous_pending_same_id_connections_cannot_steal_approval`, `same_id_join_arriving_while_authorization_waits_cannot_steal_approval`), FIFO ordering of authoritative controls (`control_sender_pressure_tests`, `expect_ordered_controls`) and boundary/Pause/Stop fencing (`playback_controls_tests`). Desktop: the approval effect rejects a request ID that is no longer the pending generation; new `host_transport_stale_approval_tests::stale_request_approval_after_reconnect_fails_and_current_request_still_admits` approves, reconnects the same device, replays the retired request (failed delivery, nothing reaches the replacement) and then admits the current request; it fails (stale approval delivered) when the guard is disabled. Gap: the Android equivalent (`pendingListenerDatagramPorts.take(requestId, listenerId)` returning no route and a failed delivery report in `MainViewModelRustHost.executeRustTransportEffect`) has no direct JVM test because the function is file-private; cover it with the F4 "reject stale approvals" item. Exact-head CI for this commit is not yet recorded.
- [ ] Expose bounded counters/status for congestion, refusals, retries and event loss, avoiding secrets in logs. **Partial evidence (local gates only, no exact-head CI; box stays open):** core `TransportCounters` already counted `queue_overflows`, `delivery_failures`, `best_effort_event_drops`, `busy_admission_refusals` and `authoritative_event_failures`, but the UniFFI host/listener counter records omitted the last three. They are now exposed through `From<TransportCounters>` impls that the handles use, with per-field mapping tests (`counter_mapping_tests`). A `duplicate_join_suppressions` counter now records ignored join retries on an admitted TCP generation (asserted exactly, 4/8/9, in `duplicate_join_frames_never_reenroll_but_new_tcp_generation_can_join`; fails when the production call is removed) and is exposed through `FfiHostTransportCounters`. `CounterState` moved out of `socket/shared.rs` into `socket/counters.rs` (796 to 648 lines) as a responsibility split. Remaining: any Android/desktop UI or diagnostics surface for these counters, and exact-head CI.
- [x] Deterministically saturate Rust transport events with join requests and critical controls; prove handled-or-visibly-refused, no indefinite spinner, no ghost admission and no hidden Stop. **Evidence (Rust transport boundary only):** handled-or-visibly-refused: `host_worker_admission_pressure_tests::saturated_join_queue_is_refused_without_silent_acceptance` (deterministic bounded channel) and real-socket `telemetry_pressure_tests::real_socket_busy_join_is_refused_then_retry_rejoins_live_pcm` (listener receives an explicit `JoinRejection` "Host busy", then a fresh TCP generation rejoins live PCM). No ghost admission: `terminal_pressure_tests::saturated_host_event_queue_rolls_back_real_socket_authorization`, `failed_control_delivery_tests::failed_pending_hello_cannot_become_a_ghost_approval_after_reconnect`, and `stalled_actor_refuses_tcp_peer_without_ghost_route_or_harming_live_peer` (also proves a live peer still receives StreamStart). No hidden Stop/disconnect: `telemetry_pressure_tests::telemetry_flood_cannot_hide_stop_disconnect_or_reconnected_play`, `multi_peer_control_pressure_tests::multi_peer_telemetry_pressure_preserves_boundary_pause_stop_and_fresh_replay`, `terminal_pressure_tests::real_socket_heartbeat_flood_does_not_hide_authoritative_disconnect` and `stalled_host_actor_expires_critical_control_and_allows_clean_reconnect`. Bounded, never-indefinite waits: `authoritative_delivery_tests` and `control_sender_pressure_tests`. Exact-head Rust quality job on `acc29010` [CI 42050844898](https://github.com/ekkus93/silent_disco/actions/runs/42050844898) **passed**, Desktop CI 42050844891 **passed**; its Android and Lab jobs were not terminal when recorded. This closes only the Rust boundary: the Android bridge and permanent-UI-spinner requirement remains under F5, and saturation here is driven by UDP telemetry, stalled actors and bounded-channel unit tests rather than a flood of concurrent real-socket joins.
- [x] Run at least one real-socket multi-peer pressure test, including disconnect/reconnect and high telemetry volume, with explicit bounds and logs. **Evidence:** `host/telemetry_pressure_tests.rs`, `host/terminal_pressure_tests.rs`, `host/capacity_refusal_tests.rs`; actual loopback 2+ peers, 1,000 telemetry datagrams, bounded queues, busy rejections, Stop, disconnect/reconnect and native PCM replay; exact-head `513f75f8` Rust tests [CI 41739460845](https://github.com/ekkus93/silent_disco/actions/runs/41739460845) Rust-quality job **passed**, [Desktop Lab 41739460850](https://github.com/ekkus93/silent_disco/actions/runs/41739460850) **passed**. This does not close end-to-end F3/F5 or physical acceptance.

## 4. F4 — Serialize Android native-handle lifetime and generations (P1)

- [ ] Map all HostTransportController and ListenerTransportController bind/send/poll/authorize/close/rebind call sites and suspend boundaries; establish clear thread/dispatcher ownership.
- [ ] Implement synchronized in-flight native-handle lifetime and session-generation guards; a delayed send must never execute on a newly rebound handle.
- [ ] Make close/cancel/bind-failure/shutdown idempotent and safe while calls are blocked; ensure native handles are not released while still in use.
- [ ] Couple join approval + authorize to the same verified peer/session generation; reject stale approvals and prevent retired poll events from mutating the next session.
- [ ] Add coroutine race tests with a barrier-blocked fake FFI send, concurrent Stop/close/rebind, repeated profile switches, exception/cancellation propagation and late event arrivals.
- [ ] Add Android API29 or JVM+native loopback stress coverage and check no crash, cross-session authorization, leaked poll jobs, duplicate listener or output re-open on same-stream resume.

## 5. F5 — Bound Android bridge queues and memory (P1)

- [ ] Inventory every Android Channel.UNLIMITED use in HostTransportController, ListenerTransportController, HostCoreController and adjacent bridges; document producers, consumers and event categories.
- [ ] Replace unlimited buffers with explicit bounded/priority-aware delivery and state conflation where safe. Preserve all distinct join requests and required terminal events; prevent blocking producer/audio threads.
- [ ] Define overflow behavior, diagnostic counters, backpressure/busy responses and cleanup on close, process restoration and session rebind.
- [ ] Test slow/stopped collectors, rapid host rebind, repeated listener reconnect, high-frequency audio/status notifications and terminal events; assert bounded retained bytes and a consistent final state.
- [ ] Qualify interaction of F3 Rust saturation with Android bridge limits; verify no silent cross-layer event loss or permanent UI spinner.

## 6. F6 — Android backup/restore and identity data (P2)

- [ ] Inventory prefs/databases/identity stores/staged files/cache and decide which data may migrate; explicitly document Android 12+ and legacy device backup policy.
- [x] Implement dataExtractionRules/fullBackupContent exclusions or disable backup intentionally; exclude Keystore-bound trust/identity, tickets, diagnostics, transient session metadata and sensitive audio staging.
- [ ] Implement safe restore detection for missing/replaced nonexportable keys; invalidate or revalidate dependent trust records and display clear re-enrollment UX without auto-trusting.
- [ ] Add Android manifest-merge and JVM/instrumented restore simulation tests for key mismatch, upgrade, corrupt record, revocation and restore to a new device identity. Do not claim cloud/physical restore proof from a simulation.

## 7. Maintainability, review debt and additional security assurance (P2)

- [ ] Extract tightly coupled Android ViewModel transport lifecycle, admission/trust projection and UI coordination into focused, testable adapters where doing so directly reduces F1/F4/F5 risk; retain Rust actor authority.
- [ ] Add owner/generation invariants and race-focused tests for refactored orchestration. Do not introduce new duplicate playlist/business policy.
- [x] Create a concise current acceptance index and separate append-only evidence ledger for the historical 167 KB gapless TODO; preserve original checkboxes/history and working relative links. **Evidence:** [index](SILENT_DISCO_ACCEPTANCE_INDEX.md), [ledger](SILENT_DISCO_ACCEPTANCE_EVIDENCE_LEDGER.md), [exact-head hosted CI at `693cd90c`](https://github.com/ekkus93/silent_disco/actions/runs/41298699164).
- [ ] Add bounded protocol parser fuzzing/property tests and negative fixtures for authenticated controls/replay, malformed packets, identities, oversized queues and cancellation.
- [ ] Add automated pinned dependency/SBOM and vulnerability auditing with documented severity triage and secrets/credential hygiene; distinguish actual findings from scanner noise.

## 8. Visual, accessibility and lifecycle acceptance follow-up (P2)

- [ ] Complete reproducible Android screenshot/golden workflows for reviewed host/client states (theme, font scale, orientation, density, errors, approval, reconnect); publish failure artifacts and require human approval for baseline updates.
- [ ] Add actual desktop webview pixel screenshot CI where supported by GitHub-hosted runners; retain DOM snapshots as separate non-pixel evidence. Document blockers rather than asserting visual parity.
- [ ] Expand Android Compose/process/config restoration tests for host/listener approval, retry, reconnect, code edit, keyboard/focus, compact/landscape, system bars/insets, large text and reduced motion.
- [ ] Expand desktop focus order/keyboard, reduced motion, contrast and touch/target semantics where relevant; link acceptance tests to actual component states.
- [ ] Reconcile this work with original gapless TODO without marking broad playback/golden/accessibility items complete prematurely.

## 9. Exact-head qualification and closeout (P0 acceptance)

- [ ] Run pinned Rust formatting, strict Clippy, full workspace/FFI tests, security/negative/pressure tests and evidence validation on the same latest master SHA.
- [ ] Run Android build, JVM tests, lint, API29 emulator instrumentation and candidate publication gate on that same SHA; investigate full job logs and fix failures.
- [ ] Run desktop React/build, Tauri Rust tests and Linux bundle smoke on that same SHA; verify hosted runner and source-line policy; collect any applicable screenshot/supply-chain checks.
- [ ] Build a final findings matrix F1–F6 with fixed production paths, threat model limitations, exact tests, SHA, completed CI runs and residual risks; distinguish proven pass, deferred and not yet tested.
- [ ] Verify no plaintext fallback in default production paths, no spoof-based trust, no silent critical-event loss, no unbounded bridge accumulation, no cross-generation FFI access and documented restore recovery.
- [ ] Confirm every non-deferred item in this checklist with evidence, then resume remaining independently actionable software items in the original gapless TODO. Do not close physical acceptance by inference.

## 10. Owner-deferred physical/hands-on acceptance (DO NOT CLOSE)

- [ ] **[DEFERRED: HARDWARE]** Objective independent speaker/headphone/DAC/Bluetooth recordings, latency/drift calibration and no-output-reopen measurements across track boundaries.
- [ ] **[DEFERRED: HARDWARE]** Packaged desktop-host → physical Android-listener and Android-host → physical Android-listener playback/gapless tests.
- [ ] **[DEFERRED: HARDWARE]** Real-device edit/Stop/replay/late join/reconnect/network-loss and physical accessibility/manual screen-reader acceptance.

No physical phone, owner Wi-Fi credentials or private keys are needed for the preceding software tasks. Leave these three boxes unchecked until the owner explicitly authorizes device work and supplies independently inspectable evidence.
