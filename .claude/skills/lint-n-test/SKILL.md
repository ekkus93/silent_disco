---
name: lint-n-test
description: Lint the files and run all tests across the repo (Rust workspace, desktop app, Android app). Use when asked to lint and/or test the whole project, or before committing broad changes that touch more than one part of the monorepo.
model: haiku
---

Run every quality gate this repo defines, across all three parts of the monorepo. Fix any failures before proceeding — do not report success unless each command actually completed without errors.

## 1. Rust workspace (`rust/`)

```bash
bash scripts/check-rust.sh
```

Runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --all-features` with the pinned toolchain.

## 2. Desktop app (`desktop/`)

```bash
cd desktop
npm run check
```

Runs UniFFI bindings-check, Biome format/lint, `tsc`, Vitest, and a production build, in sequence.

## 3. Android app

```bash
./gradlew test lintDebug --stacktrace --console=plain
```

Runs Android unit tests and Android Lint against the debug variant. This mirrors the `android-check` skill but is included here so this skill covers the whole repo on its own.

## Reporting

Summarize pass/fail for each of the three areas. If something fails, show the actual failure output, fix it, and re-run that area's command — never claim a gate passed without having actually executed it.
