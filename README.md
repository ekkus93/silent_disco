# silent_disco

[![CI](https://github.com/ekkus93/silent_disco/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/ci.yml)
[![Desktop CI](https://github.com/ekkus93/silent_disco/actions/workflows/desktop-ci.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/desktop-ci.yml)
[![Source file line limit](https://github.com/ekkus93/silent_disco/actions/workflows/source-file-line-limit.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/source-file-line-limit.yml)

Silent Disco is an offline local-network synchronized-audio system with a shared Rust core, an Android listener/host application, and a Tauri Linux desktop host.

The current desktop release target is **Ubuntu 22.04 amd64**. Windows, macOS, and iOS are future-platform work unless a later acceptance record explicitly says otherwise.

## Authoritative project documents

Read these before changing architecture or completion state:

- `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md`
- `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md`
- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md`
- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`
- `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md` for the current physical-listener playback history and measurements
- `memory.md` for dated implementation, test, and real-device evidence

`CLAUDE.md` contains the repository working rules and ownership boundaries.

## Architecture in one page

- Rust owns domain state, protocol semantics, synchronization, packetization, listener scheduling, persistence, diagnostics, and transport semantics.
- Android Kotlin/Compose owns Android presentation and Android platform adapters.
- The Tauri Rust backend owns desktop platform integration only; React/Redux is presentation-only.
- PCM/sample data does not cross Tauri IPC.
- Desktop local monitoring consumes the same Rust-owned scheduled timeline as listener playback and must not interfere with network transmission.
- Production identity secrets are stored through the operating-system credential store. There is no plaintext or environment-variable identity fallback.
- Lab Mode is developer-only and is excluded from ordinary production bundles.

## Prerequisites

### Shared Rust

- Rust **1.97.1**, pinned by `rust/rust-toolchain.toml`
- `rustfmt` and Clippy for that toolchain

The repository scripts/toolchains install or select the pinned Rust version where appropriate, but a normal Rustup installation must already be available.

### Android

- JDK 17
- Android SDK platform 36
- Android build-tools 36.0.0
- Android NDK **28.2.13676358**
- `adb` for physical-device acceptance

Set `ANDROID_SDK_ROOT` or `ANDROID_HOME`, or provide `sdk.dir` in `local.properties`. `scripts/build-rust-android.sh` installs its pinned `cargo-ndk` 4.1.2 copy under `.gradle/rust-tools/` when needed.

### Linux desktop host

The accepted production baseline is Ubuntu 22.04 amd64. The desktop package build needs Node **>=22.13.0 <23** plus the Tauri/Linux development packages used by CI:

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential curl dbus-x11 file libasound2-dev \
  libayatana-appindicator3-dev librsvg2-dev libssl-dev \
  libwebkit2gtk-4.1-dev libxdo-dev patchelf wget xauth xvfb
```

A normal graphical production launch also needs an active D-Bus user session and an available Secret Service provider so the desktop identity can be opened or created securely.

## Clean builds

From a clean checkout, Android and the shared Rust core can be rebuilt with:

```bash
./gradlew clean
./gradlew assembleDebug --stacktrace --console=plain
```

The Android build automatically builds and packages the Rust shared library for `armeabi-v7a`, `arm64-v8a`, `x86`, and `x86_64`.

For a clean desktop dependency/install and production build:

```bash
cd desktop
npm ci
npm run tauri build
```

The selected Linux bundle outputs are AppImage and `.deb` under:

```text
desktop/src-tauri/target/release/bundle/appimage/
desktop/src-tauri/target/release/bundle/deb/
```

## Development launch

Run the production-shaped desktop application through Tauri, not just the Vite frontend:

```bash
cd desktop
npm ci
npm run tauri dev
```

The frontend development server is bound to `127.0.0.1:1420` by the Tauri configuration.

## Production bundle

Build the ordinary production Linux packages with:

```bash
cd desktop
npm ci
npm run tauri build
```

`npm run build` verifies that production frontend output contains no Lab Mode entry point. Do not add `lab-mode` features or `tauri.lab.conf.json` to an ordinary release build.

Verify package contents and package lifecycle with:

```bash
python3 desktop/scripts/verify-linux-bundles.py \
  --bundle-dir desktop/src-tauri/target/release/bundle \
  --tauri-config desktop/src-tauri/tauri.conf.json \
  --cargo-manifest desktop/src-tauri/Cargo.toml

bash desktop/scripts/smoke-linux-package-lifecycle.sh \
  desktop/src-tauri/target/release/bundle \
  com.ekkus.silentdisco.desktop \
  silent-disco-desktop
```

## Test and quality gates

Shared Rust:

```bash
bash scripts/check-rust.sh
```

Equivalent commands are `cargo fmt --all -- --check`, strict workspace Clippy with `-D warnings`, and `cargo test --workspace --all-features` under `rust/`.

Android build, JVM tests, and lint:

```bash
./gradlew \
  assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  test lintDebug \
  --stacktrace --console=plain
```

Gradle-managed Android instrumentation:

```bash
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain
```

Desktop frontend plus bindings/backend formatting checks:

```bash
cd desktop
npm ci
npm run check
```

Desktop Rust strict gates, which are intentionally run separately from `npm run check`:

```bash
cd desktop/src-tauri
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --all-features
```

A physical Android acceptance run is not interchangeable with instrumentation or emulator tests.

## Physical desktop-to-Android interoperability

Block 47 requires a **packaged Ubuntu 22.04 desktop build and two physical Android devices**. Use a desktop bundle and Android debug APK built from the exact same accepted commit; when using Desktop CI, its `desktop-linux-bundle` artifact is suitable for the Linux side. Connect both phones through `adb`, then run:

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

The runner executes and records the production matrix for:

- manual endpoint join;
- mDNS discovery;
- QR invitation;
- rejection followed by approval;
- one- and two-listener audio;
- pause/resume/stop/end;
- Android disconnect/reconnect;
- desktop-interface disruption and recovery;
- host-source failure;
- local-monitor failure while preserving listener-transmit policy;
- diagnostics export;
- controlled shutdown, packaged-app restart, and profile/session-history reopen.

It also records desktop package version/SHA, APK version/SHA, both device/OS inventories, network topology, exact automated commands, PASS/FAIL/BLOCKED/NOT RUN status, synchronization measurements, and known limitations. Any result other than PASS leaves Block 47 open and makes the runner exit non-zero.

Block 46.3 fresh-machine package validation remains a separate prerequisite and uses:

```bash
bash desktop/scripts/validate-block46-fresh-machine.sh \
  desktop/src-tauri/target/release/bundle
```

## Lab Mode and deterministic scenarios

Lab Mode is a developer testing surface. It must be built explicitly:

```bash
cd desktop
npm ci
npm run tauri:lab:dev
```

For a packaged Lab build use `npm run tauri:lab:build`. Do **not** distribute it as the production package.

To exercise a deterministic scenario, save the following as `/tmp/silent-disco-lab-smoke.json`:

```json
{
  "schemaVersion": 1,
  "seed": 7,
  "nodes": [{"id": "host1"}],
  "links": [
    {"from": "host1", "to": "host1", "latencyMs": 30, "jitterMs": 8, "lossPermille": 10}
  ],
  "steps": [
    {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
    {"atMs": 10, "node": "host1", "action": {"kind": "exportDiagnostics"}}
  ],
  "assertions": [
    {
      "kind": "lifecycleReached",
      "byMs": 50,
      "node": "host1",
      "target": {"machine": "role", "state": "host"}
    }
  ],
  "timeoutMs": 100
}
```

In the Lab screen choose **Open scenario…**, select that file, inspect the loaded seed/link fault configuration, and choose **Run scenario**. The run must end with a completed outcome and held assertion. The same schema/runner is covered by desktop Rust tests, so source changes to deterministic Lab behavior must keep `cargo test --locked --all-features` green.

## Diagnostics

For the production desktop profile, application-owned data is below the Tauri local-data root in:

```text
com.ekkus.silentdisco.desktop/profiles/<profile-id>/
```

On the accepted Linux/XDG layout this resolves below `$XDG_DATA_HOME` (normally the user's local data directory). Internal diagnostics artifacts use the profile's `diagnostics/` directory.

For support or acceptance evidence, prefer the **Diagnostics** screen's export action. It opens a native save dialog and writes the bounded, redacted `DesktopDiagnosticsDto` JSON to the path you choose. A successful export is limited to 1 MiB and excludes private key material, identity secrets, invite/session secrets, and raw private paths.

Do not copy the SQLite database or secure-store contents as a substitute for the supported diagnostics export.

## Secure-store troubleshooting

The production desktop identity uses the operating-system credential store through `keyring`; on Linux this is the Secret Service backend over the user's D-Bus session. Identity startup is deliberately fail-closed.

If profile startup reports that the credential store is unavailable, locked, unreadable, or cannot persist/verify the identity:

1. launch Silent Disco from the same graphical login session as the user's keyring/Secret Service provider;
2. verify `DBUS_SESSION_BUS_ADDRESS` is present in that session;
3. make sure the desktop keyring/Secret Service provider is running and unlocked;
4. retry profile startup and use the visible structured error/Diagnostics information if it still fails.

Do **not** work around the error by writing the identity secret to a file, environment variable, SQLite, preferences, or React state; do not add an in-memory/random-per-launch production fallback; and do not convert secure-store failure into anonymous success. Tests may inject fake identity providers, but production must continue to use the OS credential store or fail visibly.

## Release completion

Run the dependency-free documentation/completion audit with:

```bash
python3 scripts/audit-block48-completion.py
```

The authoritative release-completion ledger is `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`. A checkbox that requires physical hardware, packaged-app behavior, or a platform-specific environment is complete only from corresponding executable evidence; code inspection or CI simulation alone is not enough.
