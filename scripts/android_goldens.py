#!/usr/bin/env python3
"""Android screenshot goldens: compare CI captures with approved baselines.

Captures come from `WorkflowScreenshotCaptureTest` on the CI API 29 emulator
(`scripts/run-android-emulator-tests.sh`); baselines must come from those
captures too, because other devices and renderers produce different pixels.

Baselines live in a golden directory as `<scene>.png` plus `approved.tsv`
(scene, width, height, sha256, approved_by, approved_on). They change only
through `approve`, which names the person approving; `check` refuses a
baseline whose bytes do not match its approved row, so a PNG replaced by hand
fails CI.

  check CAPTURES GOLDENS OUT
      Every approved scene must be captured with identical pixels. A mismatch
      writes OUT/<scene>.actual.png and OUT/<scene>.diff.png (changed pixels
      red over the dimmed baseline) and fails. Captured scenes with no
      baseline are reported as unreviewed, not failed.
  approve CAPTURES GOLDENS --approver NAME (--scene SCENE ... | --all)
      Copies the named captures into GOLDENS as approved baselines.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import shutil
import struct
import sys
import zlib
from pathlib import Path

APPROVED = "approved.tsv"
HEADER = "scene\twidth_px\theight_px\tsha256\tapproved_by\tapproved_on"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


# --- minimal PNG codec (8-bit RGB/RGBA, non-interlaced, all five filters) ---


def decode_png(data: bytes) -> tuple[int, int, list[bytes]]:
    """Returns (width, height, rows of RGBA bytes)."""
    if not data.startswith(PNG_SIGNATURE):
        raise ValueError("not a PNG file")
    offset = len(PNG_SIGNATURE)
    width = height = color_type = None
    compressed = bytearray()
    while offset < len(data):
        (length,) = struct.unpack(">I", data[offset : offset + 4])
        kind = data[offset + 4 : offset + 8]
        body = data[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IHDR":
            width, height, depth, color_type, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or color_type not in (2, 6) or interlace != 0:
                raise ValueError(f"unsupported PNG: depth {depth}, color type {color_type}, interlace {interlace}")
        elif kind == b"IDAT":
            compressed += body
        elif kind == b"IEND":
            break
    if width is None or height is None or color_type is None:
        raise ValueError("PNG has no IHDR")
    channels = 4 if color_type == 6 else 3
    stride = width * channels
    raw = zlib.decompress(bytes(compressed))
    if len(raw) != height * (stride + 1):
        raise ValueError("PNG image data has the wrong length")
    rows: list[bytes] = []
    previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        filter_type = raw[start]
        row = bytearray(raw[start + 1 : start + 1 + stride])
        for i in range(stride):
            left = row[i - channels] if i >= channels else 0
            up = previous[i]
            up_left = previous[i - channels] if i >= channels else 0
            if filter_type == 0:
                predictor = 0
            elif filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = up
            elif filter_type == 3:
                predictor = (left + up) // 2
            elif filter_type == 4:
                estimate = left + up - up_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - up_left))
                predictor = (left, up, up_left)[distances.index(min(distances))]
            else:
                raise ValueError(f"unknown PNG filter {filter_type}")
            row[i] = (row[i] + predictor) & 0xFF
        previous = row
        if channels == 3:
            rgba = bytearray()
            for i in range(0, stride, 3):
                rgba += row[i : i + 3] + b"\xff"
            rows.append(bytes(rgba))
        else:
            rows.append(bytes(row))
    return width, height, rows


def encode_png(width: int, height: int, rows: list[bytes]) -> bytes:
    """Encodes RGBA rows (filter 0 on every row)."""

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    raw = b"".join(b"\x00" + row for row in rows)
    return (
        PNG_SIGNATURE
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def diff_image(expected: list[bytes], actual: list[bytes], width: int) -> tuple[int, list[bytes]]:
    """Changed-pixel count and an RGBA image: changed pixels red, others the dimmed baseline."""
    changed = 0
    out: list[bytes] = []
    for expected_row, actual_row in zip(expected, actual):
        row = bytearray()
        for x in range(width):
            a = expected_row[4 * x : 4 * x + 4]
            b = actual_row[4 * x : 4 * x + 4]
            if a != b:
                changed += 1
                row += b"\xff\x00\x00\xff"
            else:
                row += bytes(channel // 3 + 170 for channel in a[:3]) + b"\xff"
        out.append(bytes(row))
    return changed, out


# --- manifests ---


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_captures(directory: Path) -> dict[str, tuple[int, int, str]]:
    lines = (directory / "manifest.tsv").read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "scene\twidth_px\theight_px\tsha256":
        raise ValueError(f"{directory}/manifest.tsv has an unexpected header")
    captures = {}
    for line in lines[1:]:
        scene, width, height, digest = line.split("\t")
        captures[scene] = (int(width), int(height), digest)
    return captures


def read_approved(directory: Path) -> dict[str, dict[str, str]]:
    path = directory / APPROVED
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line and not line.startswith("#")]
    if not lines or lines[0] != HEADER:
        raise ValueError(f"{path} must start with the header: {HEADER}")
    approved = {}
    for line in lines[1:]:
        fields = line.split("\t")
        if len(fields) != 6:
            raise ValueError(f"{path}: malformed row: {line!r}")
        scene, width, height, digest, approved_by, approved_on = fields
        if scene in approved:
            raise ValueError(f"{path}: duplicate scene {scene}")
        approved[scene] = {
            "width": width,
            "height": height,
            "sha256": digest,
            "approved_by": approved_by,
            "approved_on": approved_on,
        }
    return approved


def write_approved(directory: Path, approved: dict[str, dict[str, str]], preamble: list[str]) -> None:
    rows = [
        "\t".join(
            [scene, row["width"], row["height"], row["sha256"], row["approved_by"], row["approved_on"]]
        )
        for scene, row in sorted(approved.items())
    ]
    (directory / APPROVED).write_text("\n".join(preamble + [HEADER] + rows) + "\n", encoding="utf-8")


# --- commands ---


def check(captures_dir: Path, goldens_dir: Path, out_dir: Path) -> list[str]:
    """Problems that fail the gate; prints the review summary."""
    problems: list[str] = []
    approved = read_approved(goldens_dir)
    captures = read_captures(captures_dir)

    # Baseline integrity: only approved bytes, every approval complete.
    on_disk = {path.stem for path in goldens_dir.glob("*.png")}
    for scene in sorted(on_disk - set(approved)):
        problems.append(f"baseline {scene}.png has no approval row; add it with `approve`")
    for scene, row in sorted(approved.items()):
        png = goldens_dir / f"{scene}.png"
        if not row["approved_by"].strip():
            problems.append(f"baseline {scene} has no approver")
        try:
            datetime.date.fromisoformat(row["approved_on"])
        except ValueError:
            problems.append(f"baseline {scene} has an invalid approval date {row['approved_on']!r}")
        if not png.exists():
            problems.append(f"baseline {scene}.png is missing")
        elif sha256(png) != row["sha256"]:
            problems.append(f"baseline {scene}.png does not match its approved hash; it was changed without `approve`")

    # Every approved scene must still be captured, with the same pixels.
    unreviewed = sorted(set(captures) - set(approved))
    matched = 0
    for scene, row in sorted(approved.items()):
        if scene not in captures:
            problems.append(f"approved scene {scene} was not captured")
            continue
        width, height, digest = captures[scene]
        if digest == row["sha256"]:
            matched += 1
            continue
        out_dir.mkdir(parents=True, exist_ok=True)
        actual_png = captures_dir / f"{scene}.png"
        shutil.copyfile(actual_png, out_dir / f"{scene}.actual.png")
        golden_png = goldens_dir / f"{scene}.png"
        if (width, height) != (int(row["width"]), int(row["height"])) or not golden_png.exists():
            problems.append(
                f"scene {scene} is {width}x{height}, baseline is {row['width']}x{row['height']}"
            )
            continue
        _, _, expected_rows = decode_png(golden_png.read_bytes())
        _, _, actual_rows = decode_png(actual_png.read_bytes())
        changed, diff_rows = diff_image(expected_rows, actual_rows, width)
        (out_dir / f"{scene}.diff.png").write_bytes(encode_png(width, height, diff_rows))
        problems.append(
            f"scene {scene}: {changed} of {width * height} pixels differ from the approved baseline "
            f"(see {scene}.diff.png and {scene}.actual.png)"
        )

    print(f"goldens: {matched} match, {len(approved) - matched} differ or missing, {len(unreviewed)} unreviewed")
    for scene in unreviewed:
        print(f"  unreviewed (no approved baseline): {scene}")
    return problems


def approve(captures_dir: Path, goldens_dir: Path, approver: str, scenes: list[str], today: datetime.date) -> None:
    if not approver.strip():
        raise ValueError("--approver must name the person approving these baselines")
    captures = read_captures(captures_dir)
    unknown = [scene for scene in scenes if scene not in captures]
    if unknown:
        raise ValueError(f"not in the captures: {', '.join(unknown)}")
    path = goldens_dir / APPROVED
    preamble = [line for line in path.read_text(encoding="utf-8").splitlines() if line.startswith("#")]
    approved = read_approved(goldens_dir)
    for scene in scenes:
        width, height, digest = captures[scene]
        source = captures_dir / f"{scene}.png"
        if sha256(source) != digest:
            raise ValueError(f"capture {scene}.png does not match its capture manifest")
        shutil.copyfile(source, goldens_dir / f"{scene}.png")
        approved[scene] = {
            "width": str(width),
            "height": str(height),
            "sha256": digest,
            "approved_by": approver.strip(),
            "approved_on": today.isoformat(),
        }
        print(f"approved {scene} ({width}x{height}) by {approver.strip()}")
    write_approved(goldens_dir, approved, preamble)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    check_parser = commands.add_parser("check")
    check_parser.add_argument("captures", type=Path)
    check_parser.add_argument("goldens", type=Path)
    check_parser.add_argument("out", type=Path)
    approve_parser = commands.add_parser("approve")
    approve_parser.add_argument("captures", type=Path)
    approve_parser.add_argument("goldens", type=Path)
    approve_parser.add_argument("--approver", required=True)
    which = approve_parser.add_mutually_exclusive_group(required=True)
    which.add_argument("--scene", action="append", default=[])
    which.add_argument("--all", action="store_true")
    args = parser.parse_args(argv)

    if args.command == "check":
        problems = check(args.captures, args.goldens, args.out)
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1 if problems else 0
    scenes = sorted(read_captures(args.captures)) if args.all else args.scene
    approve(args.captures, args.goldens, args.approver, scenes, datetime.date.today())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
