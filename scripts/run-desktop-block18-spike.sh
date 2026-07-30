#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="${ROOT}/tools/decoder-spike"
WORK="${ROOT}/target/desktop-block18-spike"
FIXTURES="${WORK}/fixtures"
RAW="${WORK}/raw"
RESULTS="${ROOT}/docs/measurements"
REPORT_JSON="${RESULTS}/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.json"
REPORT_MD="${RESULTS}/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md"
DECISION_MD="${ROOT}/docs/DESKTOP_BLOCK18_DECODER_DECISION.md"

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required command not found: $1" >&2
    exit 1
  }
}

require_command cargo
require_command ffmpeg
require_command python3
require_command /usr/bin/time

rm -rf "${WORK}"
mkdir -p "${FIXTURES}" "${RAW}" "${RESULTS}"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=997:sample_rate=48000:duration=12" \
  -ar 48000 -ac 2 -c:a pcm_s16le "${FIXTURES}/reference.wav"
ffmpeg -hide_banner -loglevel error -y \
  -i "${FIXTURES}/reference.wav" -c:a flac "${FIXTURES}/reference.flac"
ffmpeg -hide_banner -loglevel error -y \
  -i "${FIXTURES}/reference.wav" -c:a libmp3lame -b:a 192k "${FIXTURES}/reference.mp3"

python3 - "${FIXTURES}" <<'PY'
from pathlib import Path
import sys

fixtures = Path(sys.argv[1])

def synchsafe(value: int) -> bytes:
    if value < 0 or value > 0x0FFFFFFF:
        raise ValueError("ID3 size does not fit a synchsafe integer")
    return bytes(((value >> 21) & 0x7F, (value >> 14) & 0x7F, (value >> 7) & 0x7F, value & 0x7F))

mp3 = (fixtures / "reference.mp3").read_bytes()
payload = b"\x03silent-disco-block18\x00" + (b"M" * (2 * 1024 * 1024))
frame = b"TXXX" + synchsafe(len(payload)) + b"\x00\x00" + payload
tag = b"ID3\x04\x00\x00" + synchsafe(len(frame)) + frame
(fixtures / "metadata-heavy.mp3").write_bytes(tag + mp3)

