# Silent Disco — Code Review 1 UI/UX TODO

Derived from the UI/UX code review conducted on 2026-06-07.
Items are ordered by impact on real-device testing usability.

---

## 1. Replace raw enum `.toString()` with human-readable labels

Raw enum names (e.g. `ADVERTISING`, `STOPPED`, `INVITE_CODE`) are displayed directly to users throughout the app. Every display site needs a label function or a `when` mapping.

### 1.1 Host lifecycle state labels
- [x] Add `hostStateLabel(state: HostLifecycleState): String` in `AppState.kt` covering all states:
  - `IDLE` → "Idle"
  - `CREATING_SESSION` → "Creating session…"
  - `ADVERTISING` → "Advertising…"
  - `WAITING_FOR_LISTENERS` → "Waiting for listeners"
  - `READY` → "Ready"
  - `STREAMING` → "Streaming"
  - `PAUSED` → "Paused"
  - `ENDING_SESSION` → "Ending session…"
  - `ERROR` → "Error"
- [x] Replace `"Host state: ${uiState.hostState}"` in `HostControlScreen.kt` with the new label

### 1.2 Playback state labels
- [x] Add `playbackStateLabel(state: PlaybackState): String` in `AppState.kt` covering all states:
  - `STOPPED` → "Stopped"
  - `PLAYING` → "Playing"
  - `PAUSED` → "Paused"
  - (any others present in the enum)
- [x] Replace `"Playback: ${uiState.hostPlaybackState}"` in `HostControlScreen.kt`
- [x] Replace `"Playback state: ${uiState.listenerPlaybackState}"` in `ListenerPlaybackScreen.kt`
- [x] Replace `"Playback state: ${uiState.listenerDiagnostics.playbackState}"` in `DiagnosticsScreen.kt`

### 1.3 Approval mode labels
- [x] Add `approvalModeLabel(mode: ApprovalMode): String` in `AppState.kt`:
  - `MANUAL` → "Manual approval"
  - `INVITE_CODE` → "Invite code"
  - `AUTOMATIC` → "Automatic"
- [x] Replace `mode.name.replace("_", " ")` in `HostSetupScreen.kt` with the label function
- [x] Replace `"Approval: ${session.approvalMode}"` in `DiscoverSessionsScreen.kt`

### 1.4 Listener info field labels on HostControlScreen
- [x] Add `joinStateLabel(state: JoinApprovalState): String` in `AppState.kt`
- [x] Add `connectionStateLabel(state: TransportConnectionState): String` in `AppState.kt`
- [x] Add `syncQualityLabel(badge: SyncQualityBadge): String` in `AppState.kt`
- [x] Replace `"Join state: ${listener.joinState}"` in `HostControlScreen.kt`
- [x] Replace `"Transport: ${listener.connectionState}"` in `HostControlScreen.kt`
- [x] Replace `"Sync: ${listener.syncQuality}"` in `HostControlScreen.kt`

### 1.5 Diagnostics stream state label
- [x] Add `streamStateLabel(state: Any): String` or extend playback label to cover stream state
- [x] Replace `"Stream state: ${uiState.hostDiagnostics.streamState}"` in `DiagnosticsScreen.kt`

---

## 2. Remove / gate the "Add Demo Join" debug button

The "Add Demo Join" button is a development artifact exposed in the production UI flow.

- [x] Wrap `onAddDemoJoinRequest` call and button in a debug-only build check:
  - Add `val isDebugBuild: Boolean` flag (e.g. via `BuildConfig.DEBUG`) to `HostControlScreen` parameters or check inline
- [x] Remove the "Add Demo Join" button from `HostControlScreen.kt` UI entirely in release builds
- [x] Remove or hide the `onAddDemoJoinRequest` parameter from `HostControlScreen` signature in non-debug builds (or make it a no-op)
- [x] Verify "Add Demo Join" still works in debug builds for development use

---

## 3. Replace boolean progress list with a visual stepper

`JoinProgressScreen` displays raw booleans (`"Discovered: true"`, `"Requested: false"`) which is developer output, not user UX. Replace with a visual step indicator and a single human-readable status message.

- [x] Design a `ConnectionStepRow` composable that renders a step with:
  - A check icon (completed), a progress indicator (in-progress), or a circle (pending)
  - A step label string
  - Visual distinction between completed/active/pending states
- [x] Replace the boolean `Text()` lines in `JoinProgressScreen.kt` with `ConnectionStepRow` for each step:
  - Discovering session
  - Sending join request
  - Awaiting host approval
  - Connecting transport
  - Syncing clock
  - Buffering audio
  - Playing
- [x] Add a single prominent status message below the step list showing the current `listenerStateLabel()` in `bodyLarge` style
- [x] Remove the raw `"Current state: ${uiState.listenerStateLabel()}"` text once it is rendered more prominently

---

## 4. Show / hide JoinProgressScreen buttons based on state

