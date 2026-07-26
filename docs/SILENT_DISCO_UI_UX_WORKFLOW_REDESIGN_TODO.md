# Silent Disco — UI/UX Workflow Redesign TODO

Baseline reviewed: current `master` after Block 9 persistence correctness and full automated validation.  
Target platform: Android / Kotlin / Jetpack Compose.  
Architecture constraint: keep Rust as the authoritative owner of domain data, persistence, synchronization, protocol, and other platform-independent behavior. Kotlin/Compose owns presentation, Android permissions, navigation, and platform integration.

## Goal

Replace the current developer-oriented seven-screen proof-of-concept flow with a user-centered experience that:

- makes the first decision obvious: **Host music** or **Join music**;
- requests permissions only when the user starts a feature that needs them;
- hides healthy storage and diagnostic details from ordinary users;
- makes host setup shorter and easier;
- makes listener approval semantics explicit: **Approve once**, **Always allow**, or **Reject**;
- keeps playback controls and session health visible while hosting;
- automatically scans for nearby sessions and automatically enters playback when ready;
- presents persistent, actionable errors instead of relying on transient snackbars;
- replaces the default diagnostics experience with user-oriented **Connection Help**;
- preserves full technical metrics and tuning behind an explicit advanced section;
- supports accessibility, large text, destructive-action confirmation, and predictable back navigation;
- never claims success before the underlying Rust/domain/platform operation succeeds.

---

## Priority legend

- **P0**: required workflow correctness, fail-visible behavior, navigation safety, trust correctness, or accessibility needed before calling the redesign complete.
- **P1**: important usability improvement, adaptive layout, settings organization, support tooling, or state polish.
- **P2**: optional convenience, future enhancement, or refinement that may depend on additional persistence/domain support.

---

## Non-negotiable implementation rules

1. Work directly on `master` unless the user explicitly requests a branch or pull request.
2. Do not create temporary branches, observer branches, marker pull requests, or self-modifying validation workflows.
3. Do not change Rust/domain ownership merely to simplify Compose code.
4. Do not add Kotlin-owned duplicate persistence for sessions, trusted devices, synchronization state, diagnostics, or settings already owned by Rust.
5. Do not advertise durable trust until the Rust persistence write has committed successfully.
6. Do not convert a failed durable trust write into silent success. Downgrade visibly to session-only approval when that is the defined behavior.
7. Do not use broad `catch`/`runCatching` blocks that only log and leave the UI in a success or loading state.
8. Do not display internal IDs, packet counts, RTT, jitter, native bridge state, or tuning thresholds in normal consumer workflows.
9. Do not remove advanced diagnostics; move them behind progressive disclosure.
10. Do not use color alone to convey connection, synchronization, approval, or error state.
11. Do not rely on snackbars for storage failure, join rejection, session termination, connection loss, or playback failure.
12. Every loading operation must end in a success, empty, retryable failure, fatal failure, or explicit cancelled state.
13. Every destructive action must have a confirmation path when it can interrupt an active session.
14. All new user-visible strings must be written in plain language and be suitable for future extraction into Android string resources.
15. All referenced implementation files in this TODO either already exist in the repository or are explicitly marked **new file**.

---

## Target information architecture

The redesigned app should expose these user-facing destinations:

1. `startup`
2. `home`
3. `host_music_setup`
4. `host_access_setup`
5. `host_dashboard`
6. `nearby_sessions`
7. `session_join`
8. `listener_playback`
9. `connection_help`
10. `advanced_diagnostics`
11. `settings`

Recommended high-level flows:

```text
Startup check
  -> Home

Home
  -> Host music
       -> Choose music
       -> Choose access
       -> Hosting dashboard
       -> End-session confirmation
       -> Home

Home
  -> Join music
       -> Nearby sessions
       -> Session details / join status
       -> Now playing
       -> Leave-session confirmation
       -> Home

Host dashboard or Now playing
  -> Connection help
       -> Advanced diagnostics

Home
  -> Settings
```

The exact route class or string representation may differ, but the navigation graph and tests must preserve these semantics.

---

# P0 — Foundation and workflow state

## P0.1 Record the current UI behavior before changing it

**Existing files to inspect:**

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/home/HomeScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostSetupScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostControlScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/JoinProgressScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/ListenerPlaybackScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

Tasks:

- [ ] Enumerate every current route and every action that navigates between routes.
- [ ] Enumerate every current state that changes button enablement or visible content.
- [ ] Record which operations are synchronous Boolean-returning calls versus asynchronous state transitions.
- [ ] Record all locations where `lastError` and `lastMessage` are surfaced only through the global snackbar.
- [ ] Record all locations where Android back navigation can leave an active host or listener session.
- [ ] Record all places where current UI wording exposes internal implementation language.
- [ ] Add or update unit tests that capture critical pre-redesign behavior before refactoring.

Acceptance:

- [ ] No production behavior is removed accidentally because a route, state, or error path was overlooked.
- [ ] The implementation can be performed in small compilable slices.

## P0.2 Introduce explicit presentation-level workflow models

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/test/java/com/ekkus/silentdisco/app/`

Add small presentation models instead of deriving all UX decisions ad hoc inside composables.

Recommended models:

```kotlin
enum class HostSetupStep {
    MUSIC,
    ACCESS,
}

enum class JoinUiStep {
    FINDING_HOST,
    REQUESTING_ACCESS,
    WAITING_FOR_APPROVAL,
    CONNECTING,
    SYNCING_AUDIO,
    COMPLETE,
}

enum class SessionHealthLevel {
    GOOD,
    ATTENTION,
    CRITICAL,
    UNKNOWN,
}

