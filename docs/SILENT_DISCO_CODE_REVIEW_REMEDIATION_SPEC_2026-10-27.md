# Silent Disco — Code Review Remediation Specification

**Date:** 2026-10-27 (Pacific)
**Status:** Implementation specification; security and correctness acceptance outstanding
**Baseline reviewed master:** 5c187acafc9df0ae07816b814ddae38e1c4a2a54
**Owner repository:** ekkus93/silent_disco
**Companion checklist:** [Code Review Remediation TODO](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md)
**Existing gapless checklist:** [Gapless Queue and Workflow Redesign TODO](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md)
**Primary sources:** 2026-10-27 cross-platform static code review; AGENTS.md and the existing Rust/Android/desktop architecture.

## 1. Objective and non-goals

Remediate all six reviewed findings (F1–F6), resolve the review's maintainability concerns where practicable, and qualify the affected production behavior with deterministic automated tests on the exact source revision. Preserve the shared Rust-authoritative actor/queue architecture, bounded audio packetizer, sample-indexed gapless timing, native callback continuity, and existing Android/desktop workflows.

Do not replace the audio engine, duplicate business logic in Kotlin or TypeScript, introduce custom cryptographic primitives, allow an insecure fallback to be silently selected, or interpret green build CI as proof of security or physical audio performance. Do not create a branch/PR for every subtask; use exact-head compare-and-swap commits on master as directed by the owner. Keep changes in focused, reviewable commits rather than bundling untested platform rewrites.

This spec is independently actionable for software implementation. If a requirement would change a previously approved product decision, document the choice in an ADR and implement the safest compatible option where possible. Escalate only a genuine owner decision, not routine formatting, tests, or CI failures.

## 2. Baseline and severity

| ID | Severity | Finding | Acceptance disposition |
| --- | --- | --- | --- |
| F1 | High, confirmed logic defect | Genuine Android joins are projected UNKNOWN and desktop joins SessionOnly, preventing the Trusted auto-admission branch in the approved-devices mode. | Host-side verified durable identity and policy enforcement on both platforms. |
| F2 | High on untrusted networks, confirmed missing security controls | Raw TCP/UDP carries plaintext invite/control/audio and relies on non-cryptographic identity/endpoint checks. | Authenticated encrypted transport, replay defenses, explicit protocol migration and threat model. |
| F3 | Medium, confirmed drop-on-full behavior | Shared bounded Rust event queue drops join/control events without ensuring application delivery or an explicit negative response. | No silent loss of authoritative admission/lifecycle events under saturation. |
| F4 | Medium, potential race not reproduced | Android send/close/rebind can access a mutable native handle without one lifetime/operation serialization boundary. | Deterministic no-cross-generation delivery and safe teardown under concurrency. |
| F5 | Medium, confirmed resource risk | Android HostTransport, ListenerTransport and HostCore notification bridges use unbounded channels. | Bounded, priority-aware buffers with observable overflow and safe critical-event delivery. |
| F6 | Low/medium, configuration risk | Android backup enabled without a documented policy for key-bound trust/profile data. | Explicit backup/restore rules and restore tests. |

Related review debt: Android coordinator cohesion, oversized historical TODO/evidence, incomplete Android visual goldens and real desktop webview pixel captures, broader accessibility/lifecycle qualification, and missing independent dependency/security fuzzing audits. These are included as explicit follow-up gates and should not displace F1–F5.

## 3. Security scope and decisions (F1 + F2)

### 3.1 Threat model

Assume venue/shared Wi-Fi and potentially hostile peers on the same LAN. Protect confidentiality and integrity of invite codes, session/control metadata, listener identity and PCM audio against passive observers, spoofing, replay, and active on-path tampering. Do not claim protection against compromised endpoints, denial of service by a network owner, compromised operating systems, or physical output interception.

Until the secured transport is qualified, **do not describe legacy v3 TCP/UDP as safe on untrusted LANs**. Expose a clear, user-visible limitation and do not make a trusted-only admission claim on the basis of a device ID alone.

### 3.2 Device identity and trust

The trusted-device principal is a stable public-key fingerprint and verified proof of possession, not the self-declared device name/ID, local IP, QR display name, or persisted trust-state enum sent by a peer. Android's private device key must remain protected by Android Keystore; desktop uses an OS-protected key/credential store. No private signing/decryption key, session ticket, or bearer secret may be recorded in logs, backups, runtime snapshots, screenshots or generic reports.

