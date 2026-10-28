# ADR — Secure Host/Listener Transport and Verified Device Identity

**Date:** 2026-10-27
**Status:** Proposed implementation direction; NO authenticated channel is yet delivered by this ADR.
**Source:** [Code review remediation spec](SILENT_DISCO_CODE_REVIEW_REMEDIATION_SPEC_2026-10-27.md), findings F1/F2; [remediation TODO](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md)
**Decision owner:** Project implementation, subject to security acceptance and product UX review
**Legacy wire baseline:** version 3, TCP control + UDP audio/sync with no cryptographic confidentiality or peer authentication.

## Threat model and existing limitation

The app is intended for venues where the local network may include unknown peers. Assume an adversary can observe, spoof, replay and inject LAN TCP/UDP, attempt invite-code theft, replay a retired session, impersonate a previously approved device by sending its claimed ID, flood control/events, and trick a listener with a fraudulent QR or manual endpoint.

Today's protocol-v3 checks of session IDs, addresses, packet sequence and CRC detect some accidental errors but **do not authenticate or encrypt the transport**. A signed QR alone does not authenticate the subsequent socket without a cryptographic channel binding. Never claim v3 plaintext is safe on shared/untrusted Wi-Fi. Host-side `TrustState::Trusted` from a legacy device ID is not proof of key possession. Reject or clearly warn on the unsupported security mode rather than silently downgrade.

Out of scope: endpoints already compromised; hostile OS/kernel; physical speaker/DAC interception; radio/network-owner denial of service; and physical gapless acceptance (explicitly owner-deferred).

## Decision: one secure Rust network stack for both platforms

**Preferred direction:** replace independent plaintext control TCP and UDP payload handling with a single mutually identified QUIC session implemented in shared Rust, using a vetted maintained QUIC/TLS 1.3 implementation (evaluate `quinn` + `rustls` and their pinned compatible versions). QUIC reliable ordered streams carry authorization, Start/Pause/Stop, Queue/TrackBoundary, sync and error messages. QUIC DATAGRAM frames carry the packetized PCM when supported; the protocol must bound payload sizes to the path MTU and report when DATAGRAM is unavailable. Do not silently reroute PCM onto a backpressured reliable control stream as if timing were equivalent.

QUIC is preferred to hand-deriving application datagram AEAD keys from a separate TLS control socket because it provides an established transport with encryption, authentication, packet replay protection, key updates and connection migration without custom nonce management. This is an architectural choice to verify in a reproducible spike, not a claim that QUIC will meet 5-ms audio latency without benchmarking.

Keep legacy v3 decoding for offline fixture migration where needed, but **do not provide an automatic fallback to plaintext in production**. Bump secure protocol to v4 (or a separately negotiated secure capability) and reject incompatible peers with a visible error. Lab-only insecure mode, if retained, is explicitly opted in and excluded from release bundles.

## Identity, bootstrap and enrollment

1. Identify the host with a stable pinned P-256 public-key fingerprint; re-evaluate reuse of existing P-256 signed invitation identity, ensuring certificate-to-invitation binding rather than assuming a matching string device ID.
2. The listener authenticates the host by comparing the negotiated certificate/key to the key embedded in a signed, freshness-checked QR invitation or to an explicitly accepted/pinned key in a manual-address join flow. First manual-use and rotation require user-confirmed fingerprint display; mismatches fail closed.
3. For device trust, bind a listener to an OS-protected private key: Android Keystore and desktop OS credential store. Require a challenge or authenticated client-certificate proof under the established secure session; bind the proof to host fingerprint, session ID, transport generation and expiration.
4. Host stores versioned, host-scoped verified public-key fingerprints with trust, revocation and rotation metadata. A self-asserted string ID, network address or prior plaintext `Trusted` value can NEVER produce a verified trust result.
5. “Approve once” grants only the current secure session. “Always allow” persists only after successful proof-of-possession; storage failure yields session-only approval plus visible notification. “Approved devices only” auto-admits only verified enrolled keys; unknown/revoked/invalid-key clients are rejected with an actionable reason. Enrollment must be a separate owner-approved workflow.
6. On key loss, restore, profile change, host restart or key rotation, require re-enrollment; no legacy trusted-device table may be imported as cryptographically verified merely from a matching device ID.
7. No private key, bearer token, invite code, raw challenge or session ticket may enter logs, crash reports, diagnostics, source snapshots, backups or CI artifact outputs.

