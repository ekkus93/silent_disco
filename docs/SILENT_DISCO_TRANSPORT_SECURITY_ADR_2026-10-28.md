# ADR 2026-10-28: Authenticated LAN transport and admission migration

**Status:** Proposed implementation decision; not shipped or security-qualified.
**Scope:** F1/F2 of [the code-review remediation specification](SILENT_DISCO_CODE_REVIEW_REMEDIATION_SPEC_2026-10-27.md).
**Baseline:** master `d7c348f48ab2db6abff4a5ac04cadc3ba2922e68` (2026-10-28).
**Authority:** Shared Rust actor owns admission, queue and playback; Android and desktop own OS credential adapters.

## Threat model and current exposure

Assume an attacker can join the venue Wi-Fi, capture and inject TCP/UDP packets, impersonate advertised sessions, replay recordings, and claim any listener device ID. The attacker cannot initially read a correctly protected endpoint private key or compromise an endpoint OS. Availability against LAN jamming, hostile access points, compromised hosts/listeners, physical audio interception, and malicious audio sources are separate threats.

**The current v3 socket path is plaintext and not authenticated.** The socket worker validates framing, session IDs, claimed device IDs, route addresses and authorization state, but those checks are not cryptographic identity or packet authentication. A signed QR, if present, is not sufficient unless the subsequent handshake proves possession of the corresponding host key. A CRC is not a MAC. Android currently submits join inputs as `UNKNOWN`; desktop uses `SessionOnly`. The Rust trusted-only policy rejects those values, but neither platform has a handshake-verified durable identity for trusted auto-admission. A successful explicit approval is therefore **session-only security**, regardless of UI wording or a legacy claimed-ID trust record. No production safety or trusted-only claim may rely on v3.

## Decision: secure v4 as an explicit new transport, not opportunistic TLS

1. Use **TLS 1.3** for the TCP control channel, with a mature maintained implementation (Rust `rustls` on Rust-owned sockets, or platform TLS where the OS-backed signing key requires it). Reject TLS 1.2 and anonymous/no-verification configurations. Pin the **host public-key identity** presented by an authenticated first-use enrollment or signed invitation; DNS/IP names and BLE advertisements are discovery hints, not trust roots. Manual address entry must show a fingerprint and require explicit first-use confirmation. A changed pinned key fails closed and offers a separate re-enrollment action.
2. Give each listener a stable, non-exportable **P-256 ECDSA signing key** where supported by the Android Keystore; use the desktop OS credential/key store with an equivalent protected key. The host sends a fresh, single-use challenge **inside the authenticated TLS channel**. The listener signs a domain-separated transcript containing the v4 protocol label, challenge, session generation, both endpoint public-key fingerprints, and an exporter-derived channel binding. Verify the signature and challenge freshness before projecting a verified listener identity into Rust. No self-declared device ID, display name, source IP or legacy trust enum is evidence of possession. The exact key API and TLS adapter must be validated in an Android API 29 emulator and on each desktop target before choosing the final library versions.
3. Derive **separate audio and synchronization datagram traffic secrets** for each direction and peer from the authenticated TLS exporter using distinct labels and the session generation. Use a vetted AEAD (AES-256-GCM or ChaCha20-Poly1305 as supported by the selected library), with a standard 96-bit nonce formed from a per-key random/session-specific prefix and an explicit bounded monotonically increasing counter. Persist no traffic key or nonce state. On counter exhaustion, reconnect/rekey **before** reuse; reject wrapped sequence numbers. Authenticate session ID, stream generation, channel, sequence, sample index and presentation metadata as associated data. Keep a bounded per-peer, per-key replay window; do not advance it on failed authentication. Retire old keys/windows on Stop, revocation, reconnect and new playback generation. No encryption/decryption occurs on the real-time audio callback.
4. Introduce a **distinct v4 wire preface/version** before any join or PCM delivery. Reject legacy-v3 and mixed-version peers with a typed, user-visible incompatibility result; never silently downgrade after a failed TLS handshake. An explicitly enabled, clearly marked local-lab v3 transport may be retained for existing deterministic tests during migration, but must not be auto-selected or represented as secure production operation. No v3 peer can be auto-trusted by a v4 host.
5. Host-side trust records are keyed by the **verified listener public-key fingerprint**, scoped to the host's own identity and versioned for migration. `Approve once` binds only the authenticated connection generation. `Always allow` writes the verified key before advertising durable trust. `Approved devices only` admits only an already trusted, non-revoked verified key; unknown/revoked/missing proof is rejected, not converted to pending. Legacy device-ID trust records are never migrated to verified trust automatically. Rotation/loss requires explicit re-enrollment; revocation rejects reconnect and retires active routes under the documented host policy.
6. Maintain a single authoritative admission generation across transport, actor and UI. No approval, UDP route, cached StreamStart, clock-sync reply or PCM is released before verified admission. On failed handshake, tag, timeout or persistence, close provisional routes and surface a finite error. Explicit Stop/replay must rotate generation-bound traffic keys and clear stale control caches.