All four action buttons are always visible simultaneously. Buttons should appear only when they are actionable.

- [x] "Request Join" — show only when state is `IDLE`, `SESSION_SELECTED`, `JOIN_REQUESTED`, or `ERROR`; hide otherwise
- [x] "Continue to Playback" — show only when state is `PLAYING`
- [x] "Retry" — show only when state is `ERROR` or `DISCONNECTED`
- [x] "Cancel" — always visible except when state is `PLAYING`
- [x] Audit each button's `enabled` state in addition to visibility so partially-valid states are handled gracefully

---

## 5. Fix "Continue to Playback" silent no-op

When tapped before the listener is in `PLAYING` state, the button does nothing and gives no feedback.

- [x] Remove the `if (uiState.listenerState == PLAYING)` guard in `SilentDiscoApp.kt` and instead hide the button when not applicable (per item 4 above)
- [x] Alternatively, if the button must remain visible when not yet playing, show a snackbar: `"Not yet playing — waiting for sync and buffering to complete"`
- [x] Add a unit test or manual checklist item confirming the button behaves correctly in both states

---

## 6. Remove placeholder / dev artifact text from session cards

`DiscoverSessionsScreen` contains hardcoded placeholder strings that look broken to real users.

- [ ] Remove `"Connection quality hint: invite protected"` and `"Connection quality hint: open local session"` strings — these are mislabelled and misleading
- [ ] Remove `"Signal / availability: local demo transport"` — this is a dev placeholder
- [ ] Replace with factual session card content:
  - Show `"Invite code required"` or `"Open — no code required"` based on `session.inviteCodeRequired`
  - Show approval mode using the label from item 1.3
  - Optionally show device count or last-seen time if that metadata is available

---

## 7. Fix radio button and checkbox tap targets

Radio buttons and checkboxes have tap targets limited to the widget itself, not the label. This is a Material accessibility issue and a common user frustration on small screens.

### 7.1 Approval mode radio buttons (`HostSetupScreen.kt`)
- [ ] Wrap each approval mode `Row` in a `Modifier.selectable(selected = ..., onClick = ...)` with `role = Role.RadioButton`
- [ ] Verify the full row (widget + label) responds to taps

### 7.2 Remember approved devices checkbox (`HostSetupScreen.kt`)
- [ ] Wrap the checkbox `Row` in a `Modifier.toggleable(value = ..., onValueChange = ...)` with `role = Role.Checkbox`
- [ ] Verify the full row responds to taps

---

## 8. Add loading / in-progress indicators

No screen shows any animated feedback during async operations. Users cannot distinguish between "working" and "stuck".

### 8.1 Discover Sessions scan state
- [ ] Add a `isScanning: Boolean` field to `AppUiState` (set true during scan, false on completion or error)
- [ ] Show a `LinearProgressIndicator` or `CircularProgressIndicator` below the "Scan / Refresh" button when scanning
- [ ] Disable the "Scan / Refresh" button while scanning to prevent double-tap

### 8.2 JoinProgressScreen connecting / syncing states
- [ ] Show a `CircularProgressIndicator` next to the currently active step while in transient states (`CONNECTING`, `SYNCING_CLOCK`, `BUFFERING`, `JOIN_REQUESTED`, `AWAITING_APPROVAL`)
- [ ] Remove the indicator once the step completes or errors

### 8.3 HostSetupScreen "Start Hosting"
- [ ] Show a brief loading state while `createHostSession()` initialises Wi-Fi Direct and BLE advertising (if those are async)
- [ ] Disable "Start Hosting" button while hosting startup is in progress

### 8.4 ListenerPlaybackScreen buffering state
- [ ] If `listenerPlaybackState` is `BUFFERING`, show a `LinearProgressIndicator` above the volume slider to communicate that audio is buffering before playback begins

---

## 9. Show volume percentage next to the volume slider

The slider gives no numeric feedback.

- [ ] Add a `Text` showing the current volume as a percentage next to or below the "Local volume" label:
  - e.g. `"Local volume — ${(uiState.localVolume * 100).roundToInt()}%"`
- [ ] Keep the text updated as the slider value changes (it is already reactive via `uiState.localVolume`)

---

## 10. Apply a basic custom theme

The app uses `darkColorScheme()` with no overrides, producing a generic unbranded look.

- [ ] Define a custom `SilentDiscoColors` palette in `Theme.kt`:
  - Pick a primary color appropriate for a music/audio app (e.g. a deep teal, indigo, or purple)
  - Set `primary`, `onPrimary`, `primaryContainer`, `secondary`, `background`, `surface` at minimum
- [ ] Apply the custom color scheme to `darkColorScheme(primary = ..., secondary = ..., ...)` in `SilentDiscoTheme`
- [ ] Optionally define a `Typography` override for the heading and body styles if the default Roboto scale is insufficient
- [ ] Verify the theme change does not break any screen's readability

---

## 11. Add a TopAppBar with back navigation

