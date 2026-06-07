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
