import os
from pathlib import Path

run_id = os.environ["GITHUB_RUN_ID"]
input_sha = os.environ["BLOCK21_INPUT"]
implementation_sha = "bef33cab2798c41172eced93747ecf73927dcd90"
ui_run_id = "30613180304"

Path("docs/DESKTOP_BLOCK21_NETWORK_BIND_POLICY.md").write_text(
    f"""# Desktop Block 21 — Network Interface and Bind Policy

**Status:** Complete

Desktop hosting now exposes an intentional, validated private-LAN bind policy instead of advertising or binding every interface blindly.

## Interface discovery and classification

- The Tauri platform layer enumerates active interfaces and addresses through the reviewed Rust `netdev` API.
- Enumeration is bounded to 256 interfaces and 1,024 addresses.
- Addresses are classified as loopback, link-local, private LAN, VPN, container, or other.
- The supported automatic baseline is active private-LAN IPv4. IPv6 addresses remain visible and classified but are not selected under this baseline.
- Automatic mode selects a sole safe candidate, or the sole default-route safe candidate when that resolves ambiguity.
- Multiple unresolved safe candidates require an explicit user choice.
- Explicit interface/address preferences are revalidated against a fresh snapshot before binding.
- Disappearing or changed interfaces are surfaced as visible state instead of silently falling back.

## Shared transport binding

- The desktop platform passes the validated address into the shared Rust `TransportFactory`.
- Host startup is not reported as successful until the TCP control and UDP synchronization/audio endpoints bind.
- The authoritative binding reports the actual interface, address, and bound control/sync/audio ports returned by the shared transport.
- Endpoint/address mismatches trigger cleanup, and cleanup failures remain attached to the primary failure.
- Active bindings are rechecked against current interface state so address or interface loss is visible.

## Desktop UI

- `HostNetworkPolicyCard` displays automatic policy, candidates, rejection reasons, explicit selection, active binding details, and interface-change errors.
- Host creation remains disabled until the network snapshot resolves to a safe, current selection.
- The UI uses generated Rust-derived bindings and typed Tauri command failures.

## Deterministic coverage

The focused backend and frontend suites cover:

- loopback-only environments;
- one private-LAN interface;
- multiple private-LAN interfaces and default-route resolution;
- VPN and container interfaces;
- disappearing requested addresses;
- occupied ports;
- partial-bind cleanup and cleanup-error preservation;
- active interface changes;
- the selected IPv4/IPv6 baseline;
- automatic and explicit UI selection, stale candidates, accessibility, and host-readiness gating.

## Validation

- Implementation commit: `{implementation_sha}`
- Focused UI/network publication run: `{ui_run_id}`
- Final Actions run: `{run_id}`
- Direct-master final-validation input: `{input_sha}`
- The focused network tests and complete Rust, desktop frontend/backend, Linux bundle, Android build/test/lint, ABI, managed-device instrumentation, lockfile reproducibility, and source-size gates passed.
"""
)

path = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
text = path.read_text()
start = text.index("## Block 21")
end = text.find("## Block 22", start)
if end == -1:
    end = len(text)
block = text[start:end].replace("- [ ]", "- [x]")
if "**Completion evidence:**" not in block:
    block = block.rstrip() + (
        f"\n\n**Completion evidence:** Actions run `{run_id}` passed against direct-master "
        f"input `{input_sha}`. See `docs/DESKTOP_BLOCK21_NETWORK_BIND_POLICY.md`.\n\n"
    )
path.write_text(text[:start] + block + text[end:])

path = Path("memory.md")
path.write_text(
    path.read_text().rstrip()
    + f"""

## 2026-07-31 — Desktop Block 21 network bind policy complete

- Desktop hosting enumerates bounded interface snapshots and classifies loopback, link-local, private LAN, VPN, container, and other addresses.
- Automatic selection is restricted to an unambiguous active private-LAN IPv4 candidate; ambiguity requires explicit user selection.
- Explicit preferences are revalidated immediately before the shared Rust transport binds TCP control and UDP synchronization/audio endpoints.
- Actual bound addresses and ports, interface changes, bind failures, partial cleanup, and cleanup failures remain visible and typed.
- Host Setup now includes an accessible network-policy card and blocks session creation until the network selection is ready.
- Implementation commit: `{implementation_sha}`.
- Focused publication run: `{ui_run_id}`.
- Direct `master` work; no branch or PR.
- Final validation run: `{run_id}`.
- Validated input: `{input_sha}`.
"""
)
