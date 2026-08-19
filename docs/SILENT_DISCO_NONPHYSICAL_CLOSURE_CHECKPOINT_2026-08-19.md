# Silent Disco Non-Physical Closure Checkpoint — 2026-08-19

## Purpose

This checkpoint records the Ralph Loop pass requested after
`docs/SILENT_DISCO_BLOCK47_BLOCK48_HANDOFF_2026-08-13.md`: complete every
current task that can be implemented or audited without physical Android/audio
hardware, while leaving physical acceptance, fresh-machine/environment evidence,
real performance measurements, and unavailable compiler/build gates open.

This is **not** a final release ledger. No physical-device result, performance
number, or compiler-backed pass is inferred from source inspection.

## Repository baseline and preservation

The original closure pass was based on a user-supplied latest-master archive.
This resumed validation was rebased onto the newer user-supplied `master`
snapshot at commit `9ab67a71d00604066d0a53f444e3075d3dd8a84d` and then overlaid with the
byte-verified WIP v7 working-copy snapshot from the persistent file library.
The current-master-only Block 47/48 resume handoff document was preserved.

The already-published two-listener startup-alignment work on GitHub `master`
(commit `434e84959c41425684b23fcc2bc286801223b5aa`) was explicitly reconstructed
in every overlapping local scheduler/pump test file before later timing changes
were made. That overlap is why the full WIP working-copy snapshot, rather than
the older raw `diff -ruN` patch, is authoritative for this resumed tree.

The recovered WIP v7 ZIP and patch matched their recorded SHA-256 hashes exactly.
A newer durable snapshot should be regenerated after this checkpoint is updated.

## Software-only implementation completed in this pass

### Discovery and connection

- Added Android mDNS/NSD discovery for `_silentdisco._tcp.`.
- Desktop advertisements now include the identity/policy/control metadata Android
  needs to reconstruct a real `SessionAdvertisement` without guessing.
- mDNS parsing is fail-closed for protocol-version and SRV/TXT port conflicts.
- BLE and mDNS results merge by session ID, with endpoint-bearing mDNS data
  taking precedence without removing BLE/Wi-Fi Direct as an alternative.
- Selecting an endpoint-backed discovered session now opens the real Rust
  listener transport and sends the join request; it no longer stops at a fake
  “network ready” state.
- Listener transport gained reusable disconnect semantics so join -> leave ->
  join can occur without permanently closing the event stream.
- Android listener discovery/network effect ownership was split into a companion
  source file to remain below the repository's strict 800-line limit.

### Host/listener playback correctness

- Preserved the published true-now vs write-ahead-horizon startup alignment fix.
- Added exact sample-rate packet timing; Rust derives presentation deadlines from
  sample geometry instead of cumulative rounded milliseconds.
- Added a 1024-frame / 48 kHz regression that catches cumulative timeline
  truncation.
- Added bounded mid-stream clock-offset slew (maximum 5 ms per accepted update),
  with convergence, threshold-equality, and non-finite regressions.
- Added post-start render-ring underrun realignment to the live host timeline,
  while proving startup/pre-roll silence does not discard the first future frame.
- Added bounded synchronization acquisition: strict RTT initially and after
  lock, acquisition-only adaptive/hard ceilings before first lock, and explicit
  degraded-lock metadata across core, UniFFI, JNI, and Android UI.
- Added pump-thread liveness fields, contained-panic accounting, terminal failure
  propagation, and repeated start/stop worker-ownership regression coverage.
- Debug WAV capture now rejects RIFF overflow and append-after-finish instead of
  silently producing misleading output.
- Jitter-buffer stop-time drain accounting counts skipped sequence holes and all
  bounded rejection classes reach playback diagnostics.

### Android threading and failure visibility

- Manual listener shared playback/session references use atomic publication.
- Discovered listener audio attaches transport -> playback entirely inside Rust;
  high-frequency audio no longer round-trips through Kotlin.
- Discovered control/diagnostics work runs off Android main, and start/stop joins
  are serialized by a ViewModel-owned playback-control executor.
- Manual disconnect/playback cleanup attempts every cleanup step and preserves
  the first failure with later failures suppressed instead of publishing an
  optimistic stopped/cancelled state.
- Manual packet-submit and sync-probe failures are visible rather than log-only.
- Attached transport -> playback failure propagates instead of incrementing a
  successful-forward counter.

