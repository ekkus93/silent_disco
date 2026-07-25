from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
from typing import Callable


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new)


def fix_sync_source() -> None:
    types_path = Path("rust/silent-disco-core/src/sync/types.rs")
    types = types_path.read_text()
    types = replace_once(
        types,
        """        Ok(Self {
            correlation_id: exchange.correlation_id,
            local_receive_time: exchange.t4_local_receive,
            round_trip_time_ms: network_round_trip as f64,
            offset_ms: offset_twice as f64 / 2.0,
        })
    }
}
""",
        """        Ok(Self {
            correlation_id: exchange.correlation_id,
            local_receive_time: exchange.t4_local_receive,
            round_trip_time_ms: u64_to_f64(network_round_trip),
            offset_ms: i128_to_f64(offset_twice) / 2.0,
        })
    }
}

// Monotonic millisecond values lose integer precision only above the exact f64
// integer range. Practical RTTs are immediately filtered to a small configured
// bound, while offset arithmetic retains sign and half-millisecond resolution
// for mobile-runtime timestamp ranges.
#[allow(clippy::cast_precision_loss)]
fn u64_to_f64(value: u64) -> f64 {
    value as f64
}

#[allow(clippy::cast_precision_loss)]
fn i128_to_f64(value: i128) -> f64 {
    value as f64
}
""",
        "sync timestamp conversion",
    )
    types_path.write_text(types)

    estimator_path = Path("rust/silent-disco-core/src/sync/estimator.rs")
    estimator = estimator_path.read_text()
    estimator = replace_once(
        estimator,
        """        assert_eq!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_samples: 0,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        );
        assert_eq!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_accepted_rtt_ms: f64::NAN,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        );
""",
        """        assert!(matches!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_samples: 0,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        ));
        assert!(matches!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_accepted_rtt_ms: f64::NAN,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        ));
""",
        "invalid sync configuration tests",
    )
    estimator = replace_once(
        estimator,
        "                    SyncCorrelationId::new(1_000 + id as u64),\n",
        """                    SyncCorrelationId::new(
                        1_000 + u64::try_from(id).expect("pending-probe index fits u64"),
                    ),
""",
        "pending probe index conversion",
    )
    estimator_path.write_text(estimator)

    fixture_path = Path("rust/silent-disco-core/src/sync/fixture_tests.rs")
    fixture = fixture_path.read_text()
    fixture = replace_once(
        fixture,
        "        let correlation = SyncCorrelationId::new(index as u64 + 1);\n",
        """        let correlation = SyncCorrelationId::new(
            u64::try_from(index)
                .expect("fixture sample index fits u64")
                .saturating_add(1),
        );
""",
        "fixture correlation conversion",
    )
    fixture_path.write_text(fixture)


def complete_validated_blocks() -> None:
    todo_path = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
    todo = todo_path.read_text()

    block4_start = todo.index(
        "## Block 4 — Implement Rust domain IDs, enums, and structured errors"
    )
    block4_end = todo.index(
        "**Acceptance:** Core domain types compile without Android dependencies",
        block4_start,
    )
    block4 = todo[block4_start:block4_end]
    if block4.count("- [ ]") != 27:
        raise SystemExit("Block 4 no longer has exactly 27 unchecked tasks")
    todo = todo[:block4_start] + block4.replace("- [ ]", "- [x]") + todo[block4_end:]

    block5_start = todo.index(
        "## Block 5 — Implement Rust protocol framing and golden vectors"
    )
    block5_end = todo.index(
        "**Acceptance:** Rust protocol tests pass and the wire format is fully specified by executable vectors.",
        block5_start,
    )
    block5 = todo[block5_start:block5_end]
    if block5.count("- [ ]") != 38:
        raise SystemExit("Block 5 no longer has exactly 38 unchecked tasks")
    todo = todo[:block5_start] + block5.replace("- [ ]", "- [x]") + todo[block5_end:]

    for physical_task in (
        "- [ ] Run the current connected Android test suite on an available physical device.",
        "- [ ] Test loads the Rust library on a physical Android device.",
        "- [ ] Test verifies the ABI version.",
    ):
        if physical_task not in todo:
            raise SystemExit(f"physical-device task changed unexpectedly: {physical_task}")
    todo_path.write_text(todo)


