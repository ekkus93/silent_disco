from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new)


def update_todo() -> None:
    path = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
    text = path.read_text()
    old = """## Block 6 — Port clock synchronization to Rust

### 6.1 Implement monotonic time types

- [ ] Separate host and local monotonic timestamp types where helpful.
- [ ] Reject impossible timestamp orderings.
- [ ] Avoid wall-clock time in scheduling calculations.

### 6.2 Port estimator behavior

Implement and test:

- [ ] four-timestamp offset calculation;
- [ ] RTT calculation;
- [ ] correlation-ID matching;
- [ ] sample history window;
- [ ] outlier rejection;
- [ ] low-RTT preference;
- [ ] confidence classification;
- [ ] drift threshold;
- [ ] initial-sync and periodic-sync decisions.

### 6.3 Verify Kotlin compatibility fixtures

- [ ] Run Kotlin baseline fixtures through Rust.
- [ ] Differences require either a Rust fix or an explicitly documented intentional behavior change in `memory.md`.
- [ ] Add edge cases for overflow, duplicate response, stale correlation ID, high RTT, and negative/invalid ordering.

### 6.4 Expose pure FFI sync smoke API

- [ ] Add temporary or permanent UniFFI-friendly sync records.
- [ ] Kotlin test invokes Rust estimator with a fixture.
- [ ] Do not maintain a new Kotlin estimator wrapper that duplicates calculations.

**Acceptance:** Rust produces approved sync results and all estimator tests pass on host and Android.
"""
    new = """## Block 6 — Port clock synchronization to Rust

### 6.1 Implement monotonic time types

- [x] Separate host and local monotonic timestamp types where helpful.
- [x] Reject impossible timestamp orderings.
- [x] Avoid wall-clock time in scheduling calculations.

### 6.2 Port estimator behavior

Implement and test:

- [x] four-timestamp offset calculation;
- [x] RTT calculation;
- [x] correlation-ID matching;
- [x] sample history window;
- [x] outlier rejection;
- [x] low-RTT preference;
- [x] confidence classification;
- [x] drift threshold;
- [x] initial-sync and periodic-sync decisions.

### 6.3 Verify Kotlin compatibility fixtures

- [x] Run Kotlin baseline fixtures through Rust.
- [x] Differences require either a Rust fix or an explicitly documented intentional behavior change in `memory.md`.
- [x] Add edge cases for overflow, duplicate response, stale correlation ID, high RTT, and negative/invalid ordering.

### 6.4 Expose pure FFI sync smoke API

- [x] Add temporary or permanent UniFFI-friendly sync records.
- [x] Kotlin test invokes Rust estimator with a fixture.
- [x] Do not maintain a new Kotlin estimator wrapper that duplicates calculations.

**Acceptance:** Rust produces approved sync results and all estimator tests pass on host and Android.

**Physical-device status:** Host tests, JVM tests, Android builds, four-ABI packaging, and compilation of the Android instrumentation-test APK pass. Execution of `RustSyncEstimatorInstrumentedTest` on a physical Android device is **NOT RUN**; Block 6 physical-Android acceptance remains open until that test is executed and recorded.
"""
    path.write_text(replace_once(text, old, new, "Block 6 TODO section"))


def update_memory() -> None:
    path = Path("memory.md")
    text = path.read_text()
    prefix = "# memory.md — `silent_disco`\n\n"
    if not text.startswith(prefix):
        raise SystemExit("memory.md header changed unexpectedly")
    if "Rust synchronization Block 6 code complete" in text:
        raise SystemExit("Block 6 memory entry already exists")
    timestamp = (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    entry = f"""## {timestamp} - GPT-5.6 Thinking - Rust synchronization Block 6 code complete

- Ported clock synchronization to Rust with distinct host/local monotonic timestamp types, checked four-timestamp RTT/offset arithmetic, bounded correlation tracking, bounded sample/drift history, low-RTT selection, confidence classification, skew estimation, and initial/periodic/drift decisions.
- Added tests for near-`u64` arithmetic, impossible orderings, duplicate and stale responses, mismatched echoed timestamps, pending-probe capacity, high-RTT rejection, history bounds, confidence thresholds, and decision behavior.
- Added binding-friendly Rust synchronization records and static JNI exports with bounded positive handles, stable explicit error statuses, collision-safe registry insertion, explicit destruction, and no JNI pointer dereferences.
- Added a synchronized Kotlin bridge that consumes every native status immediately and performs no estimator calculations. Non-finite values, unknown confidence codes, invalid handles, load/link failures, and impossible timestamps fail visibly.
- Added `RustSyncEstimatorInstrumentedTest`, which loads the existing Kotlin compatibility JSON fixture and invokes the Rust estimator. Permanent CI now compiles/packages the instrumentation-test APK.
- PR #24 merged as `929ec82a24e6e817e0e9a6a40c07558739b9222a`. Pre-merge CI run `30174145493` passed Rust formatting, Clippy with warnings denied, all Rust tests, debug/PoC-debug/release APKs, instrumentation-test APK compilation, four-ABI Rust packaging, Android unit tests, and Android lint.
- Physical execution of `RustSyncEstimatorInstrumentedTest` was **NOT RUN** because no physical Android device is attached. Block 6 physical-Android acceptance remains open; do not claim device validation until the command and device details are recorded.

"""
    path.write_text(prefix + entry + text[len(prefix) :])


def restore_ci_and_remove_script() -> None:
    ci_path = Path(".github/workflows/ci.yml")
    text = ci_path.read_text()
    temporary_skip = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != 'docs/rust-sync-block6-record'\n"
    )
    if text.count(temporary_skip) != 2:
        raise SystemExit("temporary docs branch skips changed unexpectedly")
    text = text.replace(temporary_skip, "")
    start_marker = "  # BEGIN BLOCK 6 DOCUMENTATION JOB\n"
    end_marker = "  # END BLOCK 6 DOCUMENTATION JOB\n"
    if text.count(start_marker) != 1 or text.count(end_marker) != 1:
        raise SystemExit("Block 6 documentation job markers changed unexpectedly")
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    ci_path.write_text((text[:start] + text[end:]).rstrip() + "\n")
    Path(".github/scripts/complete_sync_block6.py").unlink()


def main() -> None:
    update_todo()
    update_memory()
    restore_ci_and_remove_script()


if __name__ == "__main__":
    main()
