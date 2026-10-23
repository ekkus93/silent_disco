#!/usr/bin/env python3
"""Fail-closed metadata checks for production-path PCM callback CI artifacts.

No source audio or callback PCM is loaded. This validates only the small
JSON evidence files produced by the actual Rust socket/scheduler/C-ABI tests.
"""
import argparse
import json
import re
import sys
from pathlib import Path

REPORT_NAMES = frozenset({
    "mixed-rate-socket-callback.json",
    "encoded-socket-callback.json",
    "rejected-boundary-callback.json",
    "authoritative-edit-replay-callback.json",
})
SHA = re.compile(r"[0-9a-f]{40}\Z")


def require_equal(report, name, expected, filename):
    actual = report.get(name)
    if type(actual) is not type(expected) or actual != expected:
        raise ValueError(f"{filename}: {name} expected {expected!r}, got {actual!r}")


def require_integer(report, name, filename, minimum=0):
    actual = report.get(name)
    if type(actual) is not int or actual < minimum:
        raise ValueError(f"{filename}: {name} must be an integer >= {minimum}")
    return actual


def validate_report(filename, report, sha):
    if not isinstance(report, dict):
        raise ValueError(f"{filename}: root must be an object")
    require_equal(report, "schema", 1, filename)
    require_equal(report, "status", "PASS", filename)
    require_equal(report, "commit", sha, filename)
    require_equal(report, "physical_dac_verified", False, filename)

    if filename == "mixed-rate-socket-callback.json":
        require_equal(report, "source_boundaries", [480, 960], filename)
        require_equal(report, "callback_frame_chunks", [241, 337, 517, 345], filename)
        require_equal(report, "expected_frames", 1440, filename)
        require_equal(report, "captured_frames", 1440, filename)
        for name in (
            "mismatched_samples", "tap_mismatched_samples", "native_underrun_callbacks",
            "native_silence_filled_frames", "tap_lost_frames",
        ):
            require_equal(report, name, 0, filename)
        for name in ("first_mismatch_frame", "first_mismatch_channel"):
            require_equal(report, name, None, filename)
        require_equal(report, "tap_captured_frames", 1440, filename)
        require_equal(report, "callback_count", 4, filename)
    elif filename == "encoded-socket-callback.json":
        require_equal(report, "source_formats", ["wav", "flac", "mp3"], filename)
        require_equal(report, "listener_scheduler_exercised", True, filename)
        require_equal(report, "listener_volume_gains", [1.0, 0.5, 0.25], filename)
        boundaries = report.get("source_boundaries")
        if (not isinstance(boundaries, list) or len(boundaries) != 2
                or any(type(v) is not int or v <= 0 for v in boundaries)
                or boundaries[0] >= boundaries[1]):
            raise ValueError(f"{filename}: source boundaries must be increasing positive frames")
        require_equal(report, "observed_boundaries", boundaries, filename)
        frames = require_integer(report, "captured_frames", filename, minimum=1)
        require_equal(report, "expected_frames_with_padding", frames, filename)
        chunks = report.get("callback_frame_chunks")
        if (not isinstance(chunks, list) or len(chunks) != 3
                or any(type(v) is not int or v <= 0 for v in chunks)
                or sum(chunks) != frames):
            raise ValueError(f"{filename}: callback chunk sizes do not match output frame count")
        for name in (
            "mismatched_samples", "scheduler_mismatched_samples",
            "native_underrun_callbacks", "scheduler_underrun_callbacks",
            "native_silence_filled_frames", "scheduler_silence_filled_frames",
            "tap_lost_frames", "scheduler_tap_lost_frames",
        ):
            require_equal(report, name, 0, filename)
        for name in (
            "first_mismatch_frame", "first_mismatch_channel",
            "scheduler_first_mismatch_frame", "scheduler_first_mismatch_channel",
        ):
            require_equal(report, name, None, filename)
        for name in ("tap_captured_frames", "scheduler_tap_captured_frames"):
            require_equal(report, name, frames, filename)
        for name in ("callback_count", "scheduler_callback_count", "tap_callback_count",
                     "scheduler_tap_callback_count"):
            require_equal(report, name, 3, filename)
    elif filename == "authoritative-edit-replay-callback.json":
        require_equal(report, "capture", "actor-authorized-ffi-producer-native-callback", filename)
        require_equal(report, "source_boundaries", [9600, 10080], filename)
        for name in ("first_observed_boundaries", "replay_observed_boundaries"):
            require_equal(report, name, [9600, 10080], filename)
        require_equal(report, "expected_frames", 10560, filename)
        require_equal(report, "callback_frame_chunks", [4799, 4802, 480, 479], filename)
        for name in ("first_captured_frames", "replay_captured_frames",
                     "first_tap_frames", "replay_tap_frames"):
            require_equal(report, name, 10560, filename)
        for name in ("first_callback_count", "replay_callback_count",
                     "first_tap_callbacks", "replay_tap_callbacks"):
            require_equal(report, name, 4, filename)
        for name in ("first_native_underruns", "replay_native_underruns",
                     "first_native_silence_frames", "replay_native_silence_frames",
                     "first_tap_lost_frames", "replay_tap_lost_frames"):
            require_equal(report, name, 0, filename)
        require_equal(report, "retired_audio_packets_before_rejection", 2, filename)
        require_equal(report, "retired_successor_rejected", True, filename)
        require_equal(report, "replay_token_distinct", True, filename)
    elif filename == "rejected-boundary-callback.json":
        require_equal(report, "rejected_boundary_frame", 480, filename)
        require_equal(report, "valid_source_frames", 480, filename)
        require_equal(report, "captured_valid_frames", 480, filename)
        require_equal(report, "expected_tail_silence_frames", 17, filename)
        require_equal(report, "rejected_successor_audio_leaked", False, filename)
        for name in ("first_mismatch_frame", "first_mismatch_channel",
                     "tap_first_mismatch_frame"):
            require_equal(report, name, None, filename)
        require_equal(report, "native_callback_count", 2, filename)
        require_equal(report, "native_underrun_callbacks", 1, filename)
        require_equal(report, "native_silence_filled_frames", 17, filename)
        require_equal(report, "tap_captured_frames", 497, filename)
        require_equal(report, "tap_callback_count", 2, filename)
        require_equal(report, "tap_lost_frames", 0, filename)
    else:
        raise ValueError(f"unexpected PCM evidence name: {filename}")


