# Silent Disco UI/UX Workflow Redesign — Implementation Progress

**Updated:** 2026-07-26  
**Target branch:** `master`  
**Source TODO:** `docs/SILENT_DISCO_UI_UX_WORKFLOW_REDESIGN_TODO.md`

This record distinguishes implemented behavior and deterministic source coverage from checks that still require an observed CI or physical-device run. It does not convert unobserved acceptance criteria into success claims.

## Implemented workflow foundation

- Added presentation models for host setup, listener join stages, host/listener health, approval progress, and structured user-facing problems.
- Added safe classifications and actions for permission denial, invalid invite code, host rejection, host-ended sessions, unreachable hosts, transport failures, synchronization failures, and playback failures.
- Centralized route names and back-stack helpers.
- Moved one-shot startup, host-dashboard, listener-playback, return-home, confirmation, and transient-message effects out of root Compose state into a lifecycle-owned `WorkflowViewModel`.
- Added consumption guards so startup and playback navigation are not repeated by recomposition or repeated state delivery.
- Added late-callback handling so a cleared workflow state holder does not turn a lifecycle race into an application crash.

## Implemented startup, permissions, and discovery lifecycle

- Startup is the navigation graph entry point.
- Storage initialization, recoverable failure, fatal failure, retry, and support-report states are persistent and fail-visible.
- Home cannot be reached before storage is ready.
- Host and listener nearby-device permissions are requested contextually.
- Audio selection uses Android's document picker without broad media-library access.
- Listener discovery has one automatic scan trigger after permission state becomes ready.
- BLE discovery exposes authoritative active-scan cancellation and reports teardown failures.
- Wi-Fi Direct discovery and pending join transport teardown use explicit success/failure callbacks.
- Leaving Nearby Sessions, cancelling a join, and leaving playback invoke the discovery/transport lifecycle controller.

## Implemented navigation and destructive safety

- Host and listener workflows use centralized destinations.
- Returning Home clears the active workflow stack.
- Android back and top-app-bar back are intercepted for active hosting and playback.
- End-session and leave-session dialogs describe the impact of the action.
- Safe actions receive initial focus.
- Destructive actions are visually distinct and guarded against duplicate submission.

## Implemented Home and host setup

- Home is role-first and does not display healthy storage or generic permission dashboards.
- Home and startup content remain scrollable at large font scales.
- Host setup is split into Music and Access destinations.
- Audio and session name are required before continuing.
- Invite codes are normalized, generated, and validated as four digits.
- Access choices have radio semantics and coherent screen-reader descriptions.
- Permission and transport failures remain visible on Host Access with appropriate Settings, Retry, and support-report actions.

## Implemented host dashboard and approvals

- Dashboard emphasizes session state, listener count, selected audio, playback, and health.
- Requests, Connected, and Needs attention are separated into tabs with counts and empty states.
- Listener requests show elapsed waiting time.
- Approve once, Always allow, and Reject explain approval lifetime.
- Per-request progress remains visible while approval, rejection, or durable-trust persistence is active.
- Duplicate actions are disabled while a request operation is active, and reported failures unlock retry.
- Connected-listener Disconnect is placed in an overflow menu.
- Durable trust remains persist-before-advertise and visibly downgrades to session-only approval on persistence failure.

### Remaining approval API refinement

- `WorkflowViewModel` still stages the requested approval lifetime through `HostFormState` before invoking `MainViewModel.approveJoinRequest`. The next production-code slice should make approval lifetime an explicit argument captured by the approval command itself, so concurrent requests cannot depend on shared mutable form state.

## Implemented discovery and listener join flow

- Nearby Sessions represents permission-required, scanning, results, empty, refresh, and failure states.
- Results remain visible during safe refreshes and use stable name sorting.
- Session cards use plain-language access badges and accessible whole-card semantics.
- Session details and progress are one continuous destination.
- Join progress uses five user-facing stages with complete, active, and pending accessibility descriptions.
- Playback navigation occurs only when listener lifecycle and playback are both actually Playing.
- Invalid invite codes can be edited in place and requested again.
- Persistent failures expose context-specific Retry, Edit code, Return to sessions, Settings, Reconnect, Resynchronize, and support-report actions.
- Raw technical errors remain outside the normal join presentation.