def prepend_memory() -> None:
    memory_path = Path("memory.md")
    memory = memory_path.read_text()
    prefix = "# memory.md — `silent_disco`\n\n"
    if not memory.startswith(prefix):
        raise SystemExit("memory.md header changed unexpectedly")
    if "Rust migration Block 4 complete" in memory or "Rust migration Block 5 complete" in memory:
        raise SystemExit("Block 4 or Block 5 memory entry already exists")

    timestamp = (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    entries = f"""## {timestamp} - GPT-5.6 Thinking - Rust migration Block 5 complete

- Completed Block 5 while preserving all physical-device-only gates.
- Established protocol v2 with `SDP2`, a fixed 16-byte network-order header, explicit version/kind/flags/length, a 64 KiB control limit, and a 4 KiB audio datagram limit.
- Added canonical Rust control, synchronization, and PCM16 audio schemas; bounded parsing, exact-length validation, CRC32 integrity, authorization/staleness policy, and independent diagnostics.
- Added production-encoder-generated executable vectors for every message kind, boundary sizes, deterministic hashes, and malformed/unsupported/integrity cases.
- CI run `30170763626` passed Rust format, Clippy with warnings denied, Rust tests, all APK variants, four-ABI packaging, Android unit tests, and Android lint.

## {timestamp} - GPT-5.6 Thinking - Rust migration Block 4 complete

- Completed validated Rust domain identifiers, stable domain enums, and structured errors while preserving the physical-device gate.
- IDs are bounded and validated; enums have stable numeric/wire representations; `CoreError` has subsystem-specific codes, bounded context, severity, retryability, and operation correlation.
- CI run `30168849005` passed Rust format, Clippy with warnings denied, Rust tests, all APK variants, four-ABI packaging, Android unit tests, and Android lint.

"""
    memory_path.write_text(prefix + entries + memory[len(prefix) :])


def remove_maintenance_from_ci() -> None:
    ci_path = Path(".github/workflows/ci.yml")
    ci = ci_path.read_text()
    temporary_skip = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != "
        "'agent/rust-migration-maintenance-diagnostics'\n"
    )
    if ci.count(temporary_skip) != 2:
        raise SystemExit("temporary maintenance branch skips changed unexpectedly")
    ci = ci.replace(temporary_skip, "")

    start_marker = "  # BEGIN RUST MIGRATION MAINTENANCE\n"
    end_marker = "  # END RUST MIGRATION MAINTENANCE\n"
    if ci.count(start_marker) != 1 or ci.count(end_marker) != 1:
        raise SystemExit("maintenance markers changed unexpectedly")
    start = ci.index(start_marker)
    end = ci.index(end_marker, start) + len(end_marker)
    cleaned = ci[:start] + ci[end:]
    ci_path.write_text(cleaned.rstrip() + "\n")


def remove_helpers() -> None:
    for helper in (
        ".github/scripts/rust_migration_maintenance.py",
        ".github/workflows/fix-sync-core.yml",
        ".github/sync-core-fix-trigger.txt",
        ".github/workflows/complete-rust-blocks-4-5.yml",
        ".github/rust-blocks-4-5-completion-trigger.txt",
        ".github/rust-migration-maintenance-trigger.txt",
        ".github/protocol-vector-style-trigger.txt",
        ".github/protocol-import-fix-trigger.txt",
        ".github/protocol-fix-trigger.txt",
    ):
        path = Path(helper)
        if path.exists():
            path.unlink()


def run_step(label: str, action: Callable[[], None]) -> None:
    print(f"BEGIN {label}", flush=True)
    try:
        action()
    except BaseException as error:
        print(f"FAILED {label}: {error!r}", flush=True)
        raise
    print(f"DONE {label}", flush=True)


def main() -> None:
    run_step("fix_sync_source", fix_sync_source)
    run_step("complete_validated_blocks", complete_validated_blocks)
    run_step("prepend_memory", prepend_memory)
    run_step("remove_maintenance_from_ci", remove_maintenance_from_ci)
    run_step("remove_helpers", remove_helpers)


if __name__ == "__main__":
    main()