### Android host self-monitor migration

Listener-playback migration Item 5.5 is now implemented:

- `MainViewModelHostPlayback` no longer uses `PlaybackEngine`/`PlaybackFrame`.
- Android host local monitoring opens `FfiListenerPlaybackHandle`, the same Rust
  scheduler/pump/render-ring runtime used for listener playback.
- The platform supplies one `SystemClock.elapsedRealtime()` sample and Rust owns
  the same-process host-to-runtime clock offset calculation.
- Host pause/resume reanchors the live Rust scheduler using the accumulated pause
  offset already owned by the host stream loop.
- Dynamic local volume updates the Rust pump gain; no Kotlin PCM conversion is
  used by the production host-monitor path.
- Normal host monitor start/stop/drain and packet submission run from IO
  dispatchers, not Android main.
- Intentional Stop is distinguished from a genuine runtime-submit failure so a
  cancellation race cannot turn a normal Stop into `PlaybackState.ERROR`.
- Legacy `PlaybackEngine`/`PlaybackFrame`/`OboePlaybackEngine` code remains only
  as isolated low-level regression surface, not a `MainViewModel` production
  playback owner.

### Desktop failure handling and regression closure

- Local monitor packet rejection/pump/backend failures are retained as terminal
  monitor failures and surface through status/teardown.
- Disabling the monitor cannot return success after poisoned-state or teardown
  failure.
- Playback shutdown aggregates monitor teardown failure with other cleanup.
- Packetizer shutdown while decode is active/backpressured and database shutdown
  with queued accepted work have explicit regressions.
- Host restart/source-boundary loopback regression proves fresh stream identity,
  sequence reset, and no previous-stream audio leak.
- Pure JVM clock-domain conversion regression and API-29 legacy Bluetooth
  manifest regression were added.

### Lab/performance/source audit reconciliation

- Existing Lab raw-wire serialization, virtual-clock reconnect delay, mid-run
  `setLinkFaults`, packet-hash/fault-decision recording, replay/assertions, and
  editable fault controls were re-audited and their stale implementation boxes
  reconciled.
- Block 45 matrix/instrumentation definition is complete; actual performance
  measurements and thresholds remain open until real artifacts exist.
- Block 48's audit now includes the authoritative desktop-host TODO itself.
- Stale post-file-split exact paths were repaired.
- Current Block 48 audit passes with 91 exact repository references and four
  ignored Rust tests, all carrying explicit reason/owner accountability.
- AppImage + `.deb` are the intentional initial Linux formats; no third initial
  package format is planned.

## Ledger state after software-only reconciliation

- `docs/SILENT_DISCO_LISTENER_PLAYBACK_RUST_MIGRATION_TODO.md`: zero unchecked
  items; its remaining device protocol is prose, not a software checkbox.
- `docs/SILENT_DISCO_CODE_REVIEW2_TODO.md`: only `./gradlew test` and
  `./gradlew lintDebug` remain unchecked because Gradle cannot run here.
- `docs/SILENT_DISCO_PLAYBACK_REVIEW_FIXES_TODO.md`: only physical/device runs
  and Rust/Android compiler gates remain unchecked.
- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`: remaining unchecked items are
  physical Android/audio-device acceptance, firewall/fresh-machine environment
  evidence, real performance/soak measurements, compiler/build/test execution,
  and final evidence-ledger assertions.
- Historical oversized-source and Rust-core migration planning ledgers were not
  mass-edited merely to erase old planning checkboxes. Their remaining cleanup
  work is explicitly gated on device validation or final toolchain acceptance;
  future-product items remain future scope.

## Dependency-free validation completed after the changes

Passed in the supplied sandbox:

```text
python3 scripts/audit-block48-completion.py
  -> 91 repository references; 4 ignored Rust tests with reason/owner
python3 -m py_compile scripts/audit-block48-completion.py
bash -n desktop/scripts/validate-block46-fresh-machine.sh
bash -n desktop/scripts/validate-block47-android-interoperability.sh
bash scripts/check-source-file-line-counts.sh
  -> 579 tracked source files; all below 800 physical lines
