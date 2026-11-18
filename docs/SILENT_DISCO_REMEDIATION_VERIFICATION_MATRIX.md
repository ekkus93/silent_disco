# Silent Disco — Remediation Verification Matrix

**Established:** 2026-11-03. **Observed exact master baseline:** `dbbfa34f5461e6d17665f03088f510cdaa2172fb`.  
**Scope:** The planning/test-matrix deliverable in remediation TODO §0. This is a coverage map, **not** a declaration that F1–F6 have passed.

## Evidence interpretation

A successful workflow run proves only that its configured jobs completed; it does **not** prove every scenario in the table was executed. Scenario-level acceptance requires a named test or artifact, positive and adversarial assertions, and the same exact head SHA. The baseline below is an observed hosted CI result, not a substitute for test-case mapping. Never infer physical-device results from emulators or loopback.

| Verification layer | Relevant entry points / suite | Required scenarios and assertions | Gate / status at baseline |
| --- | --- | --- | --- |
| Rust core and protocol | `bash scripts/check-rust.sh`; `rust/silent-disco-core/` | Admission policy Manual/Invite/Approved-only; authorized lifecycle ordering; generation fencing; malformed frames; replay, identity and nonce negatives once secure protocol exists | F1–F3; **not accepted** |
| UniFFI/native boundary | `rust/silent-disco-ffi/`; Android host/listener native controllers | FFI generation ownership; stale authorize/send/poll; close while in-flight; key-bound verified identity crossing boundary | F1/F4; **not accepted** |
| Android JVM | `app/src/test/` | Queue saturation with stalled collectors; distinct approvals; terminal event retention; profile switches; restore with missing/corrupt identity keys; navigation/config state | F4–F6; **not accepted** |
| Android API29 emulator | `app/src/androidTest/`; configured hosted CI | Native loopback join/refusal, close/rebind stress, restored identity UX, host/listener reconnect, process/config lifecycle; collect test names and emulator evidence | F1/F4–F6; **not accepted** |
| Desktop React / Tauri | `desktop/src/`, `desktop/src-tauri/`; `npm run check` and Rust checks in AGENTS.md | Verified trust projection, explicit join errors, desktop keyboard/focus and lifecycle, pixel screenshot vs DOM-only evidence | F1/F2/§8; **not accepted** |
| Real loopback TCP/UDP | `rust/silent-disco-core/src/transport/socket/`; Desktop Lab live transport workflow | Multi-peer join/deny/reconnect, Stop/replay, exact PCM/sample indices, track boundaries, no native output reopen, high telemetry pressure and bounded busy refusals | F2/F3/§9; **partial v3 transport evidence only** |
| Hostile peer / security | Secure-transport ADR plus new negative fixtures and authenticated real-socket integration | Forged host/listener ID, invalid key/cert/tag, plaintext downgrade, duplicate/old packet, nonce wrap, retired generation, tampered control, QR mismatch, unauthorized audio | F1/F2; **missing authenticated-v4 acceptance** |
| Resource pressure | Rust bounded event queue, Android bounded bridges; multi-peer loopback and slow collectors | Saturate authoritative events; explicit refusal or delivery; no ghost approval/Stop loss; bounded bytes, counters, finite retry, shutdown under pressure | F3/F5; **not accepted end-to-end** |
| Android visual and accessibility | Emulator screenshot/golden and Compose tests | Theme/font scale/orientation/density, approval/reconnect/error, insets, reduced motion, keyboard and focus; publish diff artifacts | §8; **not accepted** |
| Desktop pixel visual | GitHub-hosted desktop webview capture | Real pixels and reviewed baseline update policy; separate from DOM snapshots | §8; **not accepted** |
| Supply chain | Pinned audit/SBOM and CI scanners | Reproducible dependency manifest, severity policy, failure visibility, no secret leakage | §7; **not accepted** |
| Physical hardware (owner deferred) | Physical Android and desktop devices; objective audio measurement | DAC/Bluetooth/headphone output, real-device end-to-end continuity and hands-on accessibility | §10; **DEFERRED, never infer from software CI** |

## Observed hosted CI at baseline

All five selected workflows were **completed / success** for exact `dbbfa34f5461e6d17665f03088f510cdaa2172fb`:

