# Desktop Block 22 — Manual Endpoint Host Workflow

**Status:** Complete

The desktop host now publishes authoritative manual connection information and accepts a real control-only listener connection without relying on mDNS.

## Authoritative connection projection

- The desktop backend combines the exact shared-core `SessionAdvertisement` captured at bind time with the actual TCP/UDP endpoint returned by the shared transport.
- The bounded DTO exposes the host address, control/synchronization/audio ports, session identifier, protocol version, invite-code requirement, and optional expiration.
- The current manual endpoint has no expiration policy, so expiration is explicitly `null` rather than invented by the frontend.
- Session creation is not presented as successful before the shared sockets bind.

## Real transport event bridge

- A bounded desktop receive worker drains the shared `HostTransportNode` into the authoritative core actor.
- A validated control-channel join request becomes the canonical pending-join summary in the actor snapshot.
- Identified but unapproved listeners can receive the host `Hello` over TCP without receiving UDP synchronization or audio authorization.
- Listener disconnect removes the corresponding pending request and remains visible in authoritative state.
- Worker failures and shutdown/cleanup failures remain typed and visible.

## Host Session screen

- Active host lifecycles route to `HostSessionScreen`.
- The screen displays authoritative lifecycle, transport, playback, endpoint, session, request, and connected-listener state.
- Copy controls expose the host address and bounded connection payload.
- Playback controls are intentionally disabled and labelled as unsupported until the later audio pipeline blocks.
- Transport/core failures remain visible.
- End-session submits the expected snapshot revision and does not optimistically return to setup.

## Control-only proof

The deterministic loopback test:

1. creates a real authoritative desktop host session;
2. binds the shared TCP control and UDP synchronization/audio sockets;
3. connects a shared listener to the advertised manual endpoint;
4. completes the join-request and host-`Hello` exchange;
5. observes the pending request in the core snapshot;
6. verifies no audio datagram success is claimed; and
7. verifies disconnect is reflected in authoritative state.

## Validation

- Shared handshake commit: `4c132f28f8807dd5afb6a791f747f96515051d67`
- Runtime/DTO commit: `47724cd6a4f931f14b003cb7bed249546b8fbdf7`
- Host Session UI commit: `88e2b851feba8a06cdd0016ef840f48762d3a94c`
- Shared transport run: `30616456164`
- Runtime/DTO run: `30619790710`
- Host Session UI run: `30620484607`
- Final Actions run: `30620932603`
- Direct-master final-validation input: `3f9b90aca0549e5870b34d12cee83c514a2ccd40`
- Focused control-only coverage and the complete Rust, desktop frontend/backend, Linux bundle, Android build/test/lint, ABI, managed-device instrumentation, lockfile reproducibility, and source-size gates passed.
