#!/usr/bin/env python3
"""Compare listener PCM16 WAV capture with consecutive canonical source WAVs.

Inputs must already use the same decoded sample rate/channel layout. This tool
never hides underruns by time-stretching, resampling, or automatic alignment.
"""
from __future__ import annotations

import argparse
from itertools import zip_longest
import json
import math
import re
from pathlib import Path
import struct
import sys
import wave

READ_FRAMES = 4096
MISMATCH_EXAMPLES = 8


def _wav_info(path: Path) -> dict[str, int]:
    # The wave reader stops at the declared RIFF/data lengths. A callback
    # capture with extra raw samples appended after a stale RIFF header could
    # otherwise compare PASS while silently ignoring audible tail frames.
    with path.open("rb") as source:
        header = source.read(12)
    if len(header) != 12 or header[:4] != b"RIFF" or header[8:] != b"WAVE":
        raise ValueError(f"{path}: expected RIFF/WAVE header")
    riff_end = struct.unpack_from("<I", header, 4)[0] + 8
    physical_size = path.stat().st_size
    if physical_size > riff_end:
        raise ValueError(f"{path}: unaccounted trailing bytes outside RIFF payload")
    # The wave reader consumes only the first "data" chunk and ignores
    # missing trailing RIFF metadata. Inspect every chunk even if the file is
    # short. Only a genuinely truncated *audio payload* may proceed to the
    # frame comparator, which reports missing frames rather than a false PASS.
    with path.open("rb") as source:
        offset = 12
        audio_chunks = 0
        truncated_audio = False
        while offset < physical_size:
            source.seek(offset)
            chunk_header = source.read(8)
            if len(chunk_header) != 8:
                raise ValueError(f"{path}: incomplete RIFF chunk header")
            chunk_size = struct.unpack_from("<I", chunk_header, 4)[0]
            payload_end = offset + 8 + chunk_size
            next_offset = payload_end + (chunk_size & 1)
            if next_offset > riff_end:
                raise ValueError(f"{path}: RIFF chunk exceeds declared payload")
            is_audio = chunk_header[:4] == b"data"
            if is_audio:
                audio_chunks += 1
            if next_offset > physical_size:
                if not is_audio or physical_size >= payload_end:
                    raise ValueError(f"{path}: truncated RIFF metadata or chunk padding")
                truncated_audio = True
                break
            offset = next_offset
        if audio_chunks != 1:
            raise ValueError(f"{path}: expected exactly one PCM data chunk")
        if physical_size < riff_end and not truncated_audio:
            raise ValueError(f"{path}: truncated RIFF payload after PCM data")
    with wave.open(str(path), "rb") as wav:
        if wav.getcomptype() != "NONE" or wav.getsampwidth() != 2:
            raise ValueError(f"{path}: expected uncompressed 16-bit PCM WAV")
        if wav.getnchannels() not in (1, 2) or wav.getframerate() <= 0:
            raise ValueError(f"{path}: unsupported channels or sample rate")
        return {
            "channels": wav.getnchannels(),
            "sample_rate": wav.getframerate(),
            "frames": wav.getnframes(),
        }


def _frames(path: Path, channels: int):
    """Yield bounded batches rather than allocating an entire track."""
    with wave.open(str(path), "rb") as wav:
        while data := wav.readframes(READ_FRAMES):
            if len(data) % (2 * channels):
                raise ValueError(f"{path}: incomplete interleaved PCM frame")
            yield from struct.iter_unpack(f"<{channels}h", data)