data class SessionHealthSummary(
    val level: SessionHealthLevel,
    val title: String,
    val detail: String,
    val affectedListenerCount: Int = 0,
)
```

Tasks:

- [ ] Add `HostSetupStep` or an equivalent type.
- [ ] Add a user-facing join-step mapping that collapses the current technical stages into five understandable stages.
- [ ] Add a derived host session health summary.
- [ ] Add a derived listener connection health summary.
- [ ] Add an explicit model for persistent user-facing problems; do not infer all problems from a single nullable string.
- [ ] Keep raw diagnostic/domain state available for Advanced Diagnostics.
- [ ] Normalize presentation state in one place rather than duplicating conditions across multiple screens.
- [ ] Add unit tests for every mapping from domain lifecycle state to user-facing workflow state.

Acceptance:

- [ ] Every domain lifecycle value maps deterministically to a user-facing state.
- [ ] Unknown/new enum values cannot silently appear as a healthy state.
- [ ] Composables receive presentation-ready values where practical.

## P0.3 Add one-shot UI effects for navigation and confirmations

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppUiEffect.kt`

**Existing files:**

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

Recommended shape:

```kotlin
sealed interface AppUiEffect {
    data object NavigateHome : AppUiEffect
    data object NavigateHostDashboard : AppUiEffect
    data object NavigateListenerPlayback : AppUiEffect
    data object ShowEndSessionConfirmation : AppUiEffect
    data object ShowLeaveSessionConfirmation : AppUiEffect
    data class ShowTransientMessage(val message: String) : AppUiEffect
}
```

Tasks:

- [ ] Add a `SharedFlow`, `Channel`, or equivalent one-shot effect stream.
- [ ] Use an effect for automatic transition from completed listener synchronization to playback.
- [ ] Use an effect for completed host creation.
- [ ] Use an effect for returning home after a confirmed end/leave action.
- [ ] Keep persistent failures in state, not effects.
- [ ] Ensure configuration changes and recomposition do not repeat navigation.
- [ ] Ensure a consumed effect cannot trigger twice.
- [ ] Add unit tests for effect emission order and no-duplicate behavior.

Acceptance:

- [ ] Playback navigation happens exactly once when the listener becomes ready.
- [ ] No route is pushed repeatedly during recomposition.
- [ ] Fatal/retryable problems survive configuration changes.

---

# P0 — Startup and permissions

## P0.4 Add a dedicated startup gate

**New file:**

- `app/src/main/java/com/ekkus/silentdisco/feature/startup/StartupGateScreen.kt`

**Existing files:**

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/StorageInitializationPolicy.kt`

Tasks:

- [ ] Make startup the navigation graph start destination.
- [ ] While storage is initializing, show a focused progress state such as:
  - title: `Getting Silent Disco ready…`
  - detail: `Opening local app data`
- [ ] When storage becomes ready, navigate automatically to Home exactly once.
- [ ] For recoverable storage failure, show:
  - a plain-language explanation;
  - a **Retry** primary action;
  - optional expandable technical details.
- [ ] For fatal storage failure, show:
  - a blocking title;
  - a plain-language explanation that local app data could not be opened;
  - a **Share support report** action if diagnostics are available;
  - no fake continue path.
- [ ] Preserve the current fatal/recoverable classification.
- [ ] Prevent Host and Join navigation before startup completes.
- [ ] Add test tags for loading, recoverable, fatal, retry, and ready states.
- [ ] Add Compose tests for all startup states.

Acceptance:

- [ ] Healthy storage status no longer occupies permanent Home-screen space.
- [ ] Storage failure remains fail-visible and cannot be dismissed into a broken Home screen.
- [ ] Retry is available only for recoverable failures.

## P0.5 Implement contextual permission requests

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/core/permissions/Permissions.kt`
- `app/src/test/java/com/ekkus/silentdisco/core/permissions/PermissionCatalogueTest.kt`

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/app/PermissionRequestContext.kt`

Tasks:

- [ ] Replace the Home-screen all-permissions request with capability-specific requests.
- [ ] Define permission contexts at minimum for:
  - Host nearby-device discovery/advertising;
  - Listener nearby-session discovery;
  - Audio-file selection.
- [ ] Reuse existing `wifiDirectPermissions()` and `bluetoothPermissions()` helpers where correct.
- [ ] Add a dedicated helper for media/audio read permission if needed.
- [ ] Before launching Android permission UI, show a rationale sheet explaining why the requested capability is needed.
- [ ] Do not request audio-file permission until the user chooses audio.
- [ ] Do not request listener-irrelevant permissions merely because they exist in the catalogue.
- [ ] Handle partial denial explicitly.
- [ ] Handle permanent denial with a Settings deep-link action.
- [ ] Preserve SDK-version-specific permission behavior.
- [ ] Add tests for API 29, API 31, API 33, and current compile/target behavior.

Acceptance:

- [ ] Host and listener users see only permissions relevant to their chosen action.
- [ ] Denial produces a persistent explanation and next action.
- [ ] The app never reports that scanning/hosting started when required permissions are missing.

---

# P0 — Navigation redesign

## P0.6 Replace the route set and centralize route definitions

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppRoutes.kt`

Tasks:

- [ ] Move route definitions out of the private object in `SilentDiscoApp.kt`.
- [ ] Add the target destinations listed in the information architecture section.
- [ ] Remove obsolete destinations only after replacement screens are wired and tested.
- [ ] Preserve a single source of truth for route names.
- [ ] Add helper navigation functions for clearing the stack when returning Home.
- [ ] Add helper navigation functions that avoid duplicate top destinations.
- [ ] Add navigation tests for host, listener, settings, connection-help, and advanced-diagnostics paths.

Acceptance:

- [ ] There is no duplicated route string in production code.
- [ ] The back stack is deterministic for every primary workflow.

## P0.7 Add active-session back handling and destructive confirmations

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostDashboardScreen.kt` (**new or renamed file**)
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/ListenerPlaybackScreen.kt`

**New reusable component recommended:**

- `app/src/main/java/com/ekkus/silentdisco/ui/components/ConfirmationSheet.kt`

Tasks:

- [ ] Intercept system back while hosting an active session.
- [ ] Intercept system back while listener playback is active.
- [ ] Show an end-session confirmation with listener count.
- [ ] Show a leave-session confirmation.
- [ ] Make the safe action the default focus:
  - `Keep hosting` before `End session`;
  - `Stay` before `Leave session`.