flac = (fixtures / "reference.flac").read_bytes()
(fixtures / "truncated.flac").write_bytes(flac[: max(64, len(flac) // 2)])
(fixtures / "corrupt.bin").write_bytes(bytes((index * 73 + 19) % 256 for index in range(4096)))
PY

cargo generate-lockfile --manifest-path "${CRATE}/Cargo.toml"
cargo fmt --manifest-path "${CRATE}/Cargo.toml" -- --check
cargo clippy --manifest-path "${CRATE}/Cargo.toml" --all-targets --all-features -- -D warnings
cargo test --manifest-path "${CRATE}/Cargo.toml" --all-features
cargo build --manifest-path "${CRATE}/Cargo.toml" --locked --release

BINARY="${CRATE}/target/release/silent-disco-decoder-spike"

run_case() {
  local name="$1"
  shift
  /usr/bin/time -v -o "${RAW}/${name}.time" \
    "${BINARY}" "$@" > "${RAW}/${name}.json"
}

run_case wav "${FIXTURES}/reference.wav"
run_case flac "${FIXTURES}/reference.flac"
run_case mp3 "${FIXTURES}/reference.mp3"
run_case metadata "${FIXTURES}/metadata-heavy.mp3"
run_case truncated "${FIXTURES}/truncated.flac"
run_case corrupt "${FIXTURES}/corrupt.bin"
run_case cancelled "${FIXTURES}/reference.flac" --cancel-after-frames 48000

python3 - "${RAW}" "${REPORT_JSON}" "${REPORT_MD}" "${DECISION_MD}" <<'PY'
from __future__ import annotations

import json
from pathlib import Path
import platform
import re
import subprocess
import sys

raw = Path(sys.argv[1])
report_json = Path(sys.argv[2])
report_md = Path(sys.argv[3])
decision_md = Path(sys.argv[4])

case_names = ("wav", "flac", "mp3", "metadata", "truncated", "corrupt", "cancelled")
cases: dict[str, dict[str, object]] = {}
for name in case_names:
    result = json.loads((raw / f"{name}.json").read_text())
    timing = (raw / f"{name}.time").read_text()
    match = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", timing)
    if match is None:
        raise SystemExit(f"peak RSS missing for {name}")
    result["peak_rss_kib"] = int(match.group(1))
    rate = result.get("sample_rate_hz")
    frames = result.get("frames")
    decode_us = result.get("decode_micros")
    if isinstance(rate, int) and rate > 0 and isinstance(frames, int) and isinstance(decode_us, int) and decode_us > 0:
        media_seconds = frames / rate
        result["realtime_factor"] = media_seconds / (decode_us / 1_000_000)
    else:
        result["realtime_factor"] = None
    cases[name] = result

for name in ("wav", "flac", "mp3", "metadata"):
    result = cases[name]
    if result["status"] != "decoded":
        raise SystemExit(f"{name} did not decode: {result}")
    if result["sample_rate_hz"] != 48000 or result["channels"] != 2:
        raise SystemExit(f"{name} produced an unexpected format: {result}")
    if not isinstance(result["frames"], int) or result["frames"] < 570_000:
        raise SystemExit(f"{name} produced too few frames: {result}")

if cases["cancelled"]["status"] != "cancelled" or cases["cancelled"]["cancellation_requested"] is not True:
    raise SystemExit(f"cooperative cancellation failed: {cases['cancelled']}")
if not 48_000 <= int(cases["cancelled"]["frames"]) <= 60_000:
    raise SystemExit(f"cancellation was not observed at a packet boundary: {cases['cancelled']}")

for name in ("truncated", "corrupt"):
    result = cases[name]
    if result["status"] != "error":
        raise SystemExit(f"{name} was not rejected visibly: {result}")
    if result["error_class"] not in {"corrupt", "io", "limit", "unsupported", "empty_stream"}:
        raise SystemExit(f"{name} returned an unclassified failure: {result}")

metadata_rss = int(cases["metadata"]["peak_rss_kib"])
mp3_rss = int(cases["mp3"]["peak_rss_kib"])
if metadata_rss > max(131_072, mp3_rss + 65_536):
    raise SystemExit(
        f"metadata-heavy input exceeded the memory envelope: metadata={metadata_rss} KiB, baseline={mp3_rss} KiB"
    )

for name in ("wav", "flac", "mp3"):
    factor = cases[name]["realtime_factor"]
    if not isinstance(factor, (int, float)) or factor <= 1.0:
        raise SystemExit(f"{name} did not decode faster than realtime: {cases[name]}")

rustc = subprocess.check_output(["rustc", "--version"], text=True).strip()
cargo = subprocess.check_output(["cargo", "--version"], text=True).strip()
ffmpeg = subprocess.check_output(["ffmpeg", "-version"], text=True).splitlines()[0]
report = {
    "schema_version": 1,
    "candidate": {
        "crate": "symphonia",
        "version": "0.6.0",
        "default_features": False,
        "features": ["wav", "pcm", "flac", "mp3", "id3v1", "id3v2"],
        "license": "MPL-2.0",
    },
    "environment": {
        "platform": platform.platform(),
        "rustc": rustc,
        "cargo": cargo,
        "ffmpeg": ffmpeg,
    },
    "fixtures": {
        "duration_seconds": 12,
        "sample_rate_hz": 48000,
        "channels": 2,
        "metadata_payload_bytes": 2 * 1024 * 1024,
    },
    "cases": cases,
    "decision": {
        "ownership": "shared Rust streaming decoder",
        "initial_formats": ["WAV/PCM", "FLAC", "MP3"],
        "canonical_output": "48 kHz stereo PCM16 little-endian, produced incrementally",
        "production_fallback": "none",
        "implementation_block": "Desktop Block 19 / shared decoder boundary",
    },
}
report_json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

rows = []
for name in case_names:
    case = cases[name]
    factor = case["realtime_factor"]
    factor_text = "—" if factor is None else f"{factor:.1f}×"
    rows.append(
        f"| {name} | {case['status']} | {case['error_class'] or '—'} | "
        f"{case['startup_micros'] / 1000:.2f} | {case['decode_micros'] / 1000:.2f} | "
        f"{case['peak_rss_kib'] / 1024:.1f} | {factor_text} | {case['frames']} |"
    )

report_md.write_text(
    "# Desktop Block 18 decoder spike results\n\n"
    "Generated by `scripts/run-desktop-block18-spike.sh`. The JSON companion is the machine-readable source of truth.\n\n"
    f"- Rust: `{rustc}`\n"
    f"- Cargo: `{cargo}`\n"
    f"- FFmpeg: `{ffmpeg}`\n"
    "- Candidate: `symphonia = 0.6.0`, defaults disabled, features `wav`, `pcm`, `flac`, `mp3`, `id3v1`, `id3v2`\n"
    "- Fixtures: deterministic 12-second, 48 kHz, stereo tone plus truncated, corrupt, cancellation, and 2 MiB metadata cases\n\n"
    "| Case | Status | Error class | Startup ms | Decode ms | Peak RSS MiB | Realtime factor | Frames |\n"
    "|---|---:|---|---:|---:|---:|---:|---:|\n"
    + "\n".join(rows)
    + "\n\nAll executable assertions passed. Measurements are CI-host specific and must not be presented as universal limits.\n"
)

decision_md.write_text(
    "# Desktop Block 18 decoder decision\n\n"
    "## Decision\n\n"
    "Select **Path B: one shared Rust streaming decoder**. Pin `symphonia` exactly at `0.6.0`, disable default features, "
    "and enable only `wav`, `pcm`, `flac`, `mp3`, `id3v1`, and `id3v2` for the initial release. There is no automatic "
    "fallback to Android `MediaCodec`, HTML audio, Web Audio, or a TypeScript decoder.\n\n"
    "## Initial production scope\n\n"
    "- Containers/codecs: WAV/PCM, native FLAC, and MP3.\n"
    "- Input: stable app-private files produced by the platform staging boundary.\n"
    "- Decoder output: source-native planar buffers consumed incrementally.\n"
    "- Canonical host boundary: bounded 48 kHz stereo PCM16 little-endian chunks. Block 19 owns incremental sample "
    "conversion, channel mapping, resampling, backpressure, cancellation, and worker join.\n"
    "- Metadata: retain Symphonia's defensive reader limits and regression-test oversized metadata; never load cover art "
    "or entire tracks into the host pipeline.\n\n"
    "## Evidence\n\n"
    "The executable spike builds on the repository's pinned Rust `1.97.1` toolchain, decodes representative WAV, FLAC, "
    "and MP3 fixtures, rejects corrupt/truncated input visibly, observes cancellation at a packet boundary, measures startup, "
    "throughput, and peak RSS, and exercises a 2 MiB metadata tag. Exact results are in "
    "`docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md` and its JSON companion.\n\n"
    "## Rejected alternatives\n\n"
    "- **Android/iOS-only platform decoding:** rejected as the long-term ownership model because desktop requires an "
    "independent decoder and dual production paths would create format and failure-policy drift. Existing Android decoding "
    "remains temporary until shared Block 23 migrates mobile and receives device evidence.\n"
    "- **Desktop-only Rust adapter:** unnecessary because the selected library is portable Rust and can live at the shared boundary.\n"
    "- **FFmpeg bindings:** rejected for the initial path because they add native-library packaging, ABI, licensing, and "
    "cross-platform deployment complexity not required for WAV/FLAC/MP3.\n"
    "- **Rodio:** rejected as an ownership layer because it combines playback concerns with decoding and does not replace the "
    "project's shared bounded packetization boundary.\n"
    "- **TypeScript, HTML audio, or Web Audio:** prohibited by the desktop architecture.\n\n"
    "## Coordination with shared Block 23\n\n"
    "This decision selects shared Block 23 Path B, but does **not** claim Block 23 complete. Android PCM-copy overhead, current "
    "platform memory/startup measurements, iOS file-access constraints, and physical-device format parity remain required before "
    "the temporary mobile decoder path is removed. No hidden fallback is introduced during that migration.\n"
)
PY

printf 'Block 18 spike results written to:\n  %s\n  %s\n  %s\n' \
  "${REPORT_JSON}" "${REPORT_MD}" "${DECISION_MD}"
