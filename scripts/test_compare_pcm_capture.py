#!/usr/bin/env python3
"""Deterministic PCM capture comparator checks (standard library only)."""
import struct
import sys
import tempfile
import unittest
from pathlib import Path
import wave

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compare_pcm_capture import compare  # noqa: E402


def save_wav(path, frames, *, rate=48000, channels=2):
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(channels)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        wav.writeframes(b"".join(struct.pack("<" + "h" * channels, *row) for row in frames))


class PcmCaptureComparisonTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.track_frames = [
            [(1000 + index * 7 + track * 223, -3000 - index * 5 - track * 337)
             for index in range(240)]
            for track in range(3)
        ]
        self.sources = [root / f"track-{index}.wav" for index in range(3)]
        for path, frames in zip(self.sources, self.track_frames):
            save_wav(path, frames)
        self.captured = root / "captured.wav"
        self.reference = [frame for track in self.track_frames for frame in track]

    def inspect(self, actual, **kwargs):
        save_wav(self.captured, actual)
        return compare(self.sources, self.captured, **kwargs)

    def test_exact_gapless_pcm_passes_with_both_boundaries(self):
        result = self.inspect(self.reference, boundary_window_frames=24)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual([part["frame"] for part in result["track_boundaries"]], [240, 480])
        self.assertEqual(result["mismatched_samples"], 0)

    def test_inserted_silence_at_boundary_fails(self):
        actual = self.reference[:240] + [(0, 0)] + self.reference[240:]
        result = self.inspect(actual, boundary_window_frames=16)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["extra_tail_frames"], 1)
        self.assertGreater(result["track_boundaries"][0]["mismatch_frames"], 0)
        self.assertEqual(result["first_mismatches"][0]["frame"], 240)

    def test_duplicate_frame_at_boundary_fails(self):
        actual = self.reference[:240] + [self.reference[239]] + self.reference[240:]
        result = self.inspect(actual)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["extra_tail_frames"], 1)
        self.assertEqual(result["first_mismatches"][0]["frame"], 240)

    def test_missing_frame_at_boundary_fails(self):
        actual = self.reference[:480] + self.reference[481:]
        result = self.inspect(actual)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["missing_tail_frames"], 1)
        self.assertEqual(result["first_mismatches"][0]["frame"], 480)

    def test_sample_corruption_and_tolerance_are_reported(self):
        actual = self.reference.copy()
        left, right = actual[250]
        actual[250] = (left + 1, right)
        self.assertEqual(self.inspect(actual)["status"], "FAIL")
        self.assertEqual(self.inspect(actual, tolerance_lsb=1)["status"], "PASS")

    def test_format_mismatch_is_rejected_instead_of_resampled_silently(self):
        save_wav(self.captured, self.reference, rate=44100)
        with self.assertRaisesRegex(ValueError, "canonical PCM reference"):
            compare(self.sources, self.captured)

    def test_missing_capture_tail_fails(self):
        result = self.inspect(self.reference[:-10])
        self.assertEqual(result["missing_tail_frames"], 10)
        self.assertEqual(result["status"], "FAIL")

    def test_missing_boundary_frame_is_counted_in_boundary_window(self):
        actual = self.reference[:240] + self.reference[241:]
        result = self.inspect(actual, boundary_window_frames=4)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["missing_tail_frames"], 1)
        self.assertGreater(result["track_boundaries"][0]["mismatch_frames"], 0)

    def test_empty_canonical_track_is_rejected_before_boundary_comparison(self):
        save_wav(self.sources[1], [])
        save_wav(self.captured, self.track_frames[0] + self.track_frames[2])
        with self.assertRaisesRegex(ValueError, "canonical reference track has no PCM frames"):
            compare(self.sources, self.captured)

    def test_all_empty_canonical_audio_cannot_false_pass(self):
        for source in self.sources:
            save_wav(source, [])
        save_wav(self.captured, [])
        with self.assertRaisesRegex(ValueError, "canonical reference track has no PCM frames"):
            compare(self.sources, self.captured)

    def test_truncated_canonical_reference_is_rejected_even_when_capture_matches(self):
        save_wav(self.captured, self.reference[:-2])
        with self.sources[-1].open("r+b") as source:
            source.truncate(self.sources[-1].stat().st_size - 8)
        with self.assertRaisesRegex(ValueError, "canonical reference PCM truncated"):
            compare(self.sources, self.captured)

    def test_capture_header_claims_extra_unwritten_frames_cannot_pass(self):
        save_wav(self.captured, self.reference)
        with self.captured.open("r+b") as capture:
            capture.seek(4)
            riff_size = struct.unpack("<I", capture.read(4))[0]
            capture.seek(4)
            capture.write(struct.pack("<I", riff_size + 8))
            capture.seek(40)
            data_size = struct.unpack("<I", capture.read(4))[0]
            capture.seek(40)
            capture.write(struct.pack("<I", data_size + 8))
        result = compare(self.sources, self.captured)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["captured_frames"], len(self.reference))
        self.assertEqual(result["capture_header_frames"], len(self.reference) + 2)
        self.assertEqual(result["capture_truncated_frames"], 2)
        self.assertEqual(result["mismatched_samples"], 0)

    def test_truncated_pcm_payload_reports_observed_not_declared_frames(self):
        save_wav(self.captured, self.reference)
        with self.captured.open("r+b") as capture:
            capture.truncate(self.captured.stat().st_size - 8)
        result = compare(self.sources, self.captured)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["capture_header_frames"], len(self.reference))
        self.assertEqual(result["captured_frames"], len(self.reference) - 2)
        self.assertEqual(result["capture_truncated_frames"], 2)
        self.assertEqual(result["missing_tail_frames"], 2)


if __name__ == "__main__":
    unittest.main()