def compare(
    expected_paths: list[Path],
    captured_path: Path,
    *,
    tolerance_lsb: int = 0,
    boundary_window_frames: int = 480,
    commit_sha: str | None = None,
) -> dict:
    """Compare full ordered content, exact length and each track boundary."""
    if not expected_paths:
        raise ValueError("at least one expected track is required")
    if not 0 <= tolerance_lsb <= 32767 or boundary_window_frames < 1:
        raise ValueError("invalid sample tolerance or boundary window")
    if commit_sha is not None and not re.fullmatch(r"[0-9a-fA-F]{40}", commit_sha):
        raise ValueError("commit SHA must be exactly 40 hexadecimal characters")
    expected_paths = [Path(path) for path in expected_paths]
    captured_path = Path(captured_path)
    infos = [_wav_info(path) for path in expected_paths]
    for path, info in zip(expected_paths, infos):
        if info["frames"] == 0:
            raise ValueError(f"{path}: canonical reference track has no PCM frames")
    capture_info = _wav_info(captured_path)
    for path, info in zip(expected_paths, infos):
        for field in ("channels", "sample_rate"):
            if info[field] != capture_info[field]:
                raise ValueError(
                    f"{path}: {field} differs from capture; first produce a "
                    "canonical PCM reference using the application's decoder"
                )

    expected_frames = sum(info["frames"] for info in infos)
    boundaries = []
    cumulative = 0
    for info in infos[:-1]:
        cumulative += info["frames"]
        boundaries.append({
            "frame": cumulative,
            "window_start": max(0, cumulative - boundary_window_frames),
            "window_end": min(expected_frames, cumulative + boundary_window_frames),
            "mismatch_frames": 0,
            "first_mismatch_frame": None,
        })

    def reference_frames():
        for path in expected_paths:
            yield from _frames(path, capture_info["channels"])

    def mark_boundary_mismatch(frame_index: int) -> None:
        for boundary in boundaries:
            if boundary["window_start"] <= frame_index < boundary["window_end"]:
                boundary["mismatch_frames"] += 1
                if boundary["first_mismatch_frame"] is None:
                    boundary["first_mismatch_frame"] = frame_index

    missing = extra = mismatched_frames = mismatched_samples = 0
    observed_capture_frames = 0
    observed_reference_frames = 0
    compared_samples = squared_error = max_abs_error = 0
    examples = []
    absent = object()
    for index, (wanted, actual) in enumerate(
        zip_longest(
            reference_frames(),
            _frames(captured_path, capture_info["channels"]),
            fillvalue=absent,
        )
    ):
        if wanted is not absent:
            observed_reference_frames += 1
        if actual is not absent:
            observed_capture_frames += 1
        if wanted is absent:
            extra += 1
            mark_boundary_mismatch(index)
            if len(examples) < MISMATCH_EXAMPLES:
                examples.append({"frame": index, "kind": "extra", "captured": list(actual)})
            continue
        if actual is absent:
            missing += 1
            mark_boundary_mismatch(index)
            if len(examples) < MISMATCH_EXAMPLES:
                examples.append({"frame": index, "kind": "missing", "expected": list(wanted)})
            continue
        differing = False
        for desired_sample, observed_sample in zip(wanted, actual):
            delta = abs(desired_sample - observed_sample)
            max_abs_error = max(max_abs_error, delta)
            squared_error += delta * delta
            compared_samples += 1
            if delta > tolerance_lsb:
                mismatched_samples += 1
                differing = True
        if differing:
            mismatched_frames += 1
            if len(examples) < MISMATCH_EXAMPLES:
                examples.append({
                    "frame": index, "kind": "sample_mismatch",
                    "expected": list(wanted), "captured": list(actual),
                })
            mark_boundary_mismatch(index)

    if observed_reference_frames != expected_frames:
        raise ValueError(
            "canonical reference PCM truncated: "
            f"declared {expected_frames} frames, observed {observed_reference_frames}"
        )

    # A WAV header can declare frames that are absent from its payload. If the
    # actual capture happens to equal the reference length, an index-aligned
    # comparison alone would otherwise report a false PASS.
    capture_truncated_frames = max(0, capture_info["frames"] - observed_capture_frames)

    return {
        "status": "PASS" if not (missing or extra or mismatched_samples or capture_truncated_frames) else "FAIL",
        "commit_sha": commit_sha.lower() if commit_sha is not None else None,
        "sample_rate": capture_info["sample_rate"],
        "channels": capture_info["channels"],
        "expected_frames": expected_frames,
        "captured_frames": observed_capture_frames,
        "capture_header_frames": capture_info["frames"],
        "capture_truncated_frames": capture_truncated_frames,
        "missing_tail_frames": missing,
        "extra_tail_frames": extra,
        "mismatched_frames": mismatched_frames,
        "mismatched_samples": mismatched_samples,
        "tolerance_lsb": tolerance_lsb,
        "max_absolute_sample_error_lsb": max_abs_error,
        "rms_sample_error_lsb": math.sqrt(squared_error / compared_samples)
        if compared_samples else 0.0,
        "track_boundaries": boundaries,
        "first_mismatches": examples,
        "note": (
            "Index-aligned PCM comparison. Insertions/drops shift subsequent "
            "matches; tail counts do not localize the defect. Pre-ring capture "
            "cannot establish native callback or DAC continuity."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", nargs="+", type=Path, required=True,
                        help="Canonical PCM16 WAV tracks in playback order")
    parser.add_argument("--captured", type=Path, required=True)
    parser.add_argument("--report", type=Path, help="Optional JSON evidence")
    parser.add_argument("--commit-sha", help="Exact source commit for evidence provenance (40 hex digits)")
    parser.add_argument("--tolerance-lsb", type=int, default=0)
    parser.add_argument("--boundary-window-frames", type=int, default=480)
    args = parser.parse_args()
    try:
        report = compare(
            args.expected, args.captured,
            tolerance_lsb=args.tolerance_lsb,
            boundary_window_frames=args.boundary_window_frames,
            commit_sha=args.commit_sha,
        )
    except (ValueError, OSError, EOFError, wave.Error) as error:
        report = {"status": "ERROR", "message": str(error)}
    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    print(serialized, end="")
    if args.report:
        args.report.write_text(serialized, encoding="utf-8")
    return {"PASS": 0, "FAIL": 1, "ERROR": 2}[report["status"]]


if __name__ == "__main__":
    sys.exit(main())