- [ ] Use destructive styling only for the destructive action.
- [ ] Ensure cancellation leaves state untouched.
- [ ] Ensure confirmation cannot issue duplicate end/leave operations.
- [ ] Test Android back, top-app-bar back, explicit end/leave buttons, and configuration changes while the sheet is open.

Acceptance:

- [ ] Back navigation cannot silently terminate or abandon an active session.
- [ ] Confirmed end/leave returns to a clean Home back stack.

---

# P0 — Home redesign

## P0.8 Replace system-status cards with role-first actions

**File:**

- `app/src/main/java/com/ekkus/silentdisco/feature/home/HomeScreen.kt`

**New reusable components recommended:**

- `app/src/main/java/com/ekkus/silentdisco/ui/components/RoleActionCard.kt`
- `app/src/main/java/com/ekkus/silentdisco/ui/components/AttentionBanner.kt`

Tasks:

- [ ] Remove the healthy persistent-storage card from Home.
- [ ] Remove the generic permissions card from Home.
- [ ] Add a top app bar with app title and Settings action.
- [ ] Add a large **Host music** role card:
  - supporting text: `Play music for nearby listeners.`
  - action: `Start a session`.
- [ ] Add a large **Join music** role card:
  - supporting text: `Listen to a nearby host in sync.`
  - action: `Find a session`.
- [ ] Show attention banners only when action is required.
- [ ] Do not show internal storage terminology in normal ready state.
- [ ] Add optional placeholders for recent/rejoin content without implementing fake data.
- [ ] Ensure cards remain usable with large font sizes.
- [ ] Add Compose tests for ready, attention, and disabled/blocking states.

Acceptance:

- [ ] A first-time user can immediately identify Host versus Join.
- [ ] Healthy technical status does not compete with primary actions.
- [ ] Any blocking problem remains visible and actionable.

---

# P0 — Host setup redesign

## P0.9 Split host setup into Music and Access steps

**Replace or refactor:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostSetupScreen.kt`

**New files recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostMusicSetupScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostAccessSetupScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostSetupSummary.kt`

Tasks for Music step:

- [ ] Make audio selection the most prominent control.
- [ ] Show selected file name and available metadata.
- [ ] Keep session name editable.
- [ ] Generate a useful default session name from a stable user/device-facing label when available.
- [ ] Do not expose a raw device identifier as the default name.
- [ ] Make **Next: Choose access** the single primary action.
- [ ] Disable Next only when required fields are missing.
- [ ] Explain missing fields inline.
- [ ] Request media permission only when Choose Audio is activated.

Tasks for Access step:

- [ ] Replace raw enum labels with user-oriented choices:
  - `Ask me before anyone joins`;
  - `Require an invite code`;
  - `Approved devices only`.
- [ ] Add a one-sentence explanation under each option.
- [ ] Show the invite-code field only when required.
- [ ] Add **Generate code**.
- [ ] Generate codes with unambiguous characters or numeric digits.
- [ ] Preserve manual editing of the code.
- [ ] Remove the global `Remember approved devices` checkbox from this flow.
- [ ] Show a compact summary of music, session name, and access mode before Start.
- [ ] Make **Start session** the single primary action.
- [ ] Keep a Back action that returns to Music without losing inputs.

State/model cleanup:

- [ ] Remove `rememberApprovedDevices` from `HostFormState` after all call sites and tests are migrated, unless it is still required by a persisted compatibility boundary.
- [ ] If the field must temporarily remain for a persistence contract, stop presenting it in UI and document the deprecation.
- [ ] Do not change the Rust schema solely for this presentation cleanup unless the field is actually persisted there and requires a migration.

Acceptance:

- [ ] A normal host can start a session without typing a custom name.
- [ ] The host never sees irrelevant invite-code input.
- [ ] The setup flow never asks for a global trust policy.
- [ ] Inputs survive moving forward and backward between setup steps.

## P0.10 Generate and validate invite codes

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- tests under `app/src/test/java/com/ekkus/silentdisco/app/`

Tasks:

- [ ] Add a deterministic, testable invite-code generator dependency or helper.
- [ ] Prefer a short code suitable for reading aloud.
- [ ] Define minimum/maximum length.
- [ ] Normalize whitespace.
- [ ] Reject blank or invalid codes before session creation.
- [ ] Do not silently substitute a generated code when the user entered an invalid one; explain the validation error.
- [ ] Add tests for generation, editing, validation, and access-mode switching.

Acceptance:

- [ ] Invite-code mode can always produce a valid code without user invention.
- [ ] The host sees the exact code listeners must enter.

---

# P0 — Listener approval and trust UX

## P0.11 Replace Approve/Trust ambiguity with explicit approval actions

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/JoinApprovalExecution.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostControlScreen.kt` or replacement dashboard
- tests under `app/src/test/java/com/ekkus/silentdisco/app/`

Recommended action model:

```kotlin
enum class JoinApprovalAction {
    APPROVE_ONCE,
    ALWAYS_ALLOW,
    REJECT,
}
```

Tasks:

- [ ] Replace generic `Approve` with **Approve once**.
- [ ] Add **Always allow** for durable trust.
- [ ] Keep **Reject**.
- [ ] Route `APPROVE_ONCE` through session-only approval.
- [ ] Route `ALWAYS_ALLOW` through the existing persist-before-advertise durable trust sequence.
- [ ] If durable persistence fails, downgrade visibly to session-only only when that is the intended policy.
- [ ] Tell the host that the listener was approved for this session but could not be remembered.
- [ ] Never send `trustedForFuture = true` before persistence commits.
- [ ] Ensure the approved listener model reflects the committed trust state.
- [ ] Prevent duplicate taps while an approval operation is in progress.
- [ ] Show per-request progress when approval is being written/sent.
- [ ] Add tests for ordering, downgrade, rejection, duplicate taps, delivery failure, and resulting UI state.

Acceptance:

- [ ] The host can understand the lifetime of every approval before tapping it.
- [ ] Durable trust semantics remain correct under failure.

---

# P0 — Hosting dashboard

## P0.12 Replace Host Control with a task-focused Hosting Dashboard

**Replace or rename:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostControlScreen.kt`

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostDashboardScreen.kt`

**New components recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/HostPlaybackControls.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/ListenerRequestCard.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/ConnectedListenerCard.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/host/SessionHealthCard.kt`