## Implemented listener playback and Connection Help

- Now Playing prioritizes session/host identity, host-controlled playback, audio status, and local volume.
- Statuses cover Playing in sync, Buffering, Reconnecting, Audio out of sync, Playback stopped, Connection lost, and Playback problem.
- Healthy playback hides Fix connection; troubled playback routes to Connection Help.
- Playback remains scrollable at large font scales.
- Connection Help uses icon-and-text status badges and shows Reconnect or Resynchronize only when valid.

## Implemented Advanced Diagnostics and tuning reset

- Advanced Diagnostics uses progressive disclosure and keeps user-facing health summaries ahead of raw metrics.
- Raw host, listener, and output metrics are collapsed by default.
- Expert tuning is collapsed and disabled until explicitly enabled.
- Resynchronize audio is enabled only when valid.
- Recent operation failures, including tuning persistence failures, remain visible.
- Support reports redact invite codes, session identifiers, device identifiers, and selected-file URIs.
- Reset tuning to defaults is implemented as one complete default `TuningSettings` value persisted through the Rust-owned store before UI state is updated.
- The reset control is enabled only when expert controls are enabled and the current settings differ from defaults.

## Implemented Settings and approved-device management

- Settings shows nearby-device readiness, local app-data readiness, troubleshooting, Advanced Diagnostics, and version/build information.
- Readiness uses text and icons rather than color alone.
- Raw storage errors are not shown on the normal Settings screen.
- Rust/JNI now exposes authoritative trusted-device list and delete operations.
- Kotlin decodes those results through the Android Rust domain store.
- A lifecycle-owned approved-device state holder loads, removes, refreshes, and surfaces failures.
- Settings enables an approved-device management destination backed by the authoritative Rust store.

## Implemented invitation and reusable components

- Added reusable status, problem, empty, loading, role-action, attention, and confirmation components.
- Hosts can copy the invite code and share plain-language Android instructions.
- Invite payloads omit internal session IDs and do not invent unsupported QR behavior.

## Added deterministic source coverage

### Unit and presentation tests

- Domain-to-presentation workflow mappings and unknown-state safety.
- Structured listener failure classifications and actions.
- Host/listener health summaries.
- Workflow effect order, no-duplicate behavior, and lifecycle-clear handling.
- Invite-code generation and validation.
- Contextual permission selection.
- Approval progress and waiting-duration labels.
- Playback and Connection Help mappings.
- Diagnostics/support-report redaction.
- Atomic tuning reset command and UI callback behavior.
- Approved-device state-holder loading and removal behavior.

### Compose and navigation tests

- Startup loading, recoverable, and fatal states.
- Role-first Home and host setup validation.
- Invalid invite-code editing and actionable join failures.
- Host-start permission and transport failures.
- Dashboard approval progress and duplicate-action disabling.
- Discovery, playback, Connection Help, diagnostics, Settings, and confirmation states.
- Production route helpers for single-top navigation, clearing workflows, and recovery back navigation.
- A controllable effect holder driving the real navigation helpers, confirmations, and transient messages.
- Adaptive coverage for Home, Host Music, Host Access, Host Dashboard, Session Join, listener playback, Nearby Sessions, Connection Help, Advanced Diagnostics, Settings, and startup failures across 200% font, small-window, landscape, and tablet cases.

### Compose previews

- Startup, Home, host setup, Host Dashboard tabs/states, discovery, join states, listener playback, Connection Help, Advanced Diagnostics, Settings, and destructive confirmations.
- Explicit preview seams cover selected Host Dashboard tabs and enabled expert tuning without duplicating production state ownership.

## Remaining major work

1. Observe the complete permanent Rust/Android CI matrix for the latest `master` revision and fix any reported compile, lint, unit-test, packaging, or instrumentation-APK failure.
2. Pass approval lifetime directly into `MainViewModel.approveJoinRequest` instead of staging it through mutable host-form state.
3. Complete physical two-device host/listener acceptance testing, including discovery cancellation, approval modes, playback, recovery, and destructive exits.
4. Remove obsolete proof-of-concept screens only after the replacement workflow passes the complete validation matrix.