```

Additional targeted validation:

- standalone Kotlin compile of the new host-monitor wrapper against typed Android,
  protocol, and generated-UniFFI stubs: pass;
- earlier standalone Kotlin compile of the primitive Rust bridge: pass;
- earlier Android NSD adapter compile against framework stubs and parser runtime
  checks: pass;
- no production `MainViewModel` reference to `playbackEngine` or `PlaybackFrame`;
- no merge conflict markers in maintained source/docs;
- the accidental duplicate `ListenerPlaybackRuntime::debug_capture_error`
  declaration introduced during an interrupted WIP edit was found and removed.

These checks reduce source-level risk but do not replace Rust/Gradle/Node
compiler-backed gates.

## Canonical gates attempted after the latest changes

A Rust toolchain archive from the user's persistent file library was recovered
and validated as Rust/Cargo 1.95.0 with rustfmt and Clippy. The repository and
CI intentionally require Rust 1.97.1, so the recovered toolchain is a fallback
validation aid rather than authoritative MSRV evidence. It nevertheless exposed
real formatting drift in the unpublished WIP; the affected core and desktop Rust
sources were formatted, while an untouched file affected only by Rust 1.95
rustfmt recursion was restored byte-for-byte from current `master`.

Current formatter/source-integrity evidence:

```text
changed-file-only rustfmt --check (skip_children=true)
  -> 48 changed/new Rust files pass under Rust 1.95.0
git diff --check
  -> pass
bash scripts/check-source-file-line-counts.sh
  -> 579 tracked source files; all below 800 physical lines
```

The compiler/build gates remain **environment blocked, not passing and not
code-test failures**:

```text
bash scripts/check-rust.sh with recovered Rust 1.95.0
  -> not an authoritative final-tree format gate because Rust 1.95 rustfmt
     recursively reformats one untouched current-master scheduler test while
     CI pins Rust 1.97.1. During a temporary all-tree 1.95 formatting attempt,
     the next stage reached Cargo dependency resolution and then failed because
     index.crates.io could not be resolved, before Clippy/tests started. The
     unrelated 1.95-only formatter churn was then restored from current master.

./gradlew test lintDebug --stacktrace --console=plain
  -> cannot resolve services.gradle.org / Gradle distribution

cd desktop && npm ci --offline --ignore-scripts
  -> npm cache lacks required package tarballs (for example yargs-parser), so
     desktop npm/check cannot be populated or run offline
```

Do not mark Block 48.3 green from older `memory.md` runs because the current
software-hardening diff post-dates those runs. No compiler-backed pass is claimed
from the fallback Rust 1.95 formatter evidence.

## What remains and why it remains open

### Physical/device evidence

- two real Android listeners: join/approve/sync/play/pause/resume/stop;
- inter-listener skew measurement and listener diagnostics;
- Android scan/join and mDNS/QR physical acceptance;
- Android audio-device removal/route-change behavior;
- playback-review device reruns;
- full Block 47 packaged desktop + two-Android matrix.

### Host/environment evidence

- independent firewall inspection on the target host;
- Block 46.3 fresh graphical Ubuntu 22.04 package validation;
- fresh-machine create/stage/host/export/reopen/uninstall checks.

### Measured performance evidence

- CPU/RSS/throughput/queue/backlog/UI/delivery/sync/underrun/callback/shutdown/DB
  measurements;
- evidence-backed listener limit and diagnostic aggregation cadence;
- regression thresholds only where the collected distribution supports them.

### Compiler/build/test execution

Run in a normal development/CI environment and fix genuine failures from:

```bash
bash scripts/check-rust.sh
./gradlew test lintDebug --stacktrace --console=plain
cd desktop && npm ci && npm run check
```

Then run the complete Block 48.3 matrix, including Lab scenarios, loopback
transport integration, Linux bundle build, Android instrumentation, and Tauri
strict gates.

### Final closure bookkeeping

Only after the above evidence is real:

- record current artifact versions/SHAs, devices, topology, commands, results,
  measured synchronization, and limitations;
- write the final `memory.md` ledger;
- check top-level result assertions such as deterministic shutdown and Lab
  replay/assertion pass based on executed gates, not source inspection;
- close Blocks 46.3/47/48 only from recorded evidence.

## Workflow reminder

The user monitors GitHub Actions. Do not poll or monitor CI during Ralph Loops;
wait for user-reported failures and fix them from the supplied master snapshot.
