# ADR — Authenticated v4 transport for Silent Disco

**Date:** 2026-10-29  
**Status:** Accepted implementation direction; **not implemented or qualified**  
**Related:** [F1/F2 remediation specification](SILENT_DISCO_CODE_REVIEW_REMEDIATION_SPEC_2026-10-27.md), [remediation checklist](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md)

## Context and security boundary

The current v3 TCP control and UDP PCM/synchronization paths carry plaintext frames. A listener's `device_id`, advertised host name, IP address, invite code and legacy `TrustState` are **not authenticated identities**. A shared/venue Wi-Fi adversary can observe or tamper with these packets, impersonate peers, replay controls and inject audio. CRC and endpoint/sequence validation are not cryptographic integrity. Neither the existing gapless PCM tests nor green CI establish confidentiality or peer authentication.

The Android UniFFI bridge already refuses legacy-v3 durable trust and projects unverified joins as session-only; the desktop host projection also uses session-only. Preserve these fail-closed guards throughout migration. Do not advertise “Approved devices only” as working over v3, and do not promote any legacy device-ID trust record into verified trust.

**Threats in scope:** passive LAN capture, forged host/listener, MITM and downgrade, tampered/duplicated/reordered control and audio, stale route/generation replay, wrong-key reconnection, unauthorized admission and disclosure of invitation/session metadata. **Out of scope:** compromised endpoints/OS, malicious local host, network-owner denial of service, physical microphone/output interception and traffic-analysis resistance.

## Decision: QUIC/TLS 1.3 for v4, not bespoke TCP + datagram crypto

Implement an **explicitly versioned v4 secure transport** using the established Rust QUIC/TLS 1.3 stack (evaluate `quinn` with its maintained `rustls` integration and audited crypto provider at dependency pinning time). QUIC reliable, ordered streams carry join, approval, lifecycle, clock-control and TrackBoundary messages. QUIC DATAGRAM frames carry bounded PCM and optional lossy timing telemetry. Each host↔listener connection has independent traffic secrets, stream encryption and QUIC anti-replay/packet-number protection; application sample indices and stream generations remain authoritative for scheduling.

This avoids inventing a UDP AEAD nonce format, handshake or key derivation and avoids managing two unrelated security associations for TCP and UDP. The trade-offs are a nontrivial runtime migration, per-peer QUIC datagram sends rather than unauthenticated UDP broadcast, QUIC congestion behavior and a potentially different jitter/CPU budget. Do not put QUIC I/O, certificate validation, signing or decryption in the native audio callback.

The transport must **not** negotiate down from v4 to v3. v3 is a separate, visibly insecure lab-only configuration with an explicit opt-in and no durable trust; production defaults must require v4 after the secure implementation is shipped. Until then, existing v3 operation is a known security limitation, not a secure mode. A v4 endpoint must reject v3 wire headers and incompatible capability sets before admission or audio routing.

## Identity, enrollment and trust

1. **Host identity:** persistent P-256 signing key and public-key fingerprint. On Android keep the nonexportable private key in Android Keystore; on desktop use an OS-protected credential/key service. Host QUIC certificate/handshake identity must be pinned to the verified fingerprint. Signed QR may carry a host public key/fingerprint only if the QUIC handshake proves possession of the corresponding key. A manually entered address needs first-use fingerprint confirmation and an explicit mismatch failure on later joins.
2. **Listener identity:** independent persistent P-256 key. Prove possession over the authenticated, channel-bound v4 connection (mutual TLS if the platform signing interface supports it, otherwise a signed challenge bound to a TLS exporter with reviewed channel-binding semantics). The listener's claimed device ID/name is only display metadata. Never accept a signature over an unbound, reusable random challenge.
3. **Host-owned trust store:** versioned record keyed by verified listener public-key fingerprint and scoped to the host identity. Include status, creation/revocation timestamps and rotation/re-enrollment metadata. No migration of unverified v3 ID-based approvals to trusted. “Approve once” admits only the authenticated session; “Always allow” persists only after verified proof and a successful durable write; “Approved devices only” rejects unknown/revoked/unverified keys without silently inserting a pending request.
4. **Revocation/recovery:** a revoked key is denied on every new handshake/reconnect and its current routes are retired according to explicit policy. Key loss, Android restore or desktop key-store mismatch invalidates dependent trust and requires visible re-enrollment. No private key or session ticket is exported in backups, diagnostics, screenshots or logs. Never substitute a regenerated key without warning.