Tasks:

- [ ] Show session name and streaming state prominently.
- [ ] Show connected-listener count in plain language.
- [ ] Add an **Invite** action placeholder that exposes the current access information without inventing unsupported QR behavior.
- [ ] Keep primary playback controls near the top or in a persistent bottom section.
- [ ] Show selected audio and available playback progress.
- [ ] Do not show a disabled Start button while already playing if a Play/Pause toggle is clearer.
- [ ] Group listener content into tabs or clearly separated sections:
  - Requests;
  - Connected;
  - Needs attention.
- [ ] Show a count badge for pending Requests.
- [ ] Show waiting duration for requests when timing data is available.
- [ ] Use **Approve once**, **Always allow**, and **Reject** actions.
- [ ] Show Connected listeners with status text/chips:
  - connection;
  - synchronization;
  - trust state when relevant.
- [ ] Move secondary listener operations to an overflow menu.
- [ ] Do not show raw request IDs.
- [ ] Add a plain-language session-health summary.
- [ ] Route health problems to Connection Help.
- [ ] Move End Session to overflow or a clearly separated destructive area.
- [ ] Remove `[Debug] Add Demo Join` from release UI and preserve it only under `BuildConfig.DEBUG`.
- [ ] Add empty states for Requests, Connected, and Needs attention.
- [ ] Add loading/progress states for start, pause, stop, approval, remove, trust, and end operations.

Acceptance:

- [ ] Playback controls remain easy to reach with many listeners.
- [ ] The host can distinguish pending, connected, and troubled listeners without reading raw metrics.
- [ ] Internal IDs are absent from normal UI.

## P0.13 Derive host session health honestly

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- tests under `app/src/test/java/com/ekkus/silentdisco/app/`

Tasks:

- [ ] Define exact thresholds/conditions for Good, Attention, Critical, and Unknown.
- [ ] Base the summary on authoritative diagnostics and lifecycle state.
- [ ] Never show Good when host state or playback state is ERROR.
- [ ] Treat unknown/missing diagnostics as Unknown, not Good.
- [ ] Include affected listener count.
- [ ] Provide a short title and actionable detail.
- [ ] Add exhaustive unit tests for combinations of connected, reconnecting, desynced, failed, and unknown listeners.

Acceptance:

- [ ] The health card cannot contradict the underlying diagnostics.

---

# P0 — Nearby-session discovery

## P0.14 Start discovery automatically and expose complete scan states

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

Tasks:

- [ ] Start scanning when the Nearby Sessions destination becomes active and required permissions are available.
- [ ] Prevent duplicate simultaneous scans caused by recomposition.
- [ ] Keep a smaller Refresh action for manual retry.
- [ ] Represent states explicitly:
  - permission required;
  - scanning;
  - results;
  - no sessions found;
  - retryable scan failure;
  - fatal scan failure if applicable.
- [ ] Keep previous results visible during a refresh when safe.
- [ ] Indicate that results may change as hosts appear/disappear.
- [ ] Stop or release scan resources when leaving the destination if required by the transport implementation.
- [ ] Add tests for entry-triggered scan, refresh, cancellation, duplicate suppression, and errors.

Acceptance:

- [ ] A listener does not have to press Scan on every entry.
- [ ] The UI never remains indefinitely in `Scanning…` after failure or cancellation.

## P0.15 Redesign session cards and session selection

**File:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/DiscoverSessionsScreen.kt`

**New component recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/NearbySessionCard.kt`

Tasks:

- [ ] Show session name and host display name.
- [ ] Translate approval modes into user-facing badges:
  - Approval required;
  - Invite code required;
  - You are already approved, only when actually known.
- [ ] Do not display implementation enum names.
- [ ] Make the whole card selectable with accessible semantics.
- [ ] Keep a visible Join action if it improves clarity.
- [ ] Disable selection only when a conflicting join operation is truly active.
- [ ] Explain disabled state inline.
- [ ] Sort stable results predictably; do not reorder constantly due to incidental updates.
- [ ] Preserve selection identity by stable session ID.
- [ ] Add Compose tests for every access badge and disabled state.

Acceptance:

- [ ] Users can understand access requirements before selecting a session.

---

# P0 — Session join and progress

## P0.16 Merge session details and join progress into one coherent destination

**Replace or refactor:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/JoinProgressScreen.kt`

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/SessionJoinScreen.kt`

Tasks:

- [ ] Show session name, host name, and access requirement before request.
- [ ] Show invite-code input only when required.
- [ ] Use **Request to join** as the primary action.
- [ ] After request, replace the detail action area with join progress without pushing another duplicate route.
- [ ] Collapse the current technical stages into:
  1. Finding the host;
  2. Requesting access;
  3. Waiting for host approval;
  4. Connecting;
  5. Getting audio in sync.
- [ ] Keep raw technical stages available in Advanced Diagnostics.
- [ ] Show elapsed waiting time for host approval when timing data is available.
- [ ] Show a plain-language detail under the active step.
- [ ] Allow Cancel until playback becomes active.
- [ ] Keep Retry only for states where retry is valid.
- [ ] Do not allow repeated join requests while one request is in flight.
- [ ] Add explicit rejected, cancelled, disconnected, and error states.

Acceptance:

- [ ] The listener sees one continuous story from session selection to playback.
- [ ] No extra **Continue to Playback** action is required.

## P0.17 Automatically navigate to playback when ready

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- UI effect file added in P0.3

Tasks:

- [ ] Emit `NavigateListenerPlayback` only after the listener is truly in a playable state.
- [ ] Do not navigate on merely approved, connected, syncing, or buffering states.
- [ ] Ensure the effect is emitted once.
- [ ] If playback fails before navigation completes, keep the user on Session Join with a persistent problem state.
- [ ] Remove the **Continue to Playback** button and obsolete callback.
- [ ] Add tests for normal transition, repeated state emissions, cancellation, and failure race conditions.

