from __future__ import annotations

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new)


def fix_types_tests() -> None:
    path = Path("rust/silent-disco-core/src/sync/types.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    use super::{
        HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId, SyncExchange, SyncSample,
        SyncTimestampError,
    };

    fn exchange(t1: u64, t2: u64, t3: u64, t4: u64) -> SyncExchange {
""",
        """    use super::{
        HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId, SyncExchange, SyncSample,
        SyncTimestampError,
    };

    const FLOAT_TOLERANCE: f64 = 1.0e-9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= FLOAT_TOLERANCE,
            "expected {actual} to be within {FLOAT_TOLERANCE} of {expected}"
        );
    }

    fn exchange(t1: u64, t2: u64, t3: u64, t4: u64) -> SyncExchange {
""",
        "types test tolerance helper",
    )
    for old, new, label in (
        (
            "        assert_eq!(sample.round_trip_time_ms, 20.0);\n",
            "        assert_close(sample.round_trip_time_ms, 20.0);\n",
            "types RTT 20 assertion",
        ),
        (
            "        assert_eq!(sample.offset_ms, 5.0);\n",
            "        assert_close(sample.offset_ms, 5.0);\n",
            "types offset 5 assertion",
        ),
        (
            "        assert_eq!(sample.round_trip_time_ms, 399.0);\n",
            "        assert_close(sample.round_trip_time_ms, 399.0);\n",
            "types RTT 399 assertion",
        ),
        (
            "        assert_eq!(sample.offset_ms, -99.5);\n",
            "        assert_close(sample.offset_ms, -99.5);\n",
            "types offset -99.5 assertion",
        ),
        (
            "        assert_eq!(sample.round_trip_time_ms, 19.0);\n",
            "        assert_close(sample.round_trip_time_ms, 19.0);\n",
            "types RTT 19 assertion",
        ),
        (
            "        assert_eq!(sample.offset_ms, 0.5);\n",
            "        assert_close(sample.offset_ms, 0.5);\n",
            "types offset 0.5 assertion",
        ),
    ):
        text = replace_once(text, old, new, label)
    path.write_text(text)


def fix_estimator_tests() -> None:
    path = Path("rust/silent-disco-core/src/sync/estimator.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    use crate::{
        domain::SyncConfidence,
        sync::{HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId},
    };

    fn observe(
""",
        """    use crate::{
        domain::SyncConfidence,
        sync::{HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId},
    };

    const FLOAT_TOLERANCE: f64 = 1.0e-9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= FLOAT_TOLERANCE,
            "expected {actual} to be within {FLOAT_TOLERANCE} of {expected}"
        );
    }

    fn observe(
""",
        "estimator test tolerance helper",
    )
    for old, new, label in (
        (
            "        assert_eq!(snapshot.offset_ms, 5.0);\n",
            "        assert_close(snapshot.offset_ms, 5.0);\n",
            "estimator offset assertion",
        ),
        (
            "        assert_eq!(snapshot.round_trip_time_ms, 20.0);\n",
            "        assert_close(snapshot.round_trip_time_ms, 20.0);\n",
            "estimator RTT assertion",
        ),
        (
            "        assert_eq!(snapshot.jitter_ms, 0.0);\n",
            "        assert_close(snapshot.jitter_ms, 0.0);\n",
            "estimator jitter assertion",
        ),
    ):
        text = replace_once(text, old, new, label)
    path.write_text(text)


def fix_fixture_tests() -> None:
    path = Path("rust/silent-disco-core/src/sync/fixture_tests.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """struct FixtureSample {
    t1: u64,
    t2: u64,
    t3: u64,
    t4: u64,
    expected_rtt_ms: f64,
    expected_offset_ms: f64,
    accepted: bool,
}

#[test]
""",
        """struct FixtureSample {
    t1: u64,
    t2: u64,
    t3: u64,
    t4: u64,
    expected_rtt_ms: f64,
    expected_offset_ms: f64,
    accepted: bool,
}

const FLOAT_TOLERANCE: f64 = 1.0e-9;

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= FLOAT_TOLERANCE,
        "expected {actual} to be within {FLOAT_TOLERANCE} of {expected}"
    );
}

#[test]
""",
        "fixture test tolerance helper",
    )
    text = replace_once(
        text,
        """        assert_eq!(
            observation.sample.round_trip_time_ms,
            fixture.expected_rtt_ms
        );
""",
        "        assert_close(observation.sample.round_trip_time_ms, fixture.expected_rtt_ms);\n",
        "fixture RTT assertion",
    )
    for old, new, label in (
        (
            "        assert_eq!(observation.sample.offset_ms, fixture.expected_offset_ms);\n",
            "        assert_close(observation.sample.offset_ms, fixture.expected_offset_ms);\n",
            "fixture offset assertion",
        ),
        (
            "    assert_eq!(snapshot.offset_ms, parse_f64(expected, \"offsetMs\"));\n",
            "    assert_close(snapshot.offset_ms, parse_f64(expected, \"offsetMs\"));\n",
            "fixture final offset assertion",
        ),
        (
            "    assert_eq!(snapshot.round_trip_time_ms, parse_f64(expected, \"rttMs\"));\n",
            "    assert_close(snapshot.round_trip_time_ms, parse_f64(expected, \"rttMs\"));\n",
            "fixture final RTT assertion",
        ),
        (
            "    assert_eq!(snapshot.jitter_ms, parse_f64(expected, \"jitterMs\"));\n",
            "    assert_close(snapshot.jitter_ms, parse_f64(expected, \"jitterMs\"));\n",
            "fixture final jitter assertion",
        ),
    ):
        text = replace_once(text, old, new, label)
    path.write_text(text)


def restore_ci() -> None:
    path = Path(".github/workflows/ci.yml")
    text = path.read_text()
    temporary_skip = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != 'agent/rust-sync-float-assertion-fix'\n"
    )
    if text.count(temporary_skip) != 2:
        raise SystemExit("temporary float-assertion branch skips changed unexpectedly")
    text = text.replace(temporary_skip, "")
    start_marker = "  # BEGIN SYNC FLOAT ASSERTION FIX\n"
    end_marker = "  # END SYNC FLOAT ASSERTION FIX\n"
    if text.count(start_marker) != 1 or text.count(end_marker) != 1:
        raise SystemExit("float-assertion maintenance markers changed unexpectedly")
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    path.write_text((text[:start] + text[end:]).rstrip() + "\n")


def remove_helper() -> None:
    Path(".github/scripts/fix_sync_float_assertions.py").unlink()


def main() -> None:
    fix_types_tests()
    fix_estimator_tests()
    fix_fixture_tests()
    restore_ci()
    remove_helper()


if __name__ == "__main__":
    main()