Specify first enrollment and trust persistence:
- Host verifies a connecting listener's key possession with a protocol-bound challenge or authenticated client handshake.
- One-time manual approval admits only the current authenticated session; “Always allow” persists a host-side mapping from verified key identity to trust record.
- In **Approved devices only**, only an already trusted verified key is admitted automatically. Unknown, revoked or unverified identities are explicitly rejected and surfaced, not silently treated as trusted and not implicitly enrolled through the join request. If enrollment is offered, it is a separate host-confirmed flow.
- Manual approval and invite-code modes retain their documented workflow but still require a secured host/listener channel and authenticated identity binding; invite codes must not confer permanent trust.
- Revocation is effective for new admission and reconnects, and may terminate live authenticated sessions according to the recorded policy; rejected/revoked identities cannot reuse a stale cached transport route.
- Restart/rebind does not trust a device merely because its previously claimed string ID reappears. Key loss/rotation has an explicit re-enrollment path with a user-visible warning.
- Persistent records are versioned, migrated safely, scoped to the correct host identity, and never promoted from unverified legacy claims without explicit verified enrollment.

Relevant source starting points: rust/silent-disco-core/src/runtime/host_admission.rs; rust/silent-disco-ffi/src/host_control/handle.rs; app/src/main/java/com/ekkus/silentdisco/app/MainViewModelRustHost.kt; desktop/src-tauri/src/platform/host_join_projection.rs; Android/desktop identity stores and host access UI.

### 3.3 Authenticated transport

Record a transport-security ADR before implementation. Default recommendation: a mature TLS 1.3 implementation for the control connection with verified host identity, authenticated client possession for trusted enrollment, and authenticated encryption for audio using a vetted AEAD protocol/key schedule derived from an authenticated session (or use an established secure transport such as QUIC if the architecture analysis shows it safer to integrate). Do not implement novel cryptography or nonce construction without a reviewed established construction.

Required properties:
- Explicit protocol version/capability negotiation; existing v3 plaintext peers fail visibly and cannot be silently downgraded from a secure session. Define a bounded, opt-in, clearly labeled trusted-lab-only legacy compatibility path only if required; never enable it by default on shared/untrusted networks.
- Host authentication for both discovery/QR and direct manual entry; binding between presented host identity and established encrypted channel, with a usable mismatch/first-use confirmation UX.
- Confidentiality, authenticity and integrity for control and PCM; distinct directional keys, separate traffic purposes, unique nonce per key, per-peer/session generation scoping, key rotation on reconnect/new generation, sequence/replay-window enforcement, and secure key cleanup on teardown.
- An unauthorized sender cannot inject StreamStart, Pause, Stop, TrackBoundary, join approval, clock sync or audio; rejected malformed/old-key/duplicate/out-of-window frames do not mutate actor state or advance playback.
- Preserve packet sizes, jitter budgets, gapless sample indices and callback deadlines; perform crypto outside any real-time callback and measure transport overhead. No secrets in CI artifacts or diagnostic tap evidence.
- Existing signed-QR identity features may be reused only after proving their binding to the transport handshake. CRC32 is corruption detection, not a MAC.

Evidence must include threat-model/ADR, key-lifecycle tests, simulated passive/active attacker tests, mixed-version tests and an end-to-end encrypted real-socket PCM regression on desktop and Android emulator where supported.

## 4. Reliable event delivery and bounded resources (F3 + F5)

Keep audio packets and diagnostic telemetry best-effort where the transport specification permits, but make JoinRequestReceived, authorization decisions, denial, terminal disconnect/error, Stop and other authoritative control state reliable or explicitly failed with an acknowledged retryable result.

- Classify event types by priority and persistence. Reserve bounded capacity for critical events; use backpressure or a separate reliable control path that does not block the real-time decoder/audio callback.
- A saturated queue cannot silently drop an accepted join. Either deliver exactly once at the application boundary (with deduplication keyed by session/peer/request generation) or send a defined refusal/busy response and allow bounded retries with a deadline.
- Limit incoming per-peer work, channel capacities, pending approvals, and queued message bytes; document limits and overflow behavior. Emit counters and structured non-sensitive diagnostics.
- Android bridge channels may use bounded queues, state conflation and explicit critical-event handling, but must not conflate distinct approval requests, silently discard terminal Stop/close events, or suspend native callback threads.
- On session close/rebind, remove or mark stale queued events; old-generation events may never authorize or mutate a new host.
- Pressure tests must saturate real transport ingestion and slow Android consumers, verifying bounded memory, no indefinite approval spinner, no ghost listeners and no lost terminal event. Use fake clocks and reproducible tests; retain one real-socket soak regression.

Relevant starting points: rust/silent-disco-core/src/transport/socket/shared.rs, host_workers.rs, transport/types.rs; Android HostTransportController.kt, ListenerTransportController.kt, HostCoreController.kt.

## 5. Session-generation-safe native lifecycles (F4)

Serialize bind, send, authorize, poll, shutdown and close for each Android native transport handle. Model a monotonically changing session generation or opaque handle token; every asynchronous operation is bound to the handle/session it started with and cannot transparently execute on a newer handle. Avoid holding blocking locks on the main thread or in real-time audio callbacks.

