#!/usr/bin/env bash
# Runs inside the GitHub-hosted Android emulator runner after boot.
# A separate script avoids nested shell quoting in the runner's script input.
set -euo pipefail

./gradlew connectedDebugAndroidTest --stacktrace --console=plain \
  2>&1 | tee android-instrumentation-tests.log

mkdir -p android-workflow-screenshots
adb pull \
  /sdcard/Android/data/com.ekkus.silentdisco/files/workflow-screenshots/. \
  android-workflow-screenshots/

test -s android-workflow-screenshots/manifest.tsv
png_count=$(find android-workflow-screenshots -type f -name '*.png' | wc -l)
if [[ "$png_count" -ne 9 ]]; then
  printf 'Expected 9 Android workflow screenshots; found %s\n' "$png_count" >&2
  exit 1
fi
