#!/usr/bin/env bash
# Retry transient corrupt NDK downloads on disposable GitHub-hosted runners.
# Do not mask persistent SDK installation failures or use an unverified NDK.
set -euo pipefail

sdkmanager="${ANDROID_HOME:?ANDROID_HOME must be set}/cmdline-tools/latest/bin/sdkmanager"
ndk_dir="${ANDROID_HOME}/ndk/28.2.13676358"
for attempt in 1 2 3; do
  echo "Installing pinned Android SDK packages (attempt ${attempt}/3)"
  if "$sdkmanager" \
    "platforms;android-36" \
    "build-tools;36.0.0" \
    "ndk;28.2.13676358" \
    && [[ -s "$ndk_dir/source.properties" ]]; then
    grep -Fxq 'Pkg.Revision = 28.2.13676358' "$ndk_dir/source.properties" || {
      echo "NDK version metadata does not match pinned revision." >&2
      exit 1
    }
    exit 0
  fi
  if (( attempt == 3 )); then
    echo "Pinned Android SDK/NDK installation failed after three attempts." >&2
    exit 1
  fi
  # sdkmanager sometimes leaves an incomplete directory after reporting
  # 'Error on ZipFile unknown archive'. Only remove that incomplete install.
  if [[ -d "$ndk_dir" && ! -s "$ndk_dir/source.properties" ]]; then
    echo "Removing incomplete NDK directory before retry."
    rm -rf -- "$ndk_dir"
  fi
  sleep "$(( attempt * 5 ))"
done
