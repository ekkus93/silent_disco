import os
from pathlib import Path

run_id = os.environ["GITHUB_RUN_ID"]
input_sha = os.environ["BLOCK22_INPUT"]
shared_sha = "4c132f28f8807dd5afb6a791f747f96515051d67"
runtime_sha = "47724cd6a4f931f14b003cb7bed249546b8fbdf7"
ui_sha = "88e2b851feba8a06cdd0016ef840f48762d3a94c"

Path("docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md").write_text(
    f"""# Desktop Block 22 — Manual Endpoint Host Workflow

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

- Shared handshake commit: `{shared_sha}`
- Runtime/DTO commit: `{runtime_sha}`
- Host Session UI commit: `{ui_sha}`
- Shared transport run: `30616456164`
- Runtime/DTO run: `30619790710`
- Host Session UI run: `30620484607`
- Final Actions run: `{run_id}`
- Direct-master final-validation input: `{input_sha}`
- Focused control-only coverage and the complete Rust, desktop frontend/backend, Linux bundle, Android build/test/lint, ABI, managed-device instrumentation, lockfile reproducibility, and source-size gates passed.
"""
)

path = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
text = path.read_text()
start = text.index("## Block 22")
end = text.find("## Block 23", start)
if end == -1:
    end = len(text)
block = text[start:end].replace("- [ ]", "- [x]")
if "**Completion evidence:**" not in block:
    block = block.rstrip() + (
        f"\n\n**Completion evidence:** Actions run `{run_id}` passed against direct-master "
        f"input `{input_sha}`. See `docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md`.\n\n"
    )
path.write_text(text[:start] + block + text[end:])

path = Path("memory.md")
path.write_text(
    path.read_text().rstrip()
    + f"""

## 2026-07-31 — Desktop Block 22 manual endpoint host workflow complete

- Desktop exposes authoritative manual host connection information without requiring mDNS.
- The DTO combines the shared-core session advertisement with the actual bound control/synchronization/audio endpoint.
- The desktop transport worker feeds real join/disconnect events into the authoritative actor.
- Pre-approval TCP Hello does not grant UDP synchronization or audio authorization.
- Host Session UI shows connection details, copy controls, pending and connected listeners, visible failures, disabled future playback controls, and revision-aware end-session behavior.
- Shared handshake commit: `{shared_sha}`.
- Runtime/DTO commit: `{runtime_sha}`.
- Host Session UI commit: `{ui_sha}`.
- Direct `master` work; no branch or PR.
- Final validation run: `{run_id}`.
- Validated input: `{input_sha}`.
"""
)
