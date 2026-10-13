# Repository Guidelines

## Project Structure & Architecture

- `rust/` contains the shared Rust workspace: domain/core logic, UniFFI bindings, and test support. Core integration tests live in `rust/silent-disco-core/tests/`; protocol fixtures are under `rust/silent-disco-core/testdata/`.
- `app/` is the Android Kotlin/Jetpack Compose app. JVM tests are in `app/src/test/`, instrumentation tests in `app/src/androidTest/`, and Android resources/native code under `app/src/main/`.
- `desktop/` is the Tauri desktop app: React/Redux presentation in `desktop/src/` and Rust platform integration in `desktop/src-tauri/`.
- `docs/` holds architecture specifications and acceptance records; `scripts/` contains repository build and validation helpers. Read the relevant architecture spec and TODO before changing ownership boundaries. Rust owns domain state and protocol semantics; UI layers present state and adapt platform APIs.

## Build, Test, and Development

- `bash scripts/check-rust.sh` runs Rust formatting, strict Clippy, and workspace tests.
- `./gradlew assembleDebug test lintDebug --stacktrace --console=plain` builds Android and runs JVM tests and lint. See `README.md` for instrumentation and physical-device acceptance commands.
- In `desktop/`, run `npm ci` once, then `npm run check` for bindings, formatting, lint, type checking, tests, and production frontend build. Run the app with `npm run tauri dev`.
- Desktop Rust strict checks are separate: from `desktop/src-tauri/`, run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo test --locked --all-features`.

## Coding Style & Naming

Follow existing language conventions and formatters: Rust uses `rustfmt`; desktop TypeScript/React uses Biome (100-column width, double quotes, semicolons); Android follows surrounding Kotlin style. Keep modules focused and tests descriptive, using `*_test.rs`/`tests/*.rs` for Rust integration tests and `*Test.kt` for Android tests. Do not edit generated bindings by hand; use the binding generation/check scripts.

## Testing Expectations

Add or update focused tests with behavior changes, and run the relevant checks above. Android emulator/instrumentation results do not replace physical-device acceptance. Do not report a gate as passing unless it was run with the repository-pinned toolchain and completed successfully.

## Commits & Pull Requests

Recent history uses Conventional Commit-style subjects such as `fix(core): ...`, `feat(desktop): ...`, `test(desktop): ...`, and `docs: ...`. Keep subjects imperative and scoped. PRs should explain behavior and affected platform, link related issues or acceptance records, list checks actually run, and include screenshots for visible UI changes. Call out platform/device prerequisites for validation.

## Configuration & Sensitive Data

Use the prerequisites and local setup in `README.md`. Keep credentials and signing material out of source control; production identity secrets must use the operating system credential store. Preserve the rule that Lab Mode is excluded from ordinary production bundles.
