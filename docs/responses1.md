# Claude Code Review 2 Clarifications — responses1.md

Generated 2026-06-28 from initial review of `SILENT_DISCO_CODE_REVIEW2_SPEC.md` and `SILENT_DISCO_CODE_REVIEW2_TODO.md`.

Claude Code has reviewed the spec and TODO list and identified 7 points needing clarification before implementation begins.

---

## 1. Scan window timing (P0.4)

**Question**: The TODO hardcodes `scanWindowMs = 3_000L` (3 seconds). Should this be:
- Exposed as a tuning setting in `TuningSettings` (like other sync/buffer thresholds allow real-device adjustment without rebuilding)?
- Left as a hard-coded constant?

**Context**: The existing codebase already exposes sync window, buffer threshold, late-packet threshold as in-app persisted tuning controls in Diagnostics (per prior memory entry from 2026-06-07). A fixed 3-second scan window is reasonable for a PoC, but exposing it would align with the existing pattern.

---

## 2. Playback error recovery mid-stream (P1.4)

**Question**: Section P1.4 shows state transitions for buffering → playing, but what should `connectionProgress` become if playback encounters an error mid-stream?

**Options**:
- Transition back to `buffering = false, playing = false` (return to BUFFERING state for recovery)?
- Go directly to error and stay there until manual resync/rejoin?
- Something else?

**Context**: Spec section 2.3 documents initial buffering and playback transitions, but does not explicitly cover recovery from mid-stream failures. This affects whether a user can attempt recovery without leaving the session or if they must cancel and rejoin.

---

## 3. Host startup validation consolidation (P1.6-P1.7)

**Question**: After P1.6 and P1.7, `createHostSession()` will contain:
1. Session name / audio file / invite code validation (before side effects)
2. `hostState = CREATING_SESSION` (before side effects)
3. Invite-code mode validation (before BLE/Wi-Fi start)

**Options**:
- Extract all form validation into a single `validateHostForm(): String?` helper that returns an error message or null?
- Keep validation checks scattered inline (as shown in TODO)?

**Context**: Keeping them inline is simple and matches the existing code style. A helper would reduce duplication but add one more file indirection.

---

## 4. Manual resync availability in DISCONNECTED state (P1.10)

**Question**: The helper `canManualResync()` excludes `DISCONNECTED` from resync-eligible states. Is this intentional?

**Implication**: A user whose session drops to DISCONNECTED state cannot call manual resync to reconnect — they must cancel and rejoin.

**Context**: Spec section 1.9 says "manual resync should produce a visible success/progress message when a probe is sent." If the connection is already broken, sending a probe makes less sense. Confirm this is the intended behavior.

---

## 5. OboeBridge diagnostics display (P2.1)

**Question**: P2.1 shows splitting audio backend info into two lines:
```
Text("Playback output: Android AudioTrack")
Text("Native bridge: ${OboeBridge.statusSummary()}")
```

What does `OboeBridge.statusSummary()` currently return, and what should it return after P2.2?

**Context**: P2.2 adds an `isAvailable` state to OboeBridge. Diagnostic output should be clear that AudioTrack is always the playback output and the native bridge is optional/diagnostic-only.

---

## 6. Lint acceptance criteria (Section 8)

**Question**: The acceptance criteria says "`./gradlew lintDebug` either passes, or every failure is explicitly documented with reason and fix plan."

**Current lint state**: Last lint run showed 8 GradleDependency notices (gradle version bumps for androidx.core, lifecycle, navigation, kotlinx-coroutines, and truth). These were investigated and found to require AGP 9.1.0 + compileSdk 37 upgrade to resolve.

**Options**:
- Accept the 8 notices as "documented" (reason: require AGP/compileSdk upgrade, deferred to future pass)?
- Require AGP 9.1.0 + compileSdk 37 upgrade as a prerequisite to this pass?
- Something else?

**Context**: The PoC is currently on AGP 8.9.1 and compileSdk 36. Upgrading would be a separate, sizeable task.

---

## 7. On-device integration test coverage (implicit in testing requirements)

**Question**: The TODO list focuses on unit tests (P1.15-P1.17). Should `connectedDebugAndroidTest` (instrumented/on-device tests) be updated or added to verify:
- Scan lifecycle state transitions end-to-end?
- Join progress step transitions on a real device?
- Invite-code rejection behavior with real transport?

**Context**: All existing `connectedDebugAndroidTest` items in TODO.md are marked unchecked and require physical devices. This pass could add to that workload or defer on-device testing to a later phase.

---

## Ready for implementation once clarified

Once these points are clarified, implementation can proceed in Ralph Loop style: unit tests first, then commit each solved TODO item.