Acceptance:

- [ ] Playback opens automatically exactly once after readiness.

## P0.18 Add persistent join failure presentations

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/listener/SessionJoinScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Tasks:

- [ ] Add structured failure reasons at minimum for:
  - host rejected request;
  - invalid invite code;
  - host ended session;
  - host out of range/unreachable;
  - transport connection failure;
  - synchronization failure;
  - playback initialization failure;
  - permission failure.
- [ ] Map internal errors to safe user-facing copy.
- [ ] Preserve detailed diagnostics separately.
- [ ] Provide context-appropriate actions:
  - Retry;
  - Edit code;
  - Return to sessions;
  - Open Settings;
  - Share support report.
- [ ] Clear the failure only when the user resolves, retries, cancels, or leaves the flow.
- [ ] Do not clear it merely because a snackbar was shown.

Acceptance:

- [ ] Every major join failure remains visible until the user chooses a next action.

---

# P0 — Listener playback redesign

## P0.19 Simplify Now Playing around listening and connection health

**File:**

- `app/src/main/java/com/ekkus/silentdisco/feature/listener/ListenerPlaybackScreen.kt`

Tasks:

- [ ] Show session name and host name prominently.
- [ ] Show `The host controls playback`.
- [ ] Show audio title when known.
- [ ] Show one prominent playback/sync status:
  - Playing in sync;
  - Buffering;
  - Reconnecting;
  - Audio is out of sync;
  - Playback stopped;
  - Connection lost.
- [ ] Preserve the local volume slider.
- [ ] Hide or disable reconnect when the connection is healthy.
- [ ] Show **Fix connection** only when a user action is useful.
- [ ] Route Fix connection to Connection Help.
- [ ] Keep Leave Session visible but confirm before leaving.
- [ ] Remove normal-screen raw connection and sync metric wording.
- [ ] Ensure buffering and reconnecting progress are accessible.
- [ ] Add Compose tests for every health/playback state.

Acceptance:

- [ ] A healthy listener screen contains no unnecessary recovery button.
- [ ] A troubled listener sees a clear explanation and next action.

## P0.20 Ensure playback and connection errors are stateful

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`

Tasks:

- [ ] Audit all listener playback engine failure handlers.
- [ ] Audit transport disconnect and reconnect handlers.
- [ ] Audit desynchronization and hard-resync handlers.
- [ ] Ensure each failure updates lifecycle, playback, diagnostics, and user-facing problem state consistently.
- [ ] Ensure recovery clears only the relevant problem after success.
- [ ] Do not set Playing until audio output actually starts.
- [ ] Do not show Stable while reconnecting or desynchronized.
- [ ] Add transition tests for buffering -> playing, playing -> reconnecting, reconnecting -> playing, playing -> error, and error -> retry.

Acceptance:

- [ ] The Now Playing status cannot claim healthy playback after a failed underlying operation.

---

# P0 — Connection Help and Advanced Diagnostics

## P0.21 Add user-oriented Connection Help

**New file:**

- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/ConnectionHelpScreen.kt`

**Existing files:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Tasks:

- [ ] Show a top-level result:
  - Everything looks good;
  - Connection is recovering;
  - Audio is out of sync;
  - Connection lost;
  - Playback problem;
  - Status unknown.
- [ ] Show understandable indicators:
  - Connection;
  - Synchronization;
  - Audio buffer/playback.
- [ ] Show recommended action based on state.
- [ ] Rename Manual Resync to **Resynchronize audio**.
- [ ] Enable Resynchronize only when valid.
- [ ] Show Reconnect only when valid.
- [ ] Add **Share support report**.
- [ ] Add **Advanced diagnostics** navigation.
- [ ] Do not show raw metrics by default.
- [ ] Support host and listener contexts without displaying irrelevant empty sections.
- [ ] Add tests for healthy, degraded, disconnected, desynced, playback-error, and unknown states.

Acceptance:

- [ ] Ordinary users can act without interpreting RTT, jitter, or packet loss.

## P0.22 Move the current diagnostics content into Advanced Diagnostics

**Refactor:**

- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/DiagnosticsScreen.kt`

**New or renamed file:**

- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/AdvancedDiagnosticsScreen.kt`

Tasks:

- [ ] Preserve all current host diagnostic fields.
- [ ] Preserve all current listener diagnostic fields.
- [ ] Preserve playback output and native bridge status.
- [ ] Preserve support-report generation.
- [ ] Hide irrelevant host/listener sections based on role/context when appropriate.
- [ ] Add copy/share affordances for diagnostic sections if useful.
- [ ] Keep internal IDs here, not in normal workflows.
- [ ] Add a clear `Advanced diagnostics` title and technical-audience description.
- [ ] Ensure long values wrap and remain selectable if appropriate.
- [ ] Add Compose tests that verify all critical diagnostic values remain represented.

Acceptance:

- [ ] No diagnostic capability is lost during simplification.

## P0.23 Gate expert tuning controls

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/AdvancedDiagnosticsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`

Tasks:

- [ ] Put tuning controls under a collapsed **Expert tuning** section.
- [ ] Add warning text:
  - `Changing these values can make synchronization worse.`
- [ ] Require explicit enabling before controls are interactive.
- [ ] Keep current bounds and cross-field normalization.
- [ ] Show current values and reset-to-default action.
- [ ] Require confirmation before reset if user changes would be lost.
- [ ] Make persistence failures visible.
- [ ] Do not claim a tuning change succeeded before Rust persistence succeeds.
- [ ] Add tests for disabled-by-default, enable, adjust, bounds, persistence failure, and reset.

Acceptance:

- [ ] Ordinary users cannot accidentally alter tuning from Connection Help.

---

# P0 — Error and message architecture

## P0.24 Replace the single snackbar-first error pattern

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/app/AppState.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/MainViewModel.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`
- affected feature screens

Tasks:

- [ ] Classify messages as transient confirmation versus persistent problem.
- [ ] Keep snackbar/effect behavior only for brief confirmations such as:
  - Device approved;
  - Support report copied/shared;
  - Tuning restored.
