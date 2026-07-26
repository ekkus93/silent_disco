# Silent Disco UI/UX Workflow Redesign — Implementation Progress

**Updated:** 2026-07-26  
**Target branch:** `master`  
**Source TODO:** `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_TODO.md`

This file records implementation status without changing unchecked acceptance items into unsupported claims. A task is listed as validated only when its behavior is covered by a deterministic unit or Compose test. The complete Gradle, lint, Rust, and connected-device matrix has not yet been observed for the current `master` revision because the available repository connector does not expose push-triggered workflow runs.

## Implemented P0 workflow foundation

- Added explicit presentation models for host setup, listener join stages, host/listener health, and structured user-facing problems.
- Added safe classifications and actions for permission denial, invalid invite code, host rejection, host-ended sessions, unreachable hosts, transport failures, synchronization failures, and playback failures.
- Centralized route names and back-stack helpers.
- Added a buffered one-shot UI effect channel for startup, host-dashboard, listener-playback, return-home, confirmation, and transient-message effects.
- Added saved consumption guards so startup and playback navigation are not repeated by recomposition.

### Remaining foundation limitation

- The effect channel currently belongs to the root Compose workflow rather than `MainViewModel`. Moving it into the ViewModel requires a safe targeted edit to the large ViewModel file and remains open.

## Implemented startup and permissions

- Startup is the graph entry point.
- Storage initialization, recoverable failure, fatal failure, retry, and support-report states are fail-visible.
- Home cannot be reached before storage is ready.
- Host and listener nearby-device permission requests are contextual.
- Audio selection uses Android's document picker without requesting broad media-library access.
- Permission denial remains visible on Home or Nearby Sessions and links to system Settings.
- Listener discovery now has one automatic scan trigger after permission state becomes ready; the permission callback no longer starts a duplicate scan.

### Remaining permission/discovery limitation

- Scan-resource release when navigating away still requires an authoritative public ViewModel/transport cancellation operation.

## Implemented navigation and destructive safety

- Host and listener workflows use centralized destinations.
- Returning Home clears the active workflow stack.
- Android back and top-app-bar back are intercepted for active hosting and playback.
- End-session and leave-session dialogs show contextual impact.
- Safe actions receive initial focus.
- Destructive actions are styled distinctly and guarded against duplicate submission.

## Implemented Home and host setup

- Home is role-first and no longer displays healthy storage or generic permission dashboards.
- Home and startup content scroll at large font scales.
- Host setup is split into Music and Access screens.
- Audio and session name are required before continuing.
- Invite codes are normalized, generated, and validated as four digits.
- Access cards have radio semantics and coherent screen-reader descriptions.
- Host-session creation failures remain visible on the Access screen.
- Permission failures offer Settings and Retry.
- Transport/start failures offer Retry and Share support report.
- A failed start does not leave a duplicate normal Start action beside the problem card.

## Implemented host dashboard and approvals

- Dashboard emphasizes session state, listener count, selected audio, playback, and health.
- Requests, Connected, and Needs attention are separated into tabs with counts and empty states.
- Listener requests show elapsed waiting time.
- Approve once, Always allow, and Reject explain approval lifetime.
- Per-request progress is visible while sending approval/rejection or persisting durable trust.
- Duplicate approval actions are disabled while one request is active.
- A newly reported delivery/persistence failure unlocks the request for retry.
- Connected-listener Disconnect moved into an overflow menu.
- Debug-only demo requests remain gated by `BuildConfig.DEBUG`.
- Existing persist-before-advertise durable-trust ordering remains unchanged.

### Remaining host-dashboard limitation

- Approval lifetime is still selected through the existing host-form field before calling the ViewModel. A dedicated ViewModel method accepting the requested lifetime remains preferable when a safe targeted ViewModel edit is available.

## Implemented discovery and join flow

- Nearby Sessions represents permission required, scanning, results, empty, and failure states.
- Results remain visible during safe refreshes and use stable name sorting.
- Session cards use plain-language access badges and accessible whole-card semantics.
- Session details and progress are one continuous destination.
- Join progress uses five user-facing stages.
- Progress indicators announce complete, active, and pending states.
- Playback navigation occurs only when both listener lifecycle and playback are actually Playing.
- Invalid invite codes can be edited in place and requested again.
- Persistent failures expose context-specific Retry, Edit code, Return to sessions, Settings, Reconnect, Resynchronize, and support-report actions.
- Raw technical error text is kept out of the normal join presentation.

## Implemented listener playback and Connection Help

- Now Playing prioritizes session/host identity, host-controlled playback, audio status, and local volume.
- Statuses cover Playing in sync, Buffering, Reconnecting, Audio out of sync, Playback stopped, Connection lost, and Playback problem.
- Healthy playback hides Fix connection.
- Troubled playback routes to Connection Help.
- Playback content scrolls at large font scales.
- Loading states and local-volume control have screen-reader descriptions.
- Connection Help uses icon-and-text status badges for connection, synchronization, and audio.
- Reconnect and Resynchronize appear only when valid.
- Status mappings prevent reconnecting, desynchronized, disconnected, or failed states from appearing healthy.