| Workflow | Run | What can be claimed |
| --- | --- | --- |
| Rust/Android CI | [41612379225](https://github.com/ekkus93/silent_disco/actions/runs/41612379225) | Configured CI workflow passed; inspect jobs/artifacts before naming individual tests |
| Desktop CI | [41612379167](https://github.com/ekkus93/silent_disco/actions/runs/41612379167) | Configured desktop workflow passed |
| Desktop Lab live transport | [41612379095](https://github.com/ekkus93/silent_disco/actions/runs/41612379095) | Configured lab workflow passed; does not establish encrypted v4 |
| Source file line limit | [41612379102](https://github.com/ekkus93/silent_disco/actions/runs/41612379102) | Source line policy passed |
| Publish CI Status | [41616262389](https://github.com/ekkus93/silent_disco/actions/runs/41616262389) | Status publication succeeded |

## Acceptance protocol for each future completed item

Record the implementation path, named positive and negative tests, exact SHA, workflow/run ID, and actual assertion results. Mark a requirement complete only when **all** of its acceptance clauses are proven. Refresh the SHA before any CAS commit, and re-run the applicable workflows on the new head. Preserve the original TODO and the append-only evidence ledger. No checked item here implies production secure transport, authenticated trust, or physical playback qualification.

## Final findings matrix — exact code SHA `32c2d4c5f22e17f16359a5fced0d872ca312aba7` (2026-11-18, remediation TODO §9)

Every row below was evaluated on this one code SHA. "Proven" means named automated tests passed on this SHA in hosted CI and locally; it never means physical-device or real-network proof beyond the single A54 runs named. The owner-deferred hardware items (TODO §10) are outside this matrix.

### Hosted CI on the exact SHA

| Workflow | Run | Result |
| --- | --- | --- |
| CI (Rust quality gates; Android build, unit tests, lint; API 29 emulator instrumentation; candidate publication gate) | [44192003435](https://github.com/ekkus93/silent_disco/actions/runs/44192003435) | success: Rust 948 passed / 0 failed with `--test-threads=1`, `PCM_EVIDENCE_PASS`; emulator 203/203 (0 failed, 0 skipped); 26 screenshot scenes identical across two cold processes, goldens 0 approved / 26 unreviewed |
| Desktop CI (manual dispatch on this SHA; frontend, Rust, Linux bundle smoke) | [44243570601](https://github.com/ekkus93/silent_disco/actions/runs/44243570601) | success: desktop Rust 424 lib tests; packaged WebKit pixels captured for startup, Settings and storage at 1000x700 and 760x560 |
| Desktop Lab live transport (manual dispatch) | [44244965907](https://github.com/ekkus93/silent_disco/actions/runs/44244965907) | success (v3 Lab-only proofs; not secure-transport evidence) |
| Supply chain | [44192003564](https://github.com/ekkus93/silent_disco/actions/runs/44192003564) | success: cargo-deny both workspaces, npm audit triage, osv-scanner Gradle locks, gitleaks full history, SBOMs |
| Source file line limit (and hosted-runner policy, script tests) | [44192003470](https://github.com/ekkus93/silent_disco/actions/runs/44192003470) | success |

### Local gates on the same SHA (2026-11-18)

`scripts/check-rust.sh` 948 passed / 0 failed; desktop `npm run check` (Node 22.16.0) 202 passed and production build; desktop `cargo clippy -D warnings` clean and `cargo test --locked` 429 passed (424 lib, 4 multiprocess, 1 storage); full Android gate (`assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest test lintDebug`) passed; `scripts/check-supply-chain.sh` passed; script tests (npm triage, goldens, hosted-runner policy, PCM capture, PCM evidence) passed. **Physical A54 (Android 16)**, full instrumented suite: 203 run, 198 passed, 1 skipped, 4 failed; the 4 are the known A54 large-text layout failures recorded at the 2026-11-13 baseline (3 in `AdaptiveLayoutRemainingCoverageTest`, `SessionJoinPinnedStatusTest.rejectedInviteCodeAnnouncesPinnedErrorAndRestoresEditableRecovery`), which pass on the API 29 emulator.

### Findings

| Finding | Fixed production paths | Proving tests (examples) | Status on this SHA | Residual risk / not yet tested |
| --- | --- | --- | --- | --- |
| **F1** Approved-devices-only admission | Core `runtime/host_admission.rs` (approved-only admits only `Trusted`); `transport/secure_trust_store.rs` (key-bound records scoped to the host key); FFI `host_control/handle_trust.rs` (`LoadedHostTrust`, generation refresh), `handle.rs` `connected_listener_trust`; desktop `platform/verified_trust.rs`, `host_transport_events.rs`; Android `TrustedDevicesViewModel` | `secure_host_admission.rs::approved_only_uses_verified_host_records_not_claimed_trust`, `forged_claimed_device_id_cannot_collide_with_another_verified_listener`; `secure_trust_store_tests::claimed_ids_are_never_remembered_or_trusted`; FFI `a_device_id_outside_the_key_bound_namespace_is_never_trusted`; desktop `host_transport_verified_trust_tests`; A54 `HostVerifiedTrustDeviceTest` (2/2, real Keystore keys through the real core) | **Proven in software**; TODO §2 trust-records item stays open for the owner's two-device run | A missing host-TLS or listener-proof Keystore key is silently regenerated (`KeystoreEcKey.keyPair()`): the host then loses its remembered listeners (fails closed, no notice) and a listener gets a new identity without notice. The FFI `submit_join_request` trusts a key-bound ID passed by Kotlin; in v4-only builds Kotlin only forwards IDs from verified QUIC events, but the FFI does not check the channel itself |
| **F2** Transport security | Core QUIC v4 (`quic_datagram.rs`, `quic_admission.rs`, `quic_host_worker.rs`, `quic_listener_worker.rs`, `quic_framing.rs`); FFI `legacy_v3.rs`/`protocol_policy.rs` (v3 only with the `legacy-v3` feature); desktop v3 only in Lab Mode | `protocol_policy_tests::a_production_build_refuses_every_v3_host`; `quic_transport_factory_tests::secure_v4_refuses_to_run_without_an_identity_instead_of_falling_back`; QUIC admission, worker, stalled-peer, reconnect and on-wire tests; `quic_framing_fuzz_tests` (hostile chunking, lengths, truncation, cancellation); `secure_replay_fuzz_tests`; desktop `a_release_profile_hosts_v4_and_cannot_switch_to_v3` | **Proven in software** for one listener; open TODO §1 items: two-listener v4 test and a phone-to-phone Android-host run (need a second device), and the exact-head scenario verification item | ADR scope: passive capture, forged peers, MITM/downgrade, tampering/replay, stale generations, unauthorized admission. **Out of scope:** compromised endpoint/OS, malicious local host, network-owner denial of service, physical output interception, traffic analysis. `TransportProtocol::default()` is still `LegacyV3` in core; production callers select v4 explicitly |
| **F3** Authoritative events never silently lost | QUIC worker lossless lanes; core `forward_join_or_refuse`, `send_authoritative_event_until`; actor refusal-to-failure; host busy refusal with listener retry (`runtime/busy_join_retry.rs`, Android `PendingBusyRetry`) | `quic_worker_tests::a_full_event_queue_never_stops_the_host_handling_commands`; `capacity_refusal_tests::full_host_visibly_refuses_real_join_without_starving_playback_and_recovers`; `BoundedTransportEventQueueTest`; `BoundedCoreNotificationQueueTest`; `PendingBusyRetryTest` (2,000-round cancel race); `ListenerSessionControllerTest` busy-retry cases | **Proven in software** | Peer-limit connection refusals are best-effort by design (counted; the refused peer sees the refusal). A real host refusing a phone as busy is untested on a device |
| **F4** Native-handle lifetime and generations | `HostNativeHandleGate` / `ListenerNativeHandleGate` (identity-checked calls, poll leases, deferred close); `HostSessionController.projectDequeuedEvent` | `HostNativeHandleGateTest` (rebind vs in-flight send, 128-rebind poll leases), `ListenerNativeHandleGateTest`, `ManualListenerNativeLifetimeTest`, `MainViewModelHostRebindRaceTest`, `HostSessionControllerTest.anEventDequeuedBeforeARebindIsNeverProjectedIntoTheReplacementHost` | **Proven in software, one known gap** | `ManualListenerTransportControllerPlayback` `endStream` detaches playback through `withCurrentManualHandle` (current handle, not the stream's own). The manual event loop's failure path calls it outside the gate, so a rebind racing it could detach the new session's playback. Not a use-after-free; narrow window; unfixed at this SHA |
| **F5** Bounded Android bridge queues | `BoundedTransportEventQueue` / `BoundedCoreNotificationQueue` (`Channel(64)`, best effort capped at a quarter); no unbounded channels in app, FFI or core production code | `BridgeQueueRetentionTest` (`stalledCollectorFloodRetainsAtMostCapacityTimesPayloadBytes`, `rapidRebindsRetainOnlyTheCurrentSessionsBytes`, `coreDiagnosticFloodRetainsAtMostTheDiagnosticShareInBytes`) | **Proven in software** | `ListenerSessionController.invitedHostTlsKeys` grows by one entry per scanned invitation (user-driven, not a bridge path) |
| **F6** Backup/restore and identity data | `allowBackup=false` plus cloud-backup and device-transfer exclusions; `HostKeyContinuity` / `HostIdentityManager.checkContinuity` with a visible host-key-replaced message | JVM `HostKeyContinuityTest`; instrumented `HostIdentityContinuityInstrumentedTest`, `HostKeyRestoreInstrumentedTest`, `BackupPolicyInstrumentedTest`, `BackupExclusionInstrumentedTest` (simulations; emulator and A54) | **Proven in software for the host invitation key** | Only `silent-disco-host-identity-v1` has a continuity check (see F1 residual for the other two aliases). Handoff §15's key inventory is stale (it predates key-bound trust records). No real cloud-restore or device-transfer proof |

### §9 invariant review (independent read-only audit of this SHA, then confirmed in code)

| Invariant | Verdict |
| --- | --- |
| No plaintext fallback in default production paths | Holds |
| No spoof-based trust | Holds, with the F1 caveat on the FFI channel check |
| No silent critical-event loss | Holds |
| No unbounded bridge accumulation | Holds |
| No cross-generation FFI access | **Gap**: the F4 manual-listener `detachPlayback` path |
| Documented restore recovery | **Partly**: host invitation key only; two Keystore aliases regenerate silently and handoff §15 is stale |

## Re-qualification at exact code SHA `53587f0b1f06cbc9b7546139842187c17e6baf68` (2026-11-18) — supersedes the SHA above

The two §9 invariant gaps found at `32c2d4c5` are fixed, and every gate was re-run on the new SHA.

| Gap at `32c2d4c5` | Fix | Proof |
| --- | --- | --- |
| F4: manual-listener teardown detached playback through the *current* connection, so an old session's teardown racing a rebind could detach the new session's playback | `fcf78f7f`: the owning connection is recorded with the playback under the same gate as `attachPlayback`; teardown detaches through it with `withExpected` | `ManualPlaybackDetachGenerationTest` (3); the old-session case failed before the fix; restoring the current-handle detach fails two tests |
| F6/F1: the host TLS and listener proof Keystore keys were silently regenerated, so a host lost its remembered listeners and a listener got a new identity with no notice; handoff §15 inventory stale | `53587f0b`: `KeyContinuityMonitor` checks both keys against fingerprint records at startup without creating keys and reports a replaced or missing key once; handoff §15 updated | `KeyContinuityMonitorTest` (6), `KeyReplacementMessagesTest` (3), A54 `KeyContinuityMonitorInstrumentedTest` (3, real Keystore, test-only aliases); making a missing key silent again fails a test |

| Workflow | Run | Result |
| --- | --- | --- |
| CI | [44250209583](https://github.com/ekkus93/silent_disco/actions/runs/44250209583) | success: Rust 948 passed / 0 failed, `PCM_EVIDENCE_PASS`; Android build, unit tests, lint; API 29 emulator 206/206; 26 screenshot scenes identical across cold processes (0 approved goldens); candidate gate |
| Desktop CI (dispatched on the SHA) | [44250219945](https://github.com/ekkus93/silent_disco/actions/runs/44250219945) | success: desktop Rust 424 lib tests; packaged WebKit pixel captures |
| Desktop Lab live transport (dispatched) | [44250223140](https://github.com/ekkus93/silent_disco/actions/runs/44250223140) | success |
| Supply chain | [44250209515](https://github.com/ekkus93/silent_disco/actions/runs/44250209515) | success |
| Source file line limit | [44250209558](https://github.com/ekkus93/silent_disco/actions/runs/44250209558) | success |

Local on the same SHA: `check-rust.sh` 948 passed; desktop `npm run check` 202 passed, clippy clean, `cargo test --locked` passed; full Android gate passed; supply-chain gate passed; script tests passed. A54 full instrumented suite: 206 run, 201 passed, 1 skipped, the same 4 known large-text layout failures.

**Invariant review after the fixes:** no plaintext fallback, no spoof-based trust, no silent critical-event loss, no unbounded bridge accumulation, no cross-generation FFI access found, and restore recovery documented and checked for all three Keystore keys. Every other row of the findings table above stands, including its residual risks and its limits (no real cloud restore, no multi-listener or phone-to-phone hardware runs).
