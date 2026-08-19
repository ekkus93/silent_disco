# Silent Disco Blocks 47–48 Resume Checkpoint — 2026-08-18

## Purpose

This checkpoint continues `docs/SILENT_DISCO_BLOCK47_BLOCK48_HANDOFF_2026-08-13.md` and records the work completed when the project resumed on 2026-08-18. It is intentionally **not** a final completion ledger: compiler-backed CI evidence, fresh-machine validation, and physical Android interoperability remain required.

## Repository state

The two-listener startup-alignment implementation is committed directly to `master` as:

```text
434e84959c41425684b23fcc2bc286801223b5aa
Fix two-listener startup alignment
```

Parent handoff commit:

```text
7f55cba6496dab6dfacdb14f38c0745fb4eabbdb
Add Blocks 47 and 48 project handoff
```

The uploaded master snapshot used as the sandbox baseline was verified byte-for-byte against the corresponding GitHub `master` blobs before the timing commit was created.

## What the timing fix changes

The previous alignment attempt regressed on a real Android listener because the scheduler treated the playback pump's roughly 400 ms **future write-ahead horizon** as if it were **actual current monotonic time**. That caused still-future packets to be classified as stale, discarded them, and could repeatedly empty/rebuffer the stream.

The replacement design separates those clocks:

- `PlaybackScheduler::poll(local_now_ms)` remains the ordinary no-write-ahead API.
- The playback pump uses an internal `poll_with_release_horizon(actual_now, release_horizon)` form.
- `actual_now` alone decides whether startup/rebuffer head packets are already irretrievably late.
- `release_horizon` only controls how far ahead the non-real-time producer may queue into the FIFO render ring.
- Buffered stale packets are discarded without being counted as emitted.
- Missing head slots whose deadlines are already past are skipped rather than concealed; concealing an already-past slot would recreate listener-specific timeline skew.
- Once a startup/rebuffer target has genuinely been accumulated, stale-head alignment remembers that fact. If alignment empties the entire stale buffer, the scheduler waits for the first reachable live packet instead of demanding another full target and entering an accumulate/discard loop.
- Zero-buffer-target semantics used by concealment-focused tests remain intact.

## Regression coverage added or activated

The commit adds/updates tests for:

1. the original large-write-lead failure mode: true current time must win over the future release horizon;
2. a target-sized buffer becoming wholly stale, followed by immediate recovery on the next live packet;
3. an already-past missing head slot;
4. mid-stream bootstrap after stale bootstrap packets are discarded;
5. jitter-buffer discard accounting (`skipped`, not falsely `emitted`);
6. two schedulers locking sync at different moments and reaching the same sequence within one packet of modeled playout time.

The two-listener alignment test that was previously `#[ignore]` is now an ordinary active Rust test.

## Local validation performed

Dependency-free checks completed successfully in the supplied sandbox working copy:

- `python3 scripts/audit-block48-completion.py`
  - passed with **88 repository references** checked;
  - **4 ignored Rust tests** remain, all explicit manual/device interoperability tests with reason and owner;
- `python3 -m py_compile scripts/audit-block48-completion.py`;
- `bash -n desktop/scripts/validate-block46-fresh-machine.sh`;
- `bash -n desktop/scripts/validate-block47-android-interoperability.sh`;
- manual source-size scan: 559 source files checked, none above the 800-physical-line limit;
- lightweight delimiter/string/comment structural scan of the modified Rust files;
- stale exact-path scan of the maintained guidance set: no unresolved paths after the local Block 48 reference cleanup;
- Git diff whitespace inspection of the seven published Rust files produced no whitespace-error diagnostics.

The repository's `scripts/check-source-file-line-counts.sh` itself cannot produce a meaningful count in this extracted ZIP because it intentionally enumerates Git-tracked files and the ZIP contains no `.git` metadata. The manual filesystem scan above was used only as a sandbox substitute; it is not a replacement for the normal repository gate.

## Compiler/build gates attempted but unavailable in this sandbox

These were attempted and are **not claimed as passing**:

- `bash scripts/check-rust.sh` — stops because `cargo` is not installed;
- `./gradlew test lintDebug --stacktrace --console=plain` — Gradle wrapper cannot resolve `services.gradle.org` from this sandbox;
- `cd desktop && npm run check` — its first Rust binding check stops because Cargo is missing;
- `cd desktop && npm ci --offline --ignore-scripts` — local npm cache lacks `yargs-parser@18.1.3`.

Attempts to bootstrap the pinned Rust 1.97.1 toolchain also failed because this sandbox cannot resolve/download from `static.rust-lang.org`. No hidden/preinstalled Cargo/Rust toolchain or container runtime was found.

These are environment limitations, not test passes or code-test failures. The normal CI/toolchain environment still needs to compile, format-check, lint, and run the Rust/Android/desktop suites.

## Block 48 bookkeeping advanced locally but not final

The sandbox working copy also contains a local Block 48 cleanup that should be preserved for the later completion pass:

- all nine Block 48.1 developer-documentation items are satisfied by the maintained README and locally marked complete;
- stale post-file-split references in the authoritative desktop TODO were corrected, including listener playback, Lab commands, frontend slice, app-state host operations, and host peer paths;
- the local completion audit includes `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` itself;
- the local audit therefore validates the authoritative TODO's exact paths instead of excluding it;
- the final `memory.md` release ledger remains deliberately open.

Those large ledger-file edits were not bundled into the timing commit. Do not infer Block 48 completion from this checkpoint.

## What remains

### Immediate automated validation

When a normal Rust/Android/Node environment is available, run and fix anything genuine from:

```bash
bash scripts/check-rust.sh
./gradlew test lintDebug --stacktrace --console=plain
cd desktop && npm ci && npm run check
```

Also run the full Block 48.3 gates listed in `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` before final closure.

### Physical acceptance

Still mandatory:

1. **Block 46.3** — fresh graphical Ubuntu 22.04 package validation with a physical Android listener.
2. **Playback review item 4.5** — two physical phones playing the same stream, measuring actual listener-to-listener synchronization after the new alignment implementation.
3. **Block 47** — complete packaged Linux desktop + two-physical-Android interoperability matrix using:

```bash
bash desktop/scripts/validate-block47-android-interoperability.sh \
  desktop/src-tauri/target/release/bundle \
  app/build/outputs/apk/debug/app-debug.apk \
  <DEVICE_A_SERIAL> \
  <DEVICE_B_SERIAL>
```

The runner is fail-closed: any `FAIL`, `BLOCKED`, or `NOT RUN` leaves Block 47 open.

### Final Block 48 closure

Only after the automated and physical evidence is real:

- apply/commit the authoritative TODO reference/bookkeeping cleanup;
- run `python3 scripts/audit-block48-completion.py` with the authoritative TODO included;
- mark only genuinely passing Block 48.3 gates;
- write the final `memory.md` release ledger;
- close Blocks 47/48 only if all acceptance criteria are actually met.

## Workflow reminder

The user monitors GitHub CI. During Ralph Loop work, do **not** poll or monitor CI jobs; wait for the user to report any CI failures and then fix those failures from the supplied master snapshot/local sandbox copy.