Required outcomes:
- A send overlapping close either finishes safely on its original still-live handle or returns a typed stale/cancelled result; no UAF/native crash or accidental reroute to a replacement session.
- Close waits for or safely cancels in-flight ownership before releasing the FFI handle. Repeated close, bind failures, process recreation and Stop/reconnect remain idempotent.
- Host approval and subsequent authorize are checked against the same verified session/peer generation. No approval can be applied to a newly rebound host due to suspend points.
- Listener controller has equivalent protection for join/sync/audio operations; old poll events are filtered before business-state projection and the unbounded-queue remediation still holds.
- Stress tests exercise blocked fake native calls and real loopback sockets using multiple coroutine dispatchers, repeated 100+ rebind/close cycles, and permission/error recovery.

## 6. Backup, storage and restore (F6)

Define what is eligible for Android Auto Backup and device-to-device transfer. Explicitly exclude private/key-bound identity material, approval/trust records that cannot be validated against the current Keystore identity, active session credentials, diagnostic captures, staged private source paths and transient caches. Provide Android 12+ dataExtractionRules and compatible legacy fullBackupContent rules, or disable backup altogether if a product-wide backup guarantee is not required. Verify installed-manifest merging and provider/target-specific policy.

After restore to a device lacking the old nonexportable key, the app must discard/revalidate invalid trust mappings, generate a new identity when required, show a clear re-enrollment explanation and never present stale trusted status. Test upgrade, backup+restore, corrupt record, revocation and key rotation in JVM/instrumentation; never assert a physical restore succeeded without evidence.

## 7. Maintainability and documentation

- Refactor oversized Android MainViewModel/MainViewModelRustHost/SilentDiscoApp responsibilities only where doing so makes state ownership and lifecycle tests clearer. Keep Rust authoritative; isolate session generation, admission/trust mapping, transport event projection and UI navigation as independently testable adapters. Avoid speculative wholesale rewrites.
- Preserve existing docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md as a historical/canonical acceptance record. Add a concise live acceptance index and an append-only evidence ledger (or equivalent linked records) so stale historical CI notes are not mistaken for current status. Never delete historical qualification evidence merely to shorten the file.
- Add automated security-oriented tests: protocol input fuzz/negative fixtures, serialization and replay-window property tests, dependency/SBOM audit with an explicit severity triage policy, and CI secret-scanning/credential hygiene where feasible. Audit tooling failures must not silently pass.
- Complete Android screenshot/golden capture with reproducible emulator density/font/theme settings and reviewable diff artifacts. Complete real desktop webview pixel screenshot coverage where feasible rather than treating DOM snapshots as pixel captures. Expand automated keyboard/focus, screen-reader semantics, system-inset, landscape, large-text and reduced-motion tests. Visual changes require product review before golden acceptance.

## 8. Acceptance matrix and release gates

| Gate | Required proof |
| --- | --- |
| Identity and approved-only mode | Same-key enrollment, new/restarted host auto-admit, unknown reject, key theft/ID spoof reject, revoke, rotation, all on Android and desktop. |
| Secure transport | Host+listener authentication, encrypted control/audio, UDP integrity/replay windows, explicit protocol downgrade rejection, end-to-end exact PCM. |
| Event reliability | Deterministic saturation of join/control queues, acknowledgement or visible refusal, bounded memory with slow consumers. |
| Lifecycle safety | Concurrent bind/send/close/rebind host+listener stress, stale generation rejection, native output not reopened at queued-track boundaries. |
| Backup/restore | Explicit extraction manifest, invalid-key restore recovery, no backup of nonexportable/key-bound secret state. |
| Product and maintainability | Updated docs, scenario matrix, Android/desktop visual/accessibility evidence and traceable CI links. |
| Exact-head software CI | Pinned Rust format, strict Clippy, workspace/FFI tests, Android build/JVM/lint and API29 instrumentation, desktop frontend/Rust/bundle, source policy and any applicable security gates on the **same exact master SHA**. |

For each gate, document tests, expected/actual outcomes, paths and one exact SHA; do not close an item from older, partial, skipped or cancelled jobs. Security-sensitive changes require negative/adversarial tests. Do not confuse API29/API36 emulator or native callback evidence with physical speaker/DAC/Bluetooth continuity.

## 9. Deferred physical acceptance and remaining gapless work

The original gapless checklist still owns unresolved playback permutations, late join/reconnect, UI, visual/accessibility and final acceptance tasks. After this remediation checklist closes all independently actionable software items, resume that checklist without losing the original evidence. Coordinate shared tests instead of duplicating them.

**Explicitly deferred by the owner:** real Android-to-Android hardware, packaged desktop-to-Android audio devices, objective speaker/headphone/DAC/Bluetooth capture, physical output-reopen measurement and hands-on accessibility. Keep all related checkboxes unchecked and do not request hardware or claim release-level physical audio proof merely because emulator/CI passes.
