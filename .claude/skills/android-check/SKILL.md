---
name: android-check
description: Run the local Android quality gate (unit tests + lint) for the silent_disco Android app. Use after making changes under app/ or to the Rust->Android JNI bridge, or when asked to validate/check the Android app, mirroring what scripts/check-rust.sh does for Rust and npm run check does for desktop.
---

Run the Android quality gate that CI enforces in `.github/workflows/ci.yml` (`android` job):

```bash
./gradlew test lintDebug --stacktrace --console=plain
```

- `test` runs Android unit tests.
- `lintDebug` runs Android Lint against the debug variant.

Fix any failures before proceeding — do not report success unless both tasks actually completed without errors.

Not included here (CI-only, requires the full Android SDK/NDK toolchain and emulator setup): `assembleDebug`/`assemblePocDebug`/`assembleRelease`/`assembleDebugAndroidTest`, the per-ABI Rust `.so` packaging check, and the `pixel2api29DebugAndroidTest` managed-device instrumentation suite. Only run those if explicitly asked, and never claim they passed without actually executing them.
