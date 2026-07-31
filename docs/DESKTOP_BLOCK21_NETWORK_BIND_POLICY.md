# Desktop Block 21 — Network Interface and Bind Policy

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

- Implementation commit: `bef33cab2798c41172eced93747ecf73927dcd90`
- Focused UI/network publication run: `30613180304`
- Final Actions run: `30613572498`
- Direct-master final-validation input: `fd081a1574f54956754adcd40c0578933e468c1f`
- The focused network tests and complete Rust, desktop frontend/backend, Linux bundle, Android build/test/lint, ABI, managed-device instrumentation, lockfile reproducibility, and source-size gates passed.