The `JoinRequestSummary` trust projection must come from the **verified v4 peer principal** rather than an FFI-supplied enum or claimed device ID. Tie pending approval, authorized route, request ID and connection generation to that principal; old sockets and approvals cannot authorize a replacement.

## Protocol and delivery contract

- Separate a short, bounded v4 handshake/capability exchange from encrypted application messages. Reject unknown versions, unexpected message directions, oversized frames, unsupported datagrams, malformed certificates and invalid proof before updating the host actor.
- Use one reliable ordered control stream per peer, with bounded send/receive queues and explicit application acknowledgements for admission/terminal decisions. Stop, Pause, StreamStart and TrackBoundary are never silently discarded under telemetry pressure. A full admission queue sends an authenticated busy/rejected response with finite retry guidance.
- Keep PCM lossy, bounded and per-peer. Enforce a generation-scoped `session_id`/`stream_id`, monotonic sample/sequence checks, deadline and replay rejection **after** authenticated decryption. QUIC anti-replay alone does not establish application-level timeline freshness after Pause/Resume or Stop/replay.
- Reconnect creates new traffic keys and a new connection generation; clear old cached routes, deduplication windows and pending approvals. The authoritative late-join snapshot (current Start, boundary and Pause/Stop state) is replayed over the new reliable control stream before PCM authorization.
- Maintain existing native output token across same-stream Pause/Resume and boundaries. New Stop/replay may allocate a new output generation; stale packets must not reach it. Never block callback on network backpressure.
- Establish and measure packet-size/MTU, jitter, clock-sync error, datagram loss, CPU and memory budgets on hosted tests. Do not claim physical speaker/DAC/Bluetooth performance from software results.

## Incremental implementation and acceptance gates

1. Add a v4 transport trait/adapter behind a compile-time and runtime feature boundary, pinned audited dependencies, certificate/key-store abstraction and explicit secure-vs-legacy mode in configuration. Preserve current v3 tests as **legacy-only** and prohibit automatic fallback. No production UI may label v3 encrypted.
2. Implement pinned host authentication and listener proof with host-owned trust records, rotation/revocation, manual fingerprint confirmation and safe Android/desktop restore behavior. Add fake-keystore tests and real Android Keystore emulator coverage; a software key is not evidence of hardware backing.
3. Migrate join/approval/timeout/reconnect to reliable v4 control with generation-aware request IDs and bounded busy responses. Exercise hostile-peer pressure, old-reader cleanup and Android bridge backpressure.
4. Move PCM/synchronization to QUIC datagrams, retaining sample-indexed packetizer/scheduler and native callback. Test two peers with **different connection keys** and independently recorded callback PCM across mixed-rate boundaries, edits, Pause/Resume, late join, Stop/replay and source failure.
5. Verify malformed/oversized frames, forged IDs/hosts, invalid certs and signatures, wrong-key QR, tag corruption, duplicated/retired packets, sequence exhaustion, downgrade, replay after reconnect and queue saturation. Include packet capture proving no plaintext session/invite/PCM payload on the wire. Add bounded fuzz/property tests.
6. Qualify exact-head GitHub-hosted Rust formatting, strict Clippy, workspace/FFI, Android JVM/lint/API29 instrumentation, desktop/Tauri/bundle, source policy and security/pressure regressions. Document explicit results and residual risks. Physical output/accessibility checks remain owner-deferred.

**Release gate:** No claim that F1/F2 are fixed, no automatic trusted admission and no “secure transport” label until an authenticated v4 socket → scheduler → native callback path and both platform adapters pass exact-head CI. A security ADR is design evidence, **not implementation evidence**.

## Alternatives considered

- **TLS 1.3 TCP plus hand-derived UDP AEAD:** retains socket layout but introduces custom nonce/key separation, rollover, replay and channel-binding responsibilities. Rejected as a larger cryptographic audit surface.
- **Encrypt only control TCP:** leaves PCM/sync spoofable and observable, so fails the threat model.
- **Trust invite codes, IP addresses or legacy IDs:** does not prove device possession and permits spoofing.
- **Immediate replacement of all v3 code:** would erase existing playback evidence before the secure adapter has parity. Instead maintain a deliberately isolated, clearly labeled legacy lab path while v4 is qualified.

## Outstanding engineering verification

Before choosing exact QUIC dependency versions, confirm Android minSdk/API29 integration, Rust/UniFFI threading and JNI-backed Keystore signing, QUIC exporter/channel-binding APIs, desktop OS key-store coverage, certificate rotation, transport memory limits and GitHub-hosted CI support. If QUIC's platform signing or latency proves unsuitable, amend this ADR with a measured alternative **before** implementing any custom datagram cryptography.
