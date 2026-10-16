"""Deterministic tests for the captured gapless playback evidence preflight."""

from __future__ import annotations

import copy
import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-gapless-evidence.py"
spec = importlib.util.spec_from_file_location("gapless_evidence", SCRIPT)
assert spec is not None and spec.loader is not None
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


def good_trace() -> dict:
    packets = [
        {"sequence": 30, "first_sample_index": 9600, "frame_count": 960},
        {"sequence": 31, "first_sample_index": 10560, "frame_count": 960},
        {"sequence": 32, "first_sample_index": 11520, "frame_count": 960},
    ]
    boundary = [{"entry_id": "second-track", "first_sample_index": 10601}]
    listener = {
        "id": "android-listener-A",
        "packets": copy.deepcopy(packets),
        "boundaries": copy.deepcopy(boundary),
        "boundary_scheduled": 1,
        "boundary_applied": 1,
        "output_opens_before": 1,
        "output_opens_after": 1,
        "output_closes_before": 0,
        "output_closes_after": 0,
        "concealed_frames_before": 0,
        "concealed_frames_after": 0,
        "missing_boundary_frames": 0,
        "duplicate_boundary_frames": 0,
    }
    return {
        "schema_version": 1,
        "test_case": "desktop host to Android listener: FLAC to MP3",
        "stream_id": "stream-001",
        "sample_rate_hz": 48000,
        "host": {
            "packets": packets,
            "boundaries": boundary,
            "boundary_scheduled": 1,
            "boundary_applied": 1,
            "boundary_discontinuities": 0,
        },
        "listeners": [listener],
    }


class GaplessEvidenceTests(unittest.TestCase):
    def assert_invalid(self, trace: dict, phrase: str) -> None:
        with self.assertRaisesRegex(verifier.InvalidEvidence, phrase):
            verifier.check_capture(trace)

    def test_contiguous_non_packet_aligned_boundary_passes(self) -> None:
        self.assertEqual(verifier.check_capture(good_trace()), {"listeners": 1, "boundaries": 1, "packets": 3})

    def test_missing_host_sequence_fails(self) -> None:
        trace = good_trace()
        trace["host"]["packets"][1]["sequence"] = 33
        self.assert_invalid(trace, "sequence")

    def test_overlapping_host_audio_fails(self) -> None:
        trace = good_trace()
        trace["host"]["packets"][1]["first_sample_index"] -= 1
        self.assert_invalid(trace, "overlapping")

    def test_missing_listener_packet_fails(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["packets"].pop(1)
        self.assert_invalid(trace, "sequence")

    def test_wrong_listener_boundary_fails(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["boundaries"][0]["entry_id"] = "removed-track"
        self.assert_invalid(trace, "boundary identity")

    def test_boundary_beyond_capture_fails(self) -> None:
        trace = good_trace()
        trace["host"]["boundaries"][0]["first_sample_index"] = 12500
        self.assert_invalid(trace, "strictly inside")

    def test_device_output_reopen_fails(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["output_opens_after"] = 2
        self.assert_invalid(trace, "output_opens changed")

    def test_concealment_growth_fails(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["concealed_frames_after"] = 1
        self.assert_invalid(trace, "concealed_frames changed")

    def test_duplicate_boundary_audio_fails(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["duplicate_boundary_frames"] = 1
        self.assert_invalid(trace, "duplicate_boundary_frames")

    def test_boolean_counters_rejected(self) -> None:
        trace = good_trace()
        trace["host"]["boundary_scheduled"] = True
        self.assert_invalid(trace, "nonnegative integer")

    def test_independent_golden_boundary_matches(self) -> None:
        trace = good_trace()
        trace["expected_boundaries"] = copy.deepcopy(trace["host"]["boundaries"])
        verifier.check_capture(trace, require_golden_boundaries=True)

    def test_wrong_host_song_rejected_even_when_listeners_agree(self) -> None:
        trace = good_trace()
        trace["host"]["boundaries"][0]["entry_id"] = "stale-removed-track"
        trace["listeners"][0]["boundaries"][0]["entry_id"] = "stale-removed-track"
        trace["expected_boundaries"] = [{"entry_id": "replacement-track", "first_sample_index": 10601}]
        self.assert_invalid(trace, "independent expected boundaries")

    def test_off_by_one_boundary_rejected_against_golden(self) -> None:
        trace = good_trace()
        trace["expected_boundaries"] = [{"entry_id": "second-track", "first_sample_index": 10602}]
        self.assert_invalid(trace, "independent expected boundaries")

    def test_expected_boundaries_required_in_strict_mode(self) -> None:
        with self.assertRaisesRegex(verifier.InvalidEvidence, "expected_boundaries"):
            verifier.check_capture(good_trace(), require_golden_boundaries=True)

    def test_forbidden_removed_song_rejected(self) -> None:
        trace = good_trace()
        trace["forbidden_entry_ids"] = ["second-track"]
        self.assert_invalid(trace, "removed or forbidden track")

    def test_unexpected_listener_runtime_replacement_rejected(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["output_runtime_id_before"] = "oboe-01"
        trace["listeners"][0]["output_runtime_id_after"] = "oboe-02"
        self.assert_invalid(trace, "output_runtime_id changed")

    def test_listener_stream_identity_must_match_host(self) -> None:
        trace = good_trace()
        trace["listeners"][0]["stream_id_before"] = "different-stream"
        trace["listeners"][0]["stream_id_after"] = "different-stream"
        self.assert_invalid(trace, "stream_id differs")

    def test_strict_sink_identity_requires_fields(self) -> None:
        with self.assertRaisesRegex(verifier.InvalidEvidence, "output_device_id_before"):
            verifier.check_capture(good_trace(), require_sink_identity=True)

    def test_strict_sink_identity_passes_unchanged(self) -> None:
        trace = good_trace()
        listener = trace["listeners"][0]
        for stem, value in (
            ("output_device_id", "speaker-A"),
            ("output_runtime_id", "oboe-01"),
            ("stream_id", trace["stream_id"]),
        ):
            listener[f"{stem}_before"] = value
            listener[f"{stem}_after"] = value
        verifier.check_capture(trace, require_sink_identity=True)

    def test_zero_listeners_rejected(self) -> None:
        trace = good_trace()
        trace["listeners"] = []
        self.assert_invalid(trace, "nonempty array")


if __name__ == "__main__":
    unittest.main()