## Implemented Advanced Diagnostics and support reports

- The routed diagnostics screen uses progressive disclosure.
- Host and listener health summaries appear before technical metrics.
- Raw host, listener, and output metrics are collapsed by default.
- Expert tuning is collapsed and disabled until explicitly enabled.
- Manual Resync is presented as Resynchronize audio and is enabled only when valid.
- Any recent operation failure, including tuning persistence failure, remains visible at the top of Advanced Diagnostics.
- Support-report sharing is available from failure and diagnostics surfaces.
- Support reports redact known invite codes, session identifiers, device identifiers, and selected-file URIs even when embedded in error or metric text.

### Remaining diagnostics limitation

- Atomic Reset tuning to defaults is not implemented. It must persist one complete default settings object through the Rust-owned store before updating UI state. Repeated asynchronous increment/decrement calls would be unsafe and are intentionally not used.

## Implemented Settings

- Settings includes nearby-device permission readiness, local app-data readiness, troubleshooting, Advanced Diagnostics, and version/build information.
- Readiness uses text-and-icon badges rather than color alone.
- Raw storage errors are not displayed on the normal Settings screen.
- Approved-device management is hidden because the authoritative Rust/JNI contract currently exposes only upsert and single-device trust lookup. It has no list or delete operation.

## Implemented reusable components and invitation flow

- Added reusable StatusBadge, PrimaryProblemCard, EmptyState, LoadingState, RoleActionCard, AttentionBanner, and ConfirmationSheet patterns.
- Added a real invite sheet using existing session data.
- Hosts can copy the invite code and share plain-language Android instructions.
- The invite payload omits internal session IDs and does not invent unsupported QR behavior.

## Added tests

### Unit/presentation tests

- Presentation-state and join-stage mappings.
- Structured listener failure classifications and action mappings.
- Host/listener health classifications.
- One-shot startup/playback navigation guards.
- Invite-code generation and validation.
- Contextual permission selection.
- Approval-progress and waiting-duration labels.
- Listener playback status/tone mappings.
- Connection Help indicator mappings.
- Settings storage readiness mappings.
- Nearby-session access labels.
- Invitation text for each access mode.
- Diagnostics/support-report identifier redaction.

### Compose tests

- Startup loading, recoverable, and fatal states.
- Role-first Home and required host-music inputs.
- Invalid invite-code editing in the join flow.
- Host-start permission and transport failures.
- Dashboard approval progress and duplicate-action disabling.
- Discovery permission, scanning, and result states.
- Healthy and desynchronized listener playback.
- Connection Help recovery-action visibility.
- Advanced Diagnostics expert-control gating.
- Settings ready/error/permission states and conditional approved-device entry.
- Confirmation safe-focus and duplicate-submit behavior.
- Home behavior at 200% font scale.

## Added Compose preview coverage

- Added Android Studio previews for startup loading, recoverable failure, and fatal failure.
- Added ready-state previews for Home, Host Music, Host Access, Advanced Diagnostics, and Settings.
- Added Host Dashboard previews for no listeners, a pending request, connected listeners, and a listener needing attention.
- Added Nearby Sessions previews for scanning, empty, and results states.
- Added Session Join previews for before-request, waiting-for-approval, rejected-invite-code, and connection-failure states.
- Added Now Playing previews for healthy, buffering, reconnecting, and desynchronized states.
- Added healthy and actionable Connection Help previews.
- Added end-session and leave-session confirmation previews.

### Remaining preview limitation

- Host Dashboard tab selection and the enabled Expert tuning state remain interaction-driven internal Compose state. The previews expose the corresponding data and default UI, but do not force those internal controls into a selected or enabled state.

## Remaining major work

1. Run and observe the complete permanent CI matrix for the current `master` revision.
2. Fix any compile, lint, unit-test, or instrumentation failures found by that run.
3. Move one-shot effects into `MainViewModel` when a safe targeted edit path is available.
4. Add one atomic Rust-persisted tuning-reset operation.
5. Add authoritative Rust/JNI trusted-device list/delete APIs before enabling management UI.
6. Add an explicit scan cancellation/release operation for destination exit.
7. Add full navigation/integration tests with a controllable fake ViewModel or workflow state holder.
8. Add broader 200% font, landscape, tablet, and small-window tests.
9. Add explicit preview seams for tab-selected Host Dashboard and enabled Expert tuning states only if they do not duplicate production state ownership.
10. Complete physical two-device host/listener acceptance testing.
11. Remove obsolete screens only after replacement workflows pass the complete validation matrix.