## Packet, clock and lifecycle invariants

- Secure session ID + authenticated peer fingerprint + generation own the control/audio routes. Only packets from the currently authorized peer and active stream generation may be admitted to the Rust scheduler.
- QUIC transport packet numbers/replay protection and application sample indices serve different purposes. Retain the v3 audio sequence/sample-index validation after decryption, including late join, reorder, pause, reconnect and Stop/replay.
- Authenticate all control messages that can mutate state: join, approval, denial, StreamStart, Pause, Stop, boundary, clock-sync and disconnect.
- Disable early data/0-RTT for state-changing operations; no replayable approval/Stop is acceptable. Reconnect establishes a new authenticated generation and invalidates retired tickets/requests.
- Cap control message lengths, outstanding streams, pending joins, datagram buffers, certificates/handshake time, and per-peer work. Backpressure must never occur on an Oboe callback or inside a decoder.
- Crypto work stays in Rust transport workers, never in Compose, React or native audio render callback. FFI retains typed state and errors. Compare packet cadence, CPU, memory and jitter with the pinned v3 lab baseline before claiming gapless parity.
- Private key/certificate material must be unloaded at session end and cannot be duplicated into diagnostic capture.

## Implementation phases

**Phase A — design and benchmarking:** pin vetted libraries after dependency/security review; implement an isolated real-loopback QUIC spike with encrypted control and 240-frame PCM datagrams over a 48-kHz stereo stream. Confirm path-MTU behavior, end-to-end delay, and decode→scheduler→native callback sample exactness with two listeners.

**Phase B — trust and admission:** shared Rust verified-identity principal, OS identity adapters, signed-QR/manual binding, enrollment/approval/revocation; wire Android MainViewModelRustHost and desktop host_join_projection to trusted-key verification. No “Trusted” from self-reported ID.

**Phase C — migration and negative testing:** secure v4 negotiation and observable v3 rejection, pinned-host mismatch recovery, malicious/reordered/duplicated datagrams, bad certificates, replayed approvals, key rotation, peer-route replacement and failure recovery.

**Phase D — production rollout and evidence:** Android API29 emulator and desktop hosted CI, multi-peer real socket soak, binary build and secure-by-default checks. Keep the old gapless TODO open until all software accepted and physical measurements remain explicitly deferred.

## Acceptance criteria

- A passive capture contains no invite code, auth token, metadata or PCM plaintext; forged/tampered frames fail authentication and do not mutate host/listener actor state.
- Correctly enrolled key auto-admits in approved-only mode across Android and desktop; an unknown, revoked or spoofed device never auto-admits even with the same claimed ID.
- A signed QR/manual host identity mismatch is visible and blocks control/audio; incompatible v3 peer cannot silently downgrade secure v4.
- Two listeners render exact PCM across source boundaries, including live edits, Pause, Stop/replay, late join, reconnect and failure recovery with no extra native output reopen, preserving existing application sample-index semantics.
- Negative crypto and fuzz tests, strict Rust Clippy/format, Android instrumentation, desktop build and exact-head full CI pass on one commit. The attack model and any remaining limitations are documented before release.

**Unresolved implementation risks:** QUIC DATAGRAM path-MTU limits, mobile firewall/peer discovery behavior, 5-ms packet overhead, long-lived P-256 key API support across platforms, and secure identity persistence. These must be resolved with the spike and explicit tests, not speculative claims.