## Migration and rollback

**Phase A — security scaffolding:** record the v3 threat and exact legacy behavior in tests; implement versioned key records, OS key adapters and strict parsing without enabling v4 traffic. **Phase B — control:** add authenticated TLS, pinned host confirmation, signed listener challenge and v4 admission on real loopback sockets; reject v3 and forged identities. **Phase C — datagrams:** add AEAD/replay/key rollover, then run the existing socket/scheduler/native-callback exact-PCM matrix over encrypted channels. **Phase D — platform and release:** integrate Android and desktop enrollment, revocation, restore/key-loss UX, and disable plaintext production paths by default; keep an opt-in lab-only compatibility mode only if separately justified.

Rollback is to a **non-production, explicitly labeled lab build**, not to automatic plaintext fallback. A secure-session failure cannot switch protocols, preserve authorization, or reuse keys. No production migration gate is closed until both platforms have passed exact-head positive and adversarial tests.

## Required acceptance matrix

| Gate | Positive evidence | Negative/pressure evidence |
| --- | --- | --- |
| Host identity | QR/manual pinned fingerprint, reconnect and rotation | Forged host, swapped QR key, pin mismatch, bad certificate |
| Listener identity/admission | Enrollment, approve once/always, host restart, same-key reconnect | Claimed-ID spoof, wrong key, unknown/revoked key, stale approval generation |
| Control channel | TLS 1.3 Start/Pause/Stop/TrackBoundary, authoritative ordering | Plaintext/downgrade, malformed/oversized frame, tampered controls |
| Datagram | Independent peer keys, exact sample indices/PCM through scheduler and native callback | Bad tag, replay/old generation, counter wrap, wrong channel/peer |
| Playback lifecycle | Multi-track edit, pause/resume, Stop/replay, late join/reconnect | Stale cache, duplicate approval, network loss, queue saturation |
| Platforms | Android JVM/API29, desktop/Tauri, hosted Linux bundle, source policy | Key loss/restore, invalid cert, timeout, cancelled FFI operations |

Use pinned Rust formatting/strict Clippy/workspace and UniFFI tests, Android build/JVM/lint/API29, desktop frontend/Rust/bundle, and the source-file policy **at the same exact head**. Capture bounded performance/jitter/CPU measurements on emulator/hosted runners without claiming physical speaker/DAC or Android-to-Android results. Keep the hardware items in §10 of the remediation TODO unchecked.

## Existing implementation map and outstanding blockers

- `rust/silent-disco-core/src/transport/socket/shared.rs` reads raw TCP bytes and `host_workers.rs` uses unauthenticated UDP routes; both need a secure transport boundary.
- `rust/silent-disco-core/src/runtime/host_admission.rs` already rejects untrusted input in trusted-only mode, but its `Trusted` input has no cryptographic proof type.
- `desktop/src-tauri/src/platform/host_join_projection.rs` supplies `SessionOnly`; `app/src/main/java/com/ekkus/silentdisco/app/MainViewModelRustHost.kt` supplies `UNKNOWN`. Neither can currently enable verified auto-admission.
- Android backup is already disabled and both legacy/modern XML policies exclude the private app root. The API29 instrumentation policy test checks the **merged installed manifest** and XML rules, not a real device-to-device/cloud restore.
- No v4 transport, key enrollment, TLS exporter, AEAD replay window or OS-backed identity proof has been implemented by this ADR. **F1/F2 remain open.**