- [ ] Add persistent problem presentation for:
  - storage initialization failure;
  - permission denial blocking a feature;
  - session creation failure;
  - join rejection;
  - host ended session;
  - transport loss;
  - synchronization failure;
  - playback failure;
  - persistence failure.
- [ ] Provide a stable problem ID/type so the same issue is not repeatedly announced.
- [ ] Add explicit dismiss behavior only where dismissal is safe.
- [ ] Keep technical cause and user-facing text separate.
- [ ] Add tests for persistence across recomposition/configuration and clearing on resolution.

Acceptance:

- [ ] No critical error disappears merely because a snackbar timeout elapsed.

---

# P0 — Accessibility and interaction quality

## P0.25 Apply accessibility requirements to every redesigned screen

**Files:**

- all new/refactored Compose screens and components
- Compose UI tests under `app/src/androidTest/` or `app/src/test/` as appropriate

Tasks:

- [ ] Use minimum 48 dp interactive targets.
- [ ] Add meaningful content descriptions to icon-only controls.
- [ ] Combine related text and controls into coherent semantics groups.
- [ ] Ensure role cards, session cards, listener cards, and status chips have clear screen-reader labels.
- [ ] Never rely on color alone.
- [ ] Pair status colors with text and/or icons.
- [ ] Verify disabled controls expose why they are disabled through nearby text.
- [ ] Support large font scale without clipped primary actions.
- [ ] Support screen widths down to the project minimum device size.
- [ ] Verify logical TalkBack focus order.
- [ ] Announce important state changes such as approval, connection, and playback readiness without repeated spam.
- [ ] Ensure loading indicators have accessible descriptions.
- [ ] Ensure destructive confirmations receive focus correctly.
- [ ] Add accessibility assertions where Compose testing APIs permit.

Acceptance:

- [ ] Primary workflows remain usable with TalkBack and 200% font scale.

---

# P1 — Settings and organization

## P1.1 Add a Settings destination

**New file:**

- `app/src/main/java/com/ekkus/silentdisco/feature/settings/SettingsScreen.kt`

Tasks:

- [ ] Add Settings navigation from Home.
- [ ] Show permission status and an Open System Settings action.
- [ ] Show local app-data status using user-friendly wording.
- [ ] Show app version/build information.
- [ ] Show trusted-device management entry point only if supported by current domain APIs.
- [ ] Show Advanced Diagnostics entry point.
- [ ] Do not put active-session controls in Settings.
- [ ] Add tests for ready/error storage state and permission state.

Acceptance:

- [ ] Technical readiness information remains available without cluttering Home.

## P1.2 Add trusted-device management if domain APIs support it

**Potential new file:**

- `app/src/main/java/com/ekkus/silentdisco/feature/settings/TrustedDevicesScreen.kt`

Tasks:

- [ ] Inspect Rust repository/JNI APIs for listing and deleting trusted devices.
- [ ] Do not implement a fake list from Kotlin memory.
- [ ] If APIs are missing, document the required Rust/JNI work and leave this task unchecked.
- [ ] Show device display name and last-known metadata only when authoritative data exists.
- [ ] Add Forget confirmation.
- [ ] Make deletion fail-visible.
- [ ] Refresh the list only after committed deletion.
- [ ] Add Rust, JNI, ViewModel, and Compose tests if new APIs are required.

Acceptance:

- [ ] Trusted-device management uses Rust-owned persistence end to end.

---

# P1 — Reusable visual system

## P1.3 Create reusable status and action components

**New directory recommended:**

- `app/src/main/java/com/ekkus/silentdisco/ui/components/`

Potential files:

- `StatusBadge.kt`
- `PrimaryProblemCard.kt`
- `EmptyState.kt`
- `LoadingState.kt`
- `RoleActionCard.kt`
- `ConfirmationSheet.kt`
- `SectionHeader.kt`

Tasks:

- [ ] Create a consistent status-badge API with icon, text, and semantic label.
- [ ] Create one persistent problem-card pattern.
- [ ] Create one empty-state pattern.
- [ ] Create one confirmation component for destructive actions.
- [ ] Use Material theme tokens rather than hard-coded colors in feature screens.
- [ ] Keep feature-specific wording outside generic components.
- [ ] Add previews for light-independent dark theme usage and large text.
- [ ] Add component-level Compose tests where interaction exists.

Acceptance:

- [ ] The redesign does not duplicate button, badge, empty-state, and problem-card implementations across screens.

## P1.4 Improve responsive behavior

Tasks:

- [ ] Test phone portrait as the primary layout.
- [ ] Support landscape without losing primary actions.
- [ ] Consider two-pane layout for tablets/foldables:
  - session list + detail;
  - host status + listener tabs;
  - Connection Help + advanced details.
- [ ] Do not block phone completion on tablet-specific polish.
- [ ] Use adaptive width constraints so cards do not become excessively wide.
- [ ] Add screenshot/previews at representative compact and expanded widths.

Acceptance:

- [ ] No primary screen becomes unusable in landscape.

---

# P1 — Support reports and invitations

## P1.5 Rename and harden support-report sharing

**Files:**

- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/ConnectionHelpScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/feature/diagnostics/AdvancedDiagnosticsScreen.kt`
- `app/src/main/java/com/ekkus/silentdisco/app/SilentDiscoApp.kt`

Tasks:

- [ ] Rename `Share Debug Info` to `Share support report`.
- [ ] Include user-friendly summary first.
- [ ] Include technical metrics in a clearly delimited advanced section.
- [ ] Review the report for secrets, invite codes, personal file paths, and unnecessary identifiers.
- [ ] Do not include the active invite code by default.
- [ ] Include app version and timestamp if available.
- [ ] Keep Android share-sheet integration.
- [ ] Add deterministic report-generation tests.

Acceptance:

- [ ] The report is useful for support without exposing avoidable sensitive data.

## P1.6 Add an Invite sheet using existing session data

**New file recommended:**

- `app/src/main/java/com/ekkus/silentdisco/feature/host/InviteSessionSheet.kt`

Tasks:

- [ ] Show session name.
- [ ] Show invite code only when the mode uses one.
- [ ] Add Copy Code.
- [ ] Add Android share action for plain-language instructions.
- [ ] Do not invent a QR-code protocol until a stable, validated payload is defined.
- [ ] Do not expose internal session IDs unless required by the real discovery protocol.
- [ ] Add tests for manual, invite-code, and approved-device modes.

Acceptance:

- [ ] Hosts can communicate access instructions without navigating away from the dashboard.

---

# P2 — Convenience and future enhancements

## P2.1 Add recent-session and rejoin shortcuts only with authoritative persistence

Tasks:

- [ ] Inspect Rust session-history repository support.
- [ ] Define what makes a session safe to show as recent.
- [ ] Do not imply that a previous host is currently available.
- [ ] Show **Rejoin last session** only when discovery confirms availability or the UI clearly states that availability will be checked.
- [ ] Add Rust/JNI/ViewModel support if missing.
- [ ] Add expiration/cleanup behavior.
- [ ] Add tests for stale, missing, and available sessions.

Acceptance:

- [ ] No stale session shortcut promises a connection that has not been rediscovered.

## P2.2 Add trusted-host grouping when identity semantics are complete

Tasks:

- [ ] Confirm the distinction between a trusted listener device and a trusted host identity.
- [ ] Do not reuse listener trust records for host trust unless the domain model explicitly permits it.
- [ ] Add a separate authoritative model if needed.
- [ ] Group `Trusted hosts` only when the identity can be verified.
- [ ] Add spoofing and stale-identity tests.

Acceptance:

- [ ] Trust labels cannot be produced from display-name matching.

## P2.3 Add QR-based joining only after protocol design

Tasks:

- [ ] Define a versioned QR payload.
- [ ] Define what data is public versus secret.
- [ ] Define expiration and replay behavior.
- [ ] Validate payload through Rust/domain code.
- [ ] Add scanner permission and failure UX.
- [ ] Add unit, parser, fuzz/property, and instrumentation tests.

Acceptance:

- [ ] QR joining is not implemented as an unversioned ad hoc string.

---

# Testing requirements

## P0.26 Add unit tests for presentation logic

**Location:**

- `app/src/test/java/com/ekkus/silentdisco/app/`

Required test groups:

- [ ] startup-state mapping;
- [ ] contextual permission selection;
- [ ] host setup step transitions;
- [ ] access-mode labels and validation;
- [ ] invite-code generation and validation;
- [ ] approve-once versus always-allow behavior;
- [ ] durable trust ordering and downgrade behavior;
- [ ] host session health derivation;
- [ ] listener connection health derivation;
- [ ] domain lifecycle -> user join-step mapping;
- [ ] navigation effect one-shot behavior;
- [ ] persistent problem creation and clearing;
- [ ] active-session confirmation state;
- [ ] support-report redaction;
- [ ] tuning gating and reset.

Rules:

- [ ] Test production helpers, not duplicated test-only logic.
- [ ] Use fake dependencies for time, invite-code generation, and permission state where needed.
- [ ] Avoid tests that merely assert copied enum constants.

## P0.27 Add Compose UI tests for every primary screen

**Locations:**

- `app/src/androidTest/java/com/ekkus/silentdisco/`
- or JVM Compose test location if the project supports it reliably

Required screens/states:

- [ ] Startup loading.
- [ ] Startup recoverable failure.
- [ ] Startup fatal failure.
- [ ] Home ready.
- [ ] Home attention banner.
- [ ] Host Music empty.
- [ ] Host Music selected audio.
- [ ] Host Access manual.
- [ ] Host Access invite code.
- [ ] Host Dashboard no listeners.
- [ ] Host Dashboard pending request.
- [ ] Host Dashboard connected listeners.
- [ ] Host Dashboard needs attention.
- [ ] Nearby Sessions scanning.
- [ ] Nearby Sessions empty.
- [ ] Nearby Sessions results.
- [ ] Session Join before request.
- [ ] Session Join waiting approval.
- [ ] Session Join rejected.
- [ ] Session Join connection failure.
- [ ] Now Playing healthy.
- [ ] Now Playing buffering.
- [ ] Now Playing reconnecting.
- [ ] Now Playing desynced.
- [ ] Connection Help healthy.
- [ ] Connection Help actionable problem.
- [ ] Advanced Diagnostics.
- [ ] Expert tuning disabled/enabled.
- [ ] Settings.
- [ ] End-session confirmation.
- [ ] Leave-session confirmation.

Test requirements:

- [ ] Use stable test tags for primary containers and actions.
- [ ] Assert visible copy, enablement, and callbacks.
- [ ] Assert technical values do not appear on consumer screens.
- [ ] Assert critical failures remain visible.
- [ ] Assert large-font layouts do not hide primary actions where supported.

## P0.28 Add navigation workflow tests

Required workflows:

- [ ] startup ready -> Home;
- [ ] Home -> Host Music -> Host Access -> Dashboard;
- [ ] Host Access back -> Host Music with state preserved;
- [ ] Dashboard back -> confirmation -> cancel;
- [ ] Dashboard end -> confirmation -> Home with cleared stack;
- [ ] Home -> Nearby Sessions with contextual permission path;
- [ ] session selection -> Session Join;
- [ ] playback readiness -> automatic Now Playing navigation exactly once;
- [ ] Now Playing back -> confirmation -> cancel;
- [ ] leave -> Home with cleared stack;
- [ ] Host Dashboard -> Connection Help -> Advanced Diagnostics -> back;
- [ ] Now Playing -> Connection Help -> Advanced Diagnostics -> back;
- [ ] Home -> Settings -> back.

## P0.29 Preserve end-to-end instrumentation coverage

Tasks:

- [ ] Keep the existing Rust-domain-store instrumentation tests passing.
- [ ] Keep migration-checksum fatal-failure UI-state coverage.
- [ ] Add emulator instrumentation coverage for the redesigned startup fatal state.
- [ ] Add at least one host setup/dashboard workflow test.
- [ ] Add at least one listener discovery/join/playback workflow test using deterministic fakes or existing test seams.
- [ ] Do not claim real multi-device Wi-Fi/Bluetooth validation from single-emulator tests.
- [ ] Keep physical-device acceptance requirements explicit.

---

# Implementation order

Use this order to minimize broken intermediate states:

1. [ ] P0.1 baseline inventory and regression tests.
2. [ ] P0.2 presentation workflow models.
3. [ ] P0.3 UI effects.
4. [ ] P0.4 startup gate.
5. [ ] P0.5 contextual permissions.
6. [ ] P0.6 routes and navigation helpers.
7. [ ] P0.7 confirmation/back handling infrastructure.
8. [ ] P0.8 Home redesign.
9. [ ] P0.9-P0.10 host setup redesign.
10. [ ] P0.11 explicit approval actions.
11. [ ] P0.12-P0.13 hosting dashboard and health.
12. [ ] P0.14-P0.15 discovery redesign.
13. [ ] P0.16-P0.18 join redesign and persistent failures.
14. [ ] P0.19-P0.20 playback redesign.
15. [ ] P0.21-P0.23 Connection Help, Advanced Diagnostics, and expert tuning.
16. [ ] P0.24 message/error architecture cleanup.
17. [ ] P0.25 accessibility pass.
18. [ ] P0.26-P0.29 complete test matrix.
19. [ ] P1 settings, reusable components, support reports, invitations, and adaptive layouts.
20. [ ] P2 convenience features only after authoritative domain support is verified.

Each numbered slice should compile and pass relevant tests before moving to the next slice.

---

# Required validation before completion

## Rust quality gates

Run from `rust/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## Android build and static validation