No screen has a visible back button or app bar. Users rely entirely on the Android system back gesture.

- [ ] Add a `TopAppBar` to each non-home screen using `Scaffold(topBar = ...)`:
  - Display the screen title in the top bar (remove the duplicate `headlineMedium` title `Text` from the screen body)
  - Add a back arrow `IconButton` (using `Icons.AutoMirrored.Filled.ArrowBack`) that calls `navController.popBackStack()`
  - Home screen does not need a back arrow
- [ ] Screens affected: `HostSetupScreen`, `HostControlScreen`, `DiscoverSessionsScreen`, `JoinProgressScreen`, `ListenerPlaybackScreen`, `DiagnosticsScreen`
- [ ] Pass an `onBack: () -> Unit` callback into each screen composable from `SilentDiscoApp.kt` (consistent with existing callback pattern)

---

## 12. Add icons to primary action buttons

All buttons are text-only. Material Icons on key actions improve scannability and visual polish.

- [ ] Add dependency for `androidx.compose.material:material-icons-extended` if not already present in `build.gradle.kts`
- [ ] Add icons to the following buttons:
  - "Start" → `Icons.Filled.PlayArrow`
  - "Pause" → `Icons.Filled.Pause`
  - "Stop" → `Icons.Filled.Stop`
  - "End Session" → `Icons.Filled.Close`  
  - "Leave Session" → `Icons.AutoMirrored.Filled.ExitToApp`
  - "Diagnostics" → `Icons.Filled.BarChart`
  - "Share Debug Info" → `Icons.Filled.Share`
  - "Scan / Refresh" → `Icons.Filled.Refresh`
  - "Manual Resync" → `Icons.Filled.Sync`
- [ ] Use `Button(icon + text)` pattern: `Icon` + `Spacer(Modifier.size(ButtonDefaults.IconSpacing))` + `Text`

---

## 13. Disable buttons when actions are not valid

Several buttons are always enabled regardless of app state, causing silent failures.

- [ ] `HostControlScreen` playback buttons:
  - "Start" — disable when `hostPlaybackState == PLAYING` or no audio file selected
  - "Pause" — disable when `hostPlaybackState != PLAYING`
  - "Stop" — disable when `hostPlaybackState == STOPPED`
- [ ] `HostSetupScreen`:
  - "Start Hosting" — disable when `sessionName` is blank or `selectedAudio` is null; show a helper text explaining what is missing
- [ ] `DiscoverSessionsScreen`:
  - "Join" on each session card — disable if a join is already in progress for another session

---

## 14. Fix home screen subtitle copy

The subtitle is a spec/developer description, not user-facing copy.

- [ ] Replace `"Offline host/listener sync validation with BLE discovery, Wi-Fi Direct transport, and Oboe-oriented playback."` with something user-oriented, e.g.:
  - `"Play music in sync across multiple phones — no internet required."`
- [ ] Keep the PoC nature honest without using implementation jargon

---

## 15. Move diagnostics-level detail off the ListenerPlaybackScreen

The playback screen includes raw diagnostic counters that belong in the Diagnostics screen.

- [ ] Remove `"Buffer depth: ${uiState.listenerDiagnostics.bufferDepthMs} ms"` from `ListenerPlaybackScreen`
- [ ] Remove `"Concealed packets: ${uiState.listenerDiagnostics.concealedPacketCount}"` from `ListenerPlaybackScreen`
- [ ] Remove `"EOF reached: ${uiState.listenerDiagnostics.endOfStreamReached}"` from `ListenerPlaybackScreen`
- [ ] Keep `Sync quality`, `Connection quality`, and `Playback state` on the playback screen — these are user-relevant
- [ ] Verify the removed fields remain visible in `DiagnosticsScreen`

---

## 16. Fix `navController.context` usage in SilentDiscoApp

`navController.context` is a slightly fragile way to obtain a `Context` inside a `NavHost` composable.

- [ ] Replace with `LocalContext.current` from Compose to get the context for the share intent in the `DiagnosticsScreen` composable destination
- [ ] Verify share intent still launches correctly after the change

---

## Testing checklist for this review pass

After implementing the above, manually verify:

- [ ] All screens show human-readable text only — no raw enum names visible
- [ ] "Add Demo Join" does not appear on a release build
- [ ] JoinProgressScreen stepper advances correctly through all states on a real or simulated join flow
- [ ] Only contextually valid buttons are visible on JoinProgressScreen at each state
- [ ] "Continue to Playback" navigates correctly when state is PLAYING
- [ ] Session cards show clean, accurate content with no placeholder text
- [ ] Radio button and checkbox rows respond to taps on the label text
- [ ] Scan state shows a loading indicator
- [ ] Volume slider displays percentage
- [ ] Theme uses custom brand colors
- [ ] All non-home screens have a TopAppBar with back navigation
- [ ] Start/Pause/Stop buttons reflect correct enabled states
- [ ] "Start Hosting" shows a validation message when fields are missing
