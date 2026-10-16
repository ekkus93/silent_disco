#!/usr/bin/env python3
"""Check sample/boundary continuity in *collected* gapless playback traces.

This is a strict evidence preflight, NOT proof of physical acoustic gaplessness:
it cannot infer actual speaker output or certify that a device was used.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


class InvalidEvidence(ValueError):
    """A trace is malformed or contradicts a required continuity invariant."""


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise InvalidEvidence(f"{label} must be an object")
    return value


def require_array(value: Any, label: str) -> list[Any]:
    if not isinstance(value, list) or not value:
        raise InvalidEvidence(f"{label} must be a nonempty array")
    return value


def nonnegative(value: Any, label: str) -> int:
    if type(value) is not int or value < 0:
        raise InvalidEvidence(f"{label} must be a nonnegative integer")
    return value


def positive(value: Any, label: str) -> int:
    number = nonnegative(value, label)
    if number == 0:
        raise InvalidEvidence(f"{label} must be greater than zero")
    return number


def nonempty(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise InvalidEvidence(f"{label} must be a nonempty string")
    return value


def packets_for(source: dict[str, Any], label: str) -> list[tuple[int, int, int]]:
    raw = require_array(source.get("packets"), f"{label}.packets")
    packets: list[tuple[int, int, int]] = []
    for index, item in enumerate(raw):
        packet = require_object(item, f"{label}.packets[{index}]")
        sequence = nonnegative(packet.get("sequence"), f"{label}.packets[{index}].sequence")
        first = nonnegative(
            packet.get("first_sample_index"), f"{label}.packets[{index}].first_sample_index"
        )
        frames = positive(packet.get("frame_count"), f"{label}.packets[{index}].frame_count")
        if packets:
            previous_sequence, previous_first, previous_frames = packets[-1]
            if sequence != previous_sequence + 1:
                raise InvalidEvidence(f"{label}: missing, duplicated, or reordered packet sequence")
            if first != previous_first + previous_frames:
                raise InvalidEvidence(f"{label}: missing or overlapping PCM frames")
        packets.append((sequence, first, frames))
    return packets


def boundaries_for(source: dict[str, Any], label: str, packets: list[tuple[int, int, int]]) -> list[tuple[str, int]]:
    raw = require_array(source.get("boundaries"), f"{label}.boundaries")
    boundaries: list[tuple[str, int]] = []
    start = packets[0][1]
    end = packets[-1][1] + packets[-1][2]
    for index, item in enumerate(raw):
        boundary = require_object(item, f"{label}.boundaries[{index}]")
        entry_id = nonempty(boundary.get("entry_id"), f"{label}.boundaries[{index}].entry_id")
        sample = positive(boundary.get("first_sample_index"), f"{label}.boundaries[{index}].first_sample_index")
        if sample <= start or sample >= end:
            raise InvalidEvidence(f"{label}: boundary is not strictly inside captured audio")
        if boundaries and sample <= boundaries[-1][1]:
            raise InvalidEvidence(f"{label}: boundaries are not in increasing sample order")
        boundaries.append((entry_id, sample))
    return boundaries


def check_boundary_counters(source: dict[str, Any], label: str, count: int) -> None:
    # Capture windows must start before the first boundary and end after the last.
    for field in ("boundary_scheduled", "boundary_applied"):
        value = nonnegative(source.get(field), f"{label}.{field}")
        if value < count:
            raise InvalidEvidence(f"{label}.{field} is less than captured boundaries")


def unchanged_counter(source: dict[str, Any], label: str, stem: str) -> None:
    before = nonnegative(source.get(f"{stem}_before"), f"{label}.{stem}_before")
    after = nonnegative(source.get(f"{stem}_after"), f"{label}.{stem}_after")
    if after != before:
        raise InvalidEvidence(f"{label}: {stem} changed during the captured transition")


def check_capture(document: Any) -> dict[str, int]:
    root = require_object(document, "capture")
    if root.get("schema_version") != 1 or type(root.get("schema_version")) is not int:
        raise InvalidEvidence("schema_version must equal 1")
    nonempty(root.get("test_case"), "test_case")
    nonempty(root.get("stream_id"), "stream_id")
    positive(root.get("sample_rate_hz"), "sample_rate_hz")
    host = require_object(root.get("host"), "host")
    host_packets = packets_for(host, "host")
    boundaries = boundaries_for(host, "host", host_packets)
    check_boundary_counters(host, "host", len(boundaries))
    if nonnegative(host.get("boundary_discontinuities"), "host.boundary_discontinuities") != 0:
        raise InvalidEvidence("host: boundary discontinuities are nonzero")

    listeners = require_array(root.get("listeners"), "listeners")
    seen: set[str] = set()
    for index, item in enumerate(listeners):
        label = f"listeners[{index}]"
        listener = require_object(item, label)
        listener_id = nonempty(listener.get("id"), f"{label}.id")
        if listener_id in seen:
            raise InvalidEvidence(f"duplicate listener id: {listener_id}")
        seen.add(listener_id)
        if packets_for(listener, label) != host_packets:
            raise InvalidEvidence(f"{label}: packet sequence/sample window differs from host")
        if boundaries_for(listener, label, host_packets) != boundaries:
            raise InvalidEvidence(f"{label}: boundary identity or sample index differs from host")
        check_boundary_counters(listener, label, len(boundaries))
        for stem in ("output_opens", "output_closes", "concealed_frames"):
            unchanged_counter(listener, label, stem)
        if positive(listener.get("output_opens_before"), f"{label}.output_opens_before") < 1:
            raise InvalidEvidence(f"{label}: no active output stream")
        for field in ("missing_boundary_frames", "duplicate_boundary_frames"):
            if nonnegative(listener.get(field), f"{label}.{field}") != 0:
                raise InvalidEvidence(f"{label}: {field} is nonzero")
    return {"listeners": len(listeners), "boundaries": len(boundaries), "packets": len(host_packets)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path, help="canonical gapless playback JSON trace")
    args = parser.parse_args(argv)
    try:
        document = json.loads(args.capture.read_text(encoding="utf-8"))
        counts = check_capture(document)
    except (OSError, UnicodeError, json.JSONDecodeError, InvalidEvidence) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(
        f"PASS: {counts['packets']} captured packets, "
        f"{counts['boundaries']} boundaries, {counts['listeners']} listeners; "
        "trace invariants only (physical playback is NOT qualified)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
