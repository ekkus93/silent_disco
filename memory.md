# memory.md — `silent_disco`

## 2026-06-07T07:37:04Z - Claude Sonnet 4.6 - Session start; created CLAUDE.md and initialized memory

- Project is an Android-only offline silent disco PoC using Kotlin, Jetpack Compose, Oboe, Wi-Fi Direct (primary transport), and BLE (discovery only).
- CLAUDE.md was created this session, adapted from `.github/copilot-instructions.md`, with a Memory file section added by the user.
- The implementation is nearly complete per `docs/TODO.md` — all core features are checked off. Remaining open items: verify stable Oboe/AAudio playback callback behavior, real-device integration tests, and measuring actual listener capacity.
- Sync/buffer/threshold tuning controls are exposed as in-app persisted settings in Diagnostics to allow real-device adjustment without rebuilding.
- `memory.md` did not previously exist; this is its first entry.

## 2026-06-07T19:17:15Z - Claude Sonnet 4.6 - Documented Ralph Loop workflow in CLAUDE.md

- User confirmed this project uses a Ralph Loop pattern: pick next `[ ]` from `docs/TODO.md`, implement, write unit tests, run tests; if pass → commit + push + mark `[x]` + move on; if fail → keep iterating.
- Added a "Ralph Loop workflow" section to `CLAUDE.md` documenting this process.
- `memory.md` bridges fresh context sessions so each loop iteration knows where to pick up.

## 2026-06-07T19:31:04Z - Claude Sonnet 4.6 - Completed TODO 11.4: Verify stable playback callback/write behavior

- Created `OboePlaybackEngineTest.kt` with 9 unit tests covering: null-AudioTrack fallback paths (pre-start, post-stop, empty payload, idempotent stop, playbackPositionMs), write count tracking, and write-loop integration with ListenerPlaybackScheduler (deadline ordering, null-frame skipping, empty scheduler).
- All tests pass. Committed and pushed (commit 9df11a3).
- This was the last code-implementable unchecked item in docs/TODO.md.
- Remaining unchecked items all require physical Android devices: 17.2 (integration/device tests), 17.3 (manual validation checklist), 18 (measure listener capacity). Section 20 items are explicitly deferred.

## 2026-06-07T22:07:00Z - Claude Sonnet 4.6 - UI/UX code review; created docs/CODE_REVIEW1_TODO.md

- Conducted full UI/UX review of all 7 screens (HomeScreen, HostSetupScreen, HostControlScreen, DiscoverSessionsScreen, JoinProgressScreen, ListenerPlaybackScreen, DiagnosticsScreen) plus Theme.kt, AppState.kt, SilentDiscoApp.kt.
- Key bugs found: raw enum names shown to users everywhere, "Continue to Playback" silently does nothing when tapped early, "Now playing" on listener screen reads from host's file picker field (always null on listener), radio/checkbox tap targets too small.
- Key UX issues: "Add Demo Join" dev button exposed in production UI, boolean progress list in JoinProgressScreen, all 4 action buttons always visible, placeholder "local demo transport" text on session cards, no loading indicators, no button disabled states, no TopAppBar/back navigation, unbranded default Material3 theme.
- Created docs/CODE_REVIEW1_TODO.md with 16 detailed task groups and a manual testing checklist. This is the next Ralph Loop target file.
