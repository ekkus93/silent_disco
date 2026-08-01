# Desktop Block 23 — Delivery-First Listener Management

**Status:** Complete
**Validation run:** `30678111276`
**Validated direct-master input:** `8f9d156d5d94cba7178cc01ad8cb546d691da003`

## Implemented behavior

- The desktop exposes revision-aware `approve_join_request`, `reject_join_request`, and `remove_listener` commands. Request and listener identifiers are validated before actor submission.
- `DesktopCoreObserver` routes transport effects to the active host transport and storage effects to a dedicated bounded database-effect worker. Neither effect category is forwarded to React for execution.
- The host transport owns a bounded outbound effect queue on the same thread that owns the shared `HostTransportNode`. Approval, rejection, and disconnect messages use the identified control peer and submit exact `DeliveryCompleted` evidence to the authoritative actor.
- Failed targeted sends report one intended peer, zero successful peers, and one failed peer. The actor therefore keeps the join request or listener visible and publishes a structured delivery failure rather than claiming success.
- Trusted-device persistence completes through the Rust `DatabaseWorker` before approval delivery is emitted. Existing trusted-device metadata is updated transactionally and storage failures remain correlated to the original operation.
- The host-session DTO now includes request age, synchronization confidence, offset/RTT/drift summaries, last delivery evidence, recoverable capability, and Rust-derived removal permission.
- `HostSessionScreen` keeps requests and listeners visible while commands are pending. It changes presentation only after a newer authoritative snapshot confirms completion or failure.
- `ListenerDetailScreen.tsx` exposes lifecycle, trust, last contact, synchronization, delivery state, retry/resync capability, structured errors, and a core-gated removal action.

## Automated evidence

The guarded run executed:

- focused desktop backend tests for command validation, DTO projection, storage effects, real socket approval delivery, missing-peer delivery failure, and the Block 22 loopback regression;
- frontend tests for delivery-confirmed approval, zero-recipient failure, partial delivery, rejection/stale failure, trusted-device policy, duplicate-click prevention, persistent errors, listener detail, and removal confirmation;
- generated-binding regeneration and stale-binding verification;
- shared Rust formatting, strict Clippy, and all-feature tests;
- desktop formatting, lint, TypeScript, all Vitest tests, production frontend build, backend formatting, strict Clippy, all-feature tests/check, and Linux Tauri bundle build;
- Android debug, POC, release, and instrumentation builds, unit tests, lint, four-ABI native-library checks, and the managed Pixel 2 API 29 instrumentation suite;
- source-file line-count and lockfile invariants.

## Scope boundary

This block proves desktop-owned listener admission and management, including real control-message delivery through the shared socket transport. It does not claim physical Android interoperability. Connecting a physical Android listener to the desktop host over the LAN and recording that evidence is Desktop Block 24.
