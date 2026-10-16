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


def boundaries_for(
    source: dict[str, Any],
    label: str,
    packets: list[tuple[int, int, int]],
    field: str = "boundaries",
) -> list[tuple[str, int]]:
    raw = require_array(source.get(field), f"{label}.{field}")
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


def unchanged_identity(
    source: dict[str, Any],
    label: str,
    stem: str,
    required: bool,
    stream_id: str,
) -> None:
    before_key, after_key = f"{stem}_before", f"{stem}_after"
    present = before_key in source or after_key in source
    if not required and not present:
        return
    before = nonempty(source.get(before_key), f"{label}.{before_key}")
    after = nonempty(source.get(after_key), f"{label}.{after_key}")
    if before != after:
        raise InvalidEvidence(f"{label}: {stem} changed during the captured transition")
    if stem == "stream_id" and before != stream_id:
        raise InvalidEvidence(f"{label}: stream_id differs from the captured host stream")


def check_capture(
    document: Any,
    *,
    require_golden_boundaries: bool = False,
    require_sink_identity: bool = False,
) -> dict[str, int]:
    root = require_object(document, "capture")
    if root.get("schema_version") != 1 or type(root.get("schema_version")) is not int:
        raise InvalidEvidence("schema_version must equal 1")
    nonempty(root.get("test_case"), "test_case")
    stream_id = nonempty(root.get("stream_id"), "stream_id")
    positive(root.get("sample_rate_hz"), "sample_rate_hz")
    host = require_object(root.get("host"), "host")
    host_packets = packets_for(host, "host")
    boundaries = boundaries_for(host, "host", host_packets)
    if require_golden_boundaries or "expected_boundaries" in root:
        # These must come from an independent decoded-source reference or the
        # actor-approved queue revision, not be copied from observed messages.
        expected = boundaries_for(root, "capture", host_packets, "expected_boundaries")
        if boundaries != expected:
            raise InvalidEvidence("host: boundaries differ from independent expected boundaries")
    if "forbidden_entry_ids" in root:
        forbidden = root["forbidden_entry_ids"]
        if not isinstance(forbidden, list):
            raise InvalidEvidence("forbidden_entry_ids must be an array")
        forbidden_ids = {nonempty(item, "forbidden_entry_ids[]") for item in forbidden}
        if any(entry_id in forbidden_ids for entry_id, _ in boundaries):
            raise InvalidEvidence("host: removed or forbidden track played after a queue edit")
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
        for stem in ("output_device_id", "output_runtime_id", "stream_id"):
            unchanged_identity(listener, label, stem, require_sink_identity, stream_id)
        if positive(listener.get("output_opens_before"), f"{label}.output_opens_before") < 1:
            raise InvalidEvidence(f"{label}: no active output stream")
        for field in ("missing_boundary_frames", "duplicate_boundary_frames"):
            if nonnegative(listener.get(field), f"{label}.{field}") != 0:
                raise InvalidEvidence(f"{label}: {field} is nonzero")
    return {"listeners": len(listeners), "boundaries": len(boundaries), "packets": len(host_packets)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path, help="canonical gapless playback JSON trace")
    parser.add_argument(
        "--require-golden-boundaries", action="store_true",
        help="require independently expected track identities and boundary sample indices",
    )
    parser.add_argument(
        "--require-sink-identity", action="store_true",
        help="require matching before/after output device, runtime, and stream identities",
    )
    args = parser.parse_args(argv)
    try:
        document = json.loads(args.capture.read_text(encoding="utf-8"))
        counts = check_capture(
            document,
            require_golden_boundaries=args.require_golden_boundaries,
            require_sink_identity=args.require_sink_identity,
        )
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
