#!/usr/bin/env bash
# Runs inside the GitHub-hosted Android emulator runner after boot.
# A separate script avoids nested shell quoting in the runner's script input.
set -euo pipefail

./gradlew connectedDebugAndroidTest --stacktrace --console=plain \
  2>&1 | tee android-instrumentation-tests.log

# Gradle's connected test task uninstalls its test APK (and may uninstall the
# target app) during teardown, removing app-specific external screenshot files.
# Reinstall, then capture the reviewed states in TWO independent instrumentation
# processes. A second process catches nondeterministic visual state that a
# reverse-order rerender within one composition cannot detect.
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk

capture_screenshots() {
  local log="$1"
  local destination="$2"
  adb shell am instrument -w -r \
    -e class com.ekkus.silentdisco.WorkflowScreenshotCaptureTest \
    com.ekkus.silentdisco.test/androidx.test.runner.AndroidJUnitRunner \
    | tee "$log"
  grep -Eq 'OK \(1 test\)' "$log"
  mkdir -p "$destination"
  adb pull \
    /sdcard/Android/data/com.ekkus.silentdisco/files/workflow-screenshots/. \
    "$destination/"
}

mkdir -p android-workflow-screenshots
capture_screenshots android-screenshot-capture.log android-workflow-screenshots
# Force a cold process start without clearing the app's data.
adb shell am force-stop com.ekkus.silentdisco
adb shell am force-stop com.ekkus.silentdisco.test
capture_screenshots android-screenshot-capture-repeat.log android-workflow-screenshots/repeated

python3 - <<'PY'
import hashlib
from pathlib import Path
import struct

root = Path("android-workflow-screenshots")
repeat = root / "repeated"
expected = {
    "host_music_empty": (360, 640),
    "host_music_filled": (360, 640),
    "host_access_code": (360, 640),
    "host_ready": (360, 640),
    "host_ready_permission_recovery": (320, 480),
    "host_queue_empty": (360, 640),
    "host_queue_playing": (360, 640),
    "host_queue_large_text": (320, 480),
    "host_live_playing": (360, 640),
    "host_live_paused": (360, 640),
    "host_live_buffering": (360, 640),
    "host_live_playback_error": (360, 640),
    "host_live_queue_ended": (360, 640),
    "host_live_landscape": (640, 360),
    "host_pending_approval": (360, 640),
    "client_invite_code": (360, 640),
    "client_invite_code_large_text": (320, 480),
    "client_recovery_large_text": (320, 480),
    "startup_recovery_large_text": (320, 480),
    "startup_fatal": (360, 640),
    "client_stage_find": (360, 640),
    "client_stage_connect": (360, 640),
    "client_stage_listen": (360, 640),
    "client_manual_compact": (320, 400),
    "client_manual_landscape": (640, 360),
}
def validate_pass(directory, extra_files=()):
    manifest = (directory / "manifest.tsv").read_text().splitlines()
    assert manifest[0] == "scene\twidth_px\theight_px\tsha256", "invalid screenshot manifest header"
    assert len(manifest) == len(expected) + 1, "missing or extra screenshot manifest rows"
    seen = {}
    digests = set()
    for line in manifest[1:]:
        scene, width, height, digest = line.split("\t")
        assert scene in expected and scene not in seen, f"unexpected/duplicate scene: {scene}"
        data = (directory / f"{scene}.png").read_bytes()
        assert data.startswith(b"\x89PNG\r\n\x1a\n"), f"not PNG: {scene}"
        dimensions = struct.unpack(">II", data[16:24])
        assert dimensions == expected[scene] == (int(width), int(height)), f"wrong viewport: {scene}"
        assert hashlib.sha256(data).hexdigest() == digest, f"hash mismatch: {scene}"
        assert digest not in digests, f"duplicate screenshot content: {scene}"
        digests.add(digest)
        seen[scene] = digest
    assert set(seen) == set(expected), "missing reviewed scenes"
    assert {p.name for p in directory.iterdir()} == {"manifest.tsv"} | {
        f"{scene}.png" for scene in expected
    } | set(extra_files), "unexpected screenshot files"
    return seen

# Every invocation starts a fresh instrumentation process. The exact pixel
# bytes must match across processes, not just within a single composition.
first = validate_pass(root, ("repeated",))
second = validate_pass(repeat)
for scene in expected:
    assert first[scene] == second[scene], (
        f"pixel mismatch across independent instrumentation processes: {scene}"
    )

print(f"Validated {len(expected)} Android workflow scenes twice across cold processes with identical pixels")
PY
