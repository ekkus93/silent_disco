# Deferred physical-device acceptance — 2026-10-23

## Scope / handoff rule

The user explicitly deferred **all tasks or subtasks that actually require a physical phone** while software-only work continues autonomously on `master`. These are not accepted, skipped as passing, or blockers to software development. No branches or per-task PRs are needed. All GitHub and CI operations must go through Ralph Bridge; ordinary CI must use GitHub-hosted runners.

The original authority remains `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md`, §9. Restore these tests to the active acceptance scope only when the user makes physical Android devices available. Do not change `[ ]` to `[x]` without evidence.

## Deferred phone/hardware evidence

1. **Packaged desktop host → physical Android listener:** heterogeneous multi-track queue, exact acoustic or wired loopback boundary continuity, real Oboe output open/reopen/close counters, no dropped/duplicated PCM; test physical speakers/headphones and relevant Bluetooth output.
2. **Physical Android host → physical Android listener:** real discovery/approval, session join, timing, synchronization, gapless track transition, queue-end recovery. Test Android-to-Android under real transport and platform permissions.
3. **Real-device event/race matrix:** live add/reorder/remove during playback, pause/resume at boundary, Stop/replay, queue-end/add/Play, late join, Wi-Fi/Bluetooth reconnection, failed source and Retry/Remove; gather logs and objective callback/physical-output measurements.
4. **Objective final-output confirmation:** use the opt-in debug capture in Advanced diagnostics to save callback-returned WAV and compare with canonical decoded reference via `scripts/compare_pcm_capture.py`. Capture telemetry (frames, underruns, dropped tap samples and native output lifecycle). Separately measure real DAC/OS mixer/headphone/Bluetooth audio where required; callback WAV alone is not a physical-output loopback.
5. **Manual device-only accessibility acceptance:** validate real screen readers, physical-device rotation/insets, touch targets and large text where emulator semantics are insufficient. Emulator-based automated accessibility, screenshot/golden and orientation tests remain **active software work** and must not be deferred.
6. **Environment:** two compatible Android test phones, packaged Ubuntu desktop host, local network, authorized sample audio and saved GitHub Actions/APK candidate. Document OS versions, connection type, app SHA, output route, capture sample rate/channels, frame mismatch report and device logs.

## Still active: do not defer

The Rust actor/playlist reservation race; native network-to-listener continuity; canonical mixed-format PCM comparison; Android emulator and JVM/UI tests; desktop/Tauri/React tests; automation of screenshots/goldens; reduced-motion, focus, accessibility and layout; instrumented source staging, failure and lifecycle tests; any CI lint/build/regressions. These can all progress on hosted runners without phones.

## Completion criteria when physical testing resumes

Store *measured evidence* (not listening impression alone), explicitly identify the SHA and hardware configuration, and reconcile against §9 without conflating tap-level PCM with sound emitted from the speaker or DAC. The deferred status is a scope decision, not a pass.