def validate_directory(directory, sha):
    if not SHA.fullmatch(sha):
        raise ValueError("commit SHA must be an exact 40-character lowercase hexadecimal string")
    if not directory.is_dir():
        raise ValueError(f"PCM evidence directory missing: {directory}")
    actual = {entry.name for entry in directory.iterdir()}
    if actual != REPORT_NAMES:
        raise ValueError(
            f"PCM evidence must contain exactly {sorted(REPORT_NAMES)}; "
            f"missing={sorted(REPORT_NAMES - actual)}, unexpected={sorted(actual - REPORT_NAMES)}"
        )
    for filename in sorted(REPORT_NAMES):
        path = directory / filename
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"{filename}: evidence must be an ordinary JSON file")
        # These are diagnostic metadata only, never PCM samples or user music.
        if path.stat().st_size > 32_768:
            raise ValueError(f"{filename}: JSON evidence exceeds 32 KiB")
        with path.open(encoding="utf-8") as stream:
            report = json.load(stream, parse_constant=lambda value: (_ for _ in ()).throw(
                ValueError(f"{filename}: non-finite JSON constant {value}")
            ))
        validate_report(filename, report, sha)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--commit-sha", required=True)
    args = parser.parse_args(argv)
    try:
        validate_directory(args.directory, args.commit_sha)
    except (ValueError, OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"PCM_EVIDENCE_FAIL: {error}", file=sys.stderr)
        return 1
    print("PCM_EVIDENCE_PASS: exact-SHA production callback comparison reports validated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
