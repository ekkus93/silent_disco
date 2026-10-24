#!/usr/bin/env python3
"""Regression cases for fail-closed JSON-only PCM artifact validation."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_pcm_evidence import REPORT_NAMES, validate_directory

SHA = "a" * 40


def sample_report(name):
    base = {"schema": 1, "status": "PASS", "commit": SHA, "physical_dac_verified": False}
    if name == "mixed-rate-socket-callback.json":
        base.update(
            capture="native-rust-callback-after-real-udp-socket",
            source_formats=["48000-stereo", "44100-mono", "96000-stereo"],
            sample_rate=48000, channels=2,
            source_boundaries=[480, 960], callback_frame_chunks=[241, 337, 517, 345],
            expected_frames=1440, captured_frames=1440, mismatched_samples=0,
            tap_mismatched_samples=0, native_underrun_callbacks=0,
            native_silence_filled_frames=0, tap_lost_frames=0,
            first_mismatch_frame=None, first_mismatch_channel=None,
            tap_captured_frames=1440, callback_count=4, tap_callback_count=4,
        )
    elif name == "encoded-socket-callback.json":
        base.update(
            capture="native-rust-callback-after-real-tcp-udp-sockets",
            sample_rate=48000, channels=2,
            source_formats=["wav", "flac", "mp3"], listener_scheduler_exercised=True,
            listener_volume_gains=[1.0, 0.5, 0.25],
            source_boundaries=[480, 960], observed_boundaries=[480, 960],
            captured_frames=1440, expected_frames_with_padding=1440,
            callback_frame_chunks=[481, 480, 479],
            mismatched_samples=0, scheduler_mismatched_samples=0,
            native_underrun_callbacks=0, scheduler_underrun_callbacks=0,
            native_silence_filled_frames=0, scheduler_silence_filled_frames=0,
            tap_lost_frames=0, scheduler_tap_lost_frames=0,
            first_mismatch_frame=None, first_mismatch_channel=None,
            scheduler_first_mismatch_frame=None, scheduler_first_mismatch_channel=None,
            tap_captured_frames=1440, scheduler_tap_captured_frames=1440,
            callback_count=3, scheduler_callback_count=3,
            tap_callback_count=3, scheduler_tap_callback_count=3,
        )
    elif name == "paused-resume-callback.json":
        base.update(
            capture="actor-authorized-ffi-paused-resume-native-callback",
            source_boundaries=[480], observed_boundaries=[480],
            packet_sequences=[0, 1, 2, 3],
            packet_first_sample_indices=[0, 240, 480, 720],
            expected_frames=960, captured_frames=960,
            native_ring_remaining_frames=0, callback_frame_chunks=[479, 2, 479],
            callback_count=3, native_underrun_callbacks=0,
            native_silence_filled_frames=0, tap_captured_frames=960,
            tap_lost_frames=0, tap_callback_count=3,
            pause_boundary_retained=True, paused_removal_rejected=True,
            resume_token_preserved=True,
        )
    elif name == "authoritative-edit-replay-callback.json":
        base.update(
            capture="actor-authorized-ffi-producer-native-callback",
            source_boundaries=[9600, 10080],
            first_observed_boundaries=[9600, 10080],
            replay_observed_boundaries=[9600, 10080],
            expected_frames=10560, first_captured_frames=10560,
            replay_captured_frames=10560,
            callback_frame_chunks=[4799, 4802, 480, 479],
            first_callback_count=4, replay_callback_count=4,
            first_native_underruns=0, replay_native_underruns=0,
            first_native_silence_frames=0, replay_native_silence_frames=0,
            first_tap_frames=10560, replay_tap_frames=10560,
            first_tap_lost_frames=0, replay_tap_lost_frames=0,
            first_tap_callbacks=4, replay_tap_callbacks=4,
            retired_audio_packets_before_rejection=2,
            retired_successor_rejected=True, replay_token_distinct=True,
        )
    else:
        base.update(
            capture="native-callback-after-rejected-production-reservation",
            rejected_boundary_frame=480, valid_source_frames=480,
            captured_valid_frames=480, expected_tail_silence_frames=17,
            rejected_successor_audio_leaked=False,
            first_mismatch_frame=None, first_mismatch_channel=None,
            tap_first_mismatch_frame=None, native_callback_count=2,
            native_underrun_callbacks=1, native_silence_filled_frames=17,
            tap_captured_frames=497, tap_callback_count=2, tap_lost_frames=0,
        )
    return base


class EvidenceGateTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.reports = {name: sample_report(name) for name in REPORT_NAMES}
        self.write_reports()

    def write_reports(self):
        for name, report in self.reports.items():
            (self.root / name).write_text(json.dumps(report), encoding="utf-8")

    def test_complete_exact_sha_passes(self):
        validate_directory(self.root, SHA)

    def test_failing_status_is_not_uploaded_as_passed(self):
        self.reports["encoded-socket-callback.json"]["status"] = "FAIL"
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "status"):
            validate_directory(self.root, SHA)

    def test_missing_comparison_fails_closed(self):
        (self.root / "rejected-boundary-callback.json").unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            validate_directory(self.root, SHA)

    def test_unexpected_audio_file_is_rejected(self):
        (self.root / "user-recording.wav").write_bytes(b"RIFF")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            validate_directory(self.root, SHA)

    def test_unexpected_json_audio_payload_is_rejected(self):
        name = "encoded-socket-callback.json"
        self.reports[name]["raw_audio_pcm"] = [0, 1, 2, 3]
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "unexpected metadata fields"):
            validate_directory(self.root, SHA)

    def test_duplicate_json_key_is_rejected_even_if_values_agree(self):
        name = "mixed-rate-socket-callback.json"
        path = self.root / name
        original = path.read_text(encoding="utf-8")
        path.write_text(original[:-1] + ', "status": "PASS"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate JSON field: status"):
            validate_directory(self.root, SHA)

    def test_nested_duplicate_json_key_is_rejected(self):
        name = "encoded-socket-callback.json"
        path = self.root / name
        original = path.read_text(encoding="utf-8")
        path.write_text(
            original[:-1] + ', "extra": {"a": 1, "a": 1}}',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "duplicate JSON field: a"):
            validate_directory(self.root, SHA)

    def test_wrong_exact_commit_is_rejected(self):
        self.reports["mixed-rate-socket-callback.json"]["commit"] = "b" * 40
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "commit"):
            validate_directory(self.root, SHA)

    def test_short_commit_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "40-character"):
            validate_directory(self.root, SHA[:8])

    def test_encoded_callback_loss_fails(self):
        self.reports["encoded-socket-callback.json"]["scheduler_tap_lost_frames"] = 1
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "scheduler_tap_lost_frames"):
            validate_directory(self.root, SHA)

    def test_encoded_codec_boundary_or_gain_mismatch_fails(self):
        self.reports["encoded-socket-callback.json"]["observed_boundaries"] = [480, 961]
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "observed_boundaries"):
            validate_directory(self.root, SHA)
        self.reports["encoded-socket-callback.json"] = sample_report(
            "encoded-socket-callback.json")
        self.reports["encoded-socket-callback.json"]["listener_volume_gains"] = [1, 1, 1]
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "listener_volume_gains"):
            validate_directory(self.root, SHA)

    def test_rejected_successor_leak_or_hidden_underrun_fails(self):
        self.reports["rejected-boundary-callback.json"]["rejected_successor_audio_leaked"] = True
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "rejected_successor_audio_leaked"):
            validate_directory(self.root, SHA)
        self.reports["rejected-boundary-callback.json"] = sample_report(
            "rejected-boundary-callback.json")
        self.reports["rejected-boundary-callback.json"]["native_underrun_callbacks"] = 0
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "native_underrun_callbacks"):
            validate_directory(self.root, SHA)

    def test_authoritative_replay_rejects_stale_pcm_or_missing_callback(self):
        name = "authoritative-edit-replay-callback.json"
        self.reports[name]["first_observed_boundaries"] = [9600, 10081]
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "first_observed_boundaries"):
            validate_directory(self.root, SHA)
        self.reports[name] = sample_report(name)
        self.reports[name]["replay_tap_frames"] = 10559
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "replay_tap_frames"):
            validate_directory(self.root, SHA)

    def test_authoritative_replay_requires_rejection_and_distinct_output(self):
        name = "authoritative-edit-replay-callback.json"
        self.reports[name]["retired_successor_rejected"] = False
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "retired_successor_rejected"):
            validate_directory(self.root, SHA)
        self.reports[name] = sample_report(name)
        self.reports[name]["replay_token_distinct"] = False
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "replay_token_distinct"):
            validate_directory(self.root, SHA)

    def test_authoritative_replay_rejects_native_silence(self):
        name = "authoritative-edit-replay-callback.json"
        self.reports[name]["replay_native_silence_frames"] = 1
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "replay_native_silence_frames"):
            validate_directory(self.root, SHA)

    def test_nested_boolean_and_float_impostors_do_not_pass_pcm_evidence(self):
        cases = (
            ("mixed-rate-socket-callback.json", "source_boundaries", [480.0, 960]),
            ("mixed-rate-socket-callback.json", "callback_frame_chunks",
             [241, 337, 517.0, 345]),
            ("encoded-socket-callback.json", "listener_volume_gains",
             [True, 0.5, 0.25]),
            ("authoritative-edit-replay-callback.json", "source_boundaries",
             [9600, 10080.0]),
            ("authoritative-edit-replay-callback.json", "callback_frame_chunks",
             [4799, 4802, 480.0, 479]),
        )
        for name, field, value in cases:
            with self.subTest(report=name, field=field):
                self.reports = {item: sample_report(item) for item in REPORT_NAMES}
                self.reports[name][field] = value
                self.write_reports()
                with self.assertRaisesRegex(ValueError, field):
                    validate_directory(self.root, SHA)

    def test_paused_reservation_requires_native_continuity_and_rejection(self):
        name = "paused-resume-callback.json"
        cases = (
            ("observed_boundaries", [481]),
            ("packet_sequences", [0, 1, 1, 3]),
            ("packet_sequences", [0, 1, 3, 4]),
            ("packet_first_sample_indices", [0, 240, 481, 720]),
            ("packet_first_sample_indices", [0, 240, 240, 720]),
            ("callback_frame_chunks", [480, 1, 479]),
            ("native_silence_filled_frames", 1),
            ("native_ring_remaining_frames", 1),
            ("tap_lost_frames", 1),
            ("pause_boundary_retained", False),
            ("paused_removal_rejected", False),
            ("resume_token_preserved", False),
        )
        for field, value in cases:
            with self.subTest(field=field):
                self.reports[name] = sample_report(name)
                self.reports[name][field] = value
                self.write_reports()
                with self.assertRaisesRegex(ValueError, field):
                    validate_directory(self.root, SHA)

    def test_paused_callback_evidence_requires_ring_drain_field(self):
        name = "paused-resume-callback.json"
        self.reports[name].pop("native_ring_remaining_frames")
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "native_ring_remaining_frames"):
            validate_directory(self.root, SHA)

    def test_no_dac_claim_or_nonfinite_values_are_accepted(self):
        self.reports["mixed-rate-socket-callback.json"]["physical_dac_verified"] = True
        self.write_reports()
        with self.assertRaisesRegex(ValueError, "physical_dac_verified"):
            validate_directory(self.root, SHA)
        self.reports["mixed-rate-socket-callback.json"] = sample_report(
            "mixed-rate-socket-callback.json")
        self.write_reports()
        (self.root / "mixed-rate-socket-callback.json").write_text(
            '{"status": NaN}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "non-finite"):
            validate_directory(self.root, SHA)


    def test_encoded_boundaries_must_fit_inside_captured_audio(self):
        # A self-consistent pair of impossible boundary positions is not
        # evidence that either track transition reached the callback.
        name = "encoded-socket-callback.json"
        for boundaries in ([480, 1440], [1441, 1442], [999999, 1000000]):
            with self.subTest(boundaries=boundaries):
                self.reports[name] = sample_report(name)
                self.reports[name]["source_boundaries"] = boundaries
                self.reports[name]["observed_boundaries"] = boundaries
                self.write_reports()
                with self.assertRaisesRegex(ValueError, "source boundaries must precede"):
                    validate_directory(self.root, SHA)

if __name__ == "__main__":
    unittest.main()