Run from repository root:

```bash
./gradlew \
  assembleDebug \
  assemblePocDebug \
  assembleRelease \
  assembleDebugAndroidTest \
  --stacktrace \
  --console=plain

./gradlew test --stacktrace --console=plain
./gradlew lintDebug --stacktrace --console=plain
```

## Native-library packaging

Verify `libsilent_disco_ffi.so` exists in debug, PoC-debug, and release APKs for:

- [ ] `armeabi-v7a`
- [ ] `arm64-v8a`
- [ ] `x86`
- [ ] `x86_64`

## Connected instrumentation

Run the complete connected suite on a booted Android test device/emulator:

```bash
./gradlew connectedDebugAndroidTest --stacktrace --console=plain
```

Recommended CI emulator configuration based on the validated repository setup:

- API 34;
- `aosp_atd` target;
- `x86_64` architecture;
- KVM enabled;
- required Linux emulator libraries installed.

## Physical-device acceptance — must remain explicit

The redesign is not fully device-accepted until verified on at least two physical Android devices.

Host workflow:

- [ ] contextual permissions;
- [ ] audio file selection;
- [ ] manual approval;
- [ ] invite-code approval;
- [ ] approve once;
- [ ] always allow;
- [ ] playback start/pause/stop;
- [ ] end-session confirmation;
- [ ] host Connection Help.

Listener workflow:

- [ ] automatic discovery;
- [ ] session selection;
- [ ] invite-code entry;
- [ ] waiting-for-approval UI;
- [ ] automatic playback navigation;
- [ ] local volume;
- [ ] reconnect/problem UX;
- [ ] resynchronize audio;
- [ ] leave confirmation;
- [ ] listener Connection Help.

Resilience:

- [ ] permission denial and permanent denial;
- [ ] recoverable storage startup failure;
- [ ] fatal storage startup failure;
- [ ] host rejection;
- [ ] invalid invite code;
- [ ] host leaves during join;
- [ ] listener moves out of range;
- [ ] host/listener reconnect;
- [ ] playback engine failure where reproducible;
- [ ] process recreation/configuration change during setup and active workflows.

Do not mark physical-device items complete based only on unit tests, Compose tests, APK assembly, or emulator execution.

---

# Cleanup requirements

Before marking this TODO complete:

- [ ] Remove obsolete screens and callbacks after replacements are fully wired.
- [ ] Remove obsolete route names.
- [ ] Remove the Home storage/permission cards.
- [ ] Remove the **Continue to Playback** action.
- [ ] Remove always-visible healthy-state Reconnect.
- [ ] Remove normal-UI request IDs and session IDs.
- [ ] Remove or deprecate `rememberApprovedDevices` presentation state.
- [ ] Remove duplicated status-label logic from composables.
- [ ] Remove dead preview/test fixtures.
- [ ] Remove temporary debug files, logs, screenshots, workflows, and generated artifacts from the repository.
- [ ] Confirm only intended source, test, resource, and documentation files changed.
- [ ] Confirm `git status` is clean after committing.
- [ ] Confirm remote `master` points to the validated commit.

---

# Definition of done

The UI/UX redesign is complete only when all of the following are true:

- [ ] Startup storage initialization is handled before Home.
- [ ] Home is role-first and free of healthy technical-status cards.
- [ ] Permissions are requested contextually.
- [ ] Host setup is split into Music and Access steps.
- [ ] Invite-code generation and validation work.
- [ ] Global `Remember approved devices` UI is gone.
- [ ] Pending listeners support Approve once, Always allow, and Reject.
- [ ] Durable trust ordering remains correct.
- [ ] Hosting Dashboard prioritizes playback, requests, connected listeners, and session health.
- [ ] Nearby-session discovery starts automatically.
- [ ] Join progress uses understandable user-facing steps.
- [ ] Playback opens automatically when actually ready.
- [ ] Critical failures are persistent and actionable.
- [ ] Healthy Now Playing does not show an unnecessary Reconnect action.
- [ ] Connection Help is the default troubleshooting destination.
- [ ] Advanced Diagnostics preserves all technical information.
- [ ] Expert tuning is gated and fail-visible.
- [ ] Active host/listener back actions require confirmation.
- [ ] Accessibility requirements are met.
- [ ] Rust, Android unit, Compose, navigation, lint, build, packaging, and connected tests pass.
- [ ] Physical-device results are recorded honestly.
- [ ] No temporary validation infrastructure or untracked artifacts remain.
- [ ] All intended files are committed and pushed directly to `master`.
