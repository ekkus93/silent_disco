"""Tests for the Android screenshot golden check and approval workflow."""

import datetime
import hashlib
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from android_goldens import (
    HEADER,
    approve,
    check,
    decode_png,
    encode_png,
)

TODAY = datetime.date(2026, 11, 17)


def solid(width, height, rgba):
    return [bytes(rgba) * width for _ in range(height)]


def encode_filtered(width, height, rows, filter_type, channels=4):
    """Reference encoder applying one PNG filter to every row, as Android's encoder may."""

    def paeth(left, up, up_left):
        estimate = left + up - up_left
        distances = (abs(estimate - left), abs(estimate - up), abs(estimate - up_left))
        return (left, up, up_left)[distances.index(min(distances))]

    if channels == 3:
        rows = [bytes(b for i, b in enumerate(row) if i % 4 != 3) for row in rows]
    stride = width * channels
    previous = bytes(stride)
    raw = bytearray()
    for row in rows:
        out = bytearray([filter_type])
        for i in range(stride):
            left = row[i - channels] if i >= channels else 0
            up = previous[i]
            up_left = previous[i - channels] if i >= channels else 0
            predictor = (0, left, up, (left + up) // 2, paeth(left, up, up_left))[filter_type]
            out.append((row[i] - predictor) & 0xFF)
        raw += out
        previous = row

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    color_type = 6 if channels == 4 else 2
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, color_type, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw)))
        + chunk(b"IEND", b"")
    )


def gradient(width, height):
    return [
        bytes(v for x in range(width) for v in ((x * 37 + y) % 256, (y * 53) % 256, (x * y) % 256, 255))
        for y in range(height)
    ]


class Workspace:
    """A capture directory as the CI pull leaves it, and a golden directory."""

    def __init__(self, root: Path):
        self.captures = root / "captures"
        self.goldens = root / "goldens"
        self.out = root / "diffs"
        self.captures.mkdir()
        self.goldens.mkdir()
        (self.goldens / "approved.tsv").write_text("# baselines\n" + HEADER + "\n")
        self.scenes = {}

    def capture(self, scene, width, height, rows):
        data = encode_png(width, height, rows)
        (self.captures / f"{scene}.png").write_bytes(data)
        self.scenes[scene] = (width, height, hashlib.sha256(data).hexdigest())
        lines = ["scene\twidth_px\theight_px\tsha256"] + [
            f"{name}\t{w}\t{h}\t{digest}" for name, (w, h, digest) in self.scenes.items()
        ]
        (self.captures / "manifest.tsv").write_text("\n".join(lines) + "\n")

    def check(self):
        return check(self.captures, self.goldens, self.out)


class PngCodecTests(unittest.TestCase):
    def test_round_trip(self):
        rows = gradient(7, 5)
        self.assertEqual(decode_png(encode_png(7, 5, rows)), (7, 5, rows))

    def test_every_filter_type_and_rgb_decode_to_the_same_pixels(self):
        rows = gradient(9, 6)
        for filter_type in range(5):
            for channels in (3, 4):
                with self.subTest(filter=filter_type, channels=channels):
                    data = encode_filtered(9, 6, rows, filter_type, channels)
                    self.assertEqual(decode_png(data), (9, 6, rows))

    def test_non_png_is_refused(self):
        with self.assertRaises(ValueError):
            decode_png(b"GIF89a")


class GoldenWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.ws = Workspace(Path(self.temp.name))
        self.ws.capture("host_ready", 4, 3, solid(4, 3, (10, 20, 30, 255)))
        self.ws.capture("client_find", 3, 2, solid(3, 2, (200, 200, 200, 255)))

    def tearDown(self):
        self.temp.cleanup()

    def test_with_no_approved_baselines_every_scene_is_unreviewed_and_nothing_fails(self):
        self.assertEqual(self.ws.check(), [])

    def test_approved_baselines_matching_the_captures_pass(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["host_ready"], TODAY)
        self.assertEqual(self.ws.check(), [])
        row = (self.ws.goldens / "approved.tsv").read_text().splitlines()[-1].split("\t")
        self.assertEqual(row[0], "host_ready")
        self.assertEqual(row[4:], ["Owner", "2026-11-17"])
        self.assertTrue((self.ws.goldens / "approved.tsv").read_text().startswith("# baselines\n"))

    def test_a_changed_pixel_fails_and_publishes_the_actual_and_a_diff(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["host_ready"], TODAY)
        changed = solid(4, 3, (10, 20, 30, 255))
        changed[1] = changed[1][:4] + b"\x00\x00\x00\xff" + changed[1][8:]
        self.ws.capture("host_ready", 4, 3, changed)

        problems = self.ws.check()
        self.assertEqual(len(problems), 1)
        self.assertIn("1 of 12 pixels differ", problems[0])
        self.assertTrue((self.ws.out / "host_ready.actual.png").exists())
        _, _, diff = decode_png((self.ws.out / "host_ready.diff.png").read_bytes())
        self.assertEqual(diff[1][4:8], b"\xff\x00\x00\xff")
        self.assertNotEqual(diff[0][0:4], b"\xff\x00\x00\xff")

    def test_a_baseline_replaced_without_approval_fails(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["host_ready"], TODAY)
        (self.ws.goldens / "host_ready.png").write_bytes(encode_png(4, 3, solid(4, 3, (1, 2, 3, 255))))
        problems = self.ws.check()
        self.assertTrue(any("changed without `approve`" in problem for problem in problems))

    def test_a_baseline_with_no_approval_row_fails(self):
        (self.ws.goldens / "host_ready.png").write_bytes((self.ws.captures / "host_ready.png").read_bytes())
        problems = self.ws.check()
        self.assertTrue(any("has no approval row" in problem for problem in problems))

    def test_an_approved_scene_that_is_no_longer_captured_fails(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["client_find"], TODAY)
        (self.ws.captures / "client_find.png").unlink()
        del self.ws.scenes["client_find"]
        self.ws.capture("host_ready", 4, 3, solid(4, 3, (10, 20, 30, 255)))
        self.assertEqual(self.ws.check(), ["approved scene client_find was not captured"])

    def test_a_changed_viewport_fails(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["host_ready"], TODAY)
        self.ws.capture("host_ready", 3, 4, solid(3, 4, (10, 20, 30, 255)))
        problems = self.ws.check()
        self.assertTrue(any("is 3x4, baseline is 4x3" in problem for problem in problems))

    def test_approval_needs_a_named_approver_and_a_known_scene(self):
        with self.assertRaises(ValueError):
            approve(self.ws.captures, self.ws.goldens, "  ", ["host_ready"], TODAY)
        with self.assertRaises(ValueError):
            approve(self.ws.captures, self.ws.goldens, "Owner", ["no_such_scene"], TODAY)
        self.assertEqual(list(self.ws.goldens.glob("*.png")), [])

    def test_an_approval_row_without_an_approver_fails(self):
        approve(self.ws.captures, self.ws.goldens, "Owner", ["host_ready"], TODAY)
        path = self.ws.goldens / "approved.tsv"
        path.write_text(path.read_text().replace("\tOwner\t", "\t\t"))
        self.assertTrue(any("has no approver" in problem for problem in self.ws.check()))


if __name__ == "__main__":
    unittest.main()
