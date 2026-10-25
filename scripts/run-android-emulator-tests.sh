#!/usr/bin/env bash
# Runs inside the GitHub-hosted Android emulator runner after boot.
# A separate script avoids nested shell quoting in the runner's script input.
set -euo pipefail

./gradlew connectedDebugAndroidTest --stacktrace --console=plain \
  2>&1 | tee android-instrumentation-tests.log

# Gradle's connected test task uninstalls its test APK (and may uninstall the
# target app) during teardown, removing app-specific external screenshot files.
# Reinstall and run the screenshot fixture directly so the pull happens while
# the target package is still installed. This is separate from the full suite.
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell am instrument -w -r \
  -e class com.ekkus.silentdisco.WorkflowScreenshotCaptureTest \
  com.ekkus.silentdisco.test/androidx.test.runner.AndroidJUnitRunner \
  | tee android-screenshot-capture.log
grep -Eq 'OK \(1 test\)' android-screenshot-capture.log

mkdir -p android-workflow-screenshots
adb pull \
  /sdcard/Android/data/com.ekkus.silentdisco/files/workflow-screenshots/. \
  android-workflow-screenshots/

python3 - <<'PY'
import hashlib
from pathlib import Path
import struct

root = Path("android-workflow-screenshots")
expected = {
    "host_queue_empty": (360, 640),
    "host_queue_playing": (360, 640),
    "host_queue_large_text": (320, 480),
    "host_live_playing": (360, 640),
    "host_live_paused": (360, 640),
    "host_live_landscape": (640, 360),
    "host_pending_approval": (360, 640),
    "client_recovery_large_text": (320, 480),
    "startup_recovery_large_text": (320, 480),
    "startup_fatal": (360, 640),
    "client_stage_find": (360, 640),
    "client_stage_connect": (360, 640),
    "client_stage_listen": (360, 640),
}
manifest = (root / "manifest.tsv").read_text().splitlines()
assert manifest[0] == "scene\twidth_px\theight_px\tsha256", "invalid screenshot manifest header"
assert len(manifest) == len(expected) + 1, "missing or extra screenshot manifest rows"
seen = set()
digests = set()
for line in manifest[1:]:
    scene, width, height, digest = line.split("\t")
    assert scene in expected and scene not in seen, f"unexpected/duplicate scene: {scene}"
    seen.add(scene)
    data = (root / f"{scene}.png").read_bytes()
    assert data.startswith(b"\x89PNG\r\n\x1a\n"), f"not PNG: {scene}"
    dimensions = struct.unpack(">II", data[16:24])
    assert dimensions == expected[scene] == (int(width), int(height)), f"wrong viewport: {scene}"
    assert hashlib.sha256(data).hexdigest() == digest, f"hash mismatch: {scene}"
    assert digest not in digests, f"duplicate screenshot content: {scene}"
    digests.add(digest)
assert seen == set(expected)
assert {p.name for p in root.iterdir()} == {"manifest.tsv"} | {
    f"{scene}.png" for scene in expected
}, "unexpected screenshot files"
print(f"Validated {len(expected)} distinct Android workflow PNGs and SHA-256 manifest")
PY
