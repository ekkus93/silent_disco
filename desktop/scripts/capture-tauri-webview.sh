#!/usr/bin/env bash
# Real packaged Tauri WebKit2GTK pixel capture; no mock or golden update.
set -euo pipefail

if [[ "$1" == "--inside" ]]; then
  appimage="$2"
  output="$3"
  work="$4"
  export XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/config"
  export XDG_CACHE_HOME="$work/cache" APPIMAGE_EXTRACT_AND_RUN=1
  export GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_COMPOSITING_MODE=1
  timeout --signal=TERM 45s "$appimage" >"$work/appimage.log" 2>&1 &
  pid=$!
  trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT

  window=""
  width=0
  height=0
  for attempt in $(seq 1 120); do
    if ! kill -0 "$pid" 2>/dev/null; then
      cat "$work/appimage.log" >&2
      echo "Packaged Tauri process exited before pixel capture." >&2
      exit 1
    fi
    candidate="$(xdotool search --onlyvisible --name "Silent Disco" 2>/dev/null | head -n 1 || true)"
    if [[ -n "$candidate" ]]; then
      geometry="$(xdotool getwindowgeometry --shell "$candidate" 2>/dev/null || true)"
      width="$(printf "%s\n" "$geometry" | awk -F= '/^WIDTH=/{print $2}')"
      height="$(printf "%s\n" "$geometry" | awk -F= '/^HEIGHT=/{print $2}')"
      width="${width:-0}"
      height="${height:-0}"
      if (( width >= 720 && height >= 520 )); then
        window="$candidate"
        break
      fi
    fi
    sleep 0.25
  done
  if [[ -z "$window" ]]; then
    cat "$work/appimage.log" >&2
    echo "No visible Tauri window >=720x520 after 30s; last observed ${width}x${height}." >&2
    exit 1
  fi
  # Exercise actual packaged WebKit at both normal and compact viewports.
  # Reject blank/near-solid renders without treating them as approved goldens.
  capture_pixels() {
    local destination="$1"
    local label="$2"
    local captured_width captured_height colors
    import -window "$window" "$destination"
    read -r captured_width captured_height < <(identify -format "%w %h\\n" "$destination")
    colors="$(identify -format "%k" "$destination")"
    if (( captured_width < 720 || captured_height < 520 || colors < 64 )); then
      echo "Blank/undersized $label WebKit capture: ${captured_width}x${captured_height}, $colors colors." >&2
      exit 1
    fi
    echo "Captured $label Tauri WebKit pixels: ${captured_width}x${captured_height}, $colors colors."
    sha256sum "$destination"
  }
  sleep 1
  capture_pixels "$output" "default"
  compact_output="${output%.png}-compact.png"
  xdotool windowsize "$window" 760 560
  resized=false
  for attempt in $(seq 1 40); do
    geometry="$(xdotool getwindowgeometry --shell "$window" 2>/dev/null || true)"
    width="$(printf "%s\\n" "$geometry" | awk -F= '/^WIDTH=/{print $2}')"
    height="$(printf "%s\\n" "$geometry" | awk -F= '/^HEIGHT=/{print $2}')"
    if [[ "${width:-0}" == "760" && "${height:-0}" == "560" ]]; then
      resized=true
      break
    fi
    sleep 0.25
  done
  if [[ "$resized" != true ]]; then
    echo "Packaged WebKit window did not resize to 760x560; observed ${width:-0}x${height:-0}." >&2
    exit 1
  fi
  sleep 0.5
  capture_pixels "$compact_output" "compact"
  exit 0
fi

if [[ "$#" -ne 2 ]]; then
  echo "Usage: $0 <packaged.AppImage> <absolute-output.png>" >&2
  exit 2
fi
appimage="$(realpath "$1")"
output="$(realpath -m "$2")"
if [[ ! -f "$appimage" || "$output" != /* || "$output" != *.png ]]; then
  echo "A packaged AppImage and absolute output path are required." >&2
  exit 2
fi
for program in xvfb-run dbus-run-session xdotool import identify compare timeout; do
  command -v "$program" >/dev/null 2>&1 || { echo "Missing utility: $program" >&2; exit 2; }
done
mkdir -p "$(dirname "$output")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Launch the packaged WebKit application twice in independent Xvfb, DBus
# and XDG sessions. This tests cold-start pixel reproducibility rather than
# merely taking two frames from one already-running webview.
for pass in first repeat; do
  mkdir -p "$work/$pass/data" "$work/$pass/config" "$work/$pass/cache"
done
for pass in first repeat; do
  destination="$output"
  if [[ "$pass" == repeat ]]; then
    destination="${output%.png}-repeat.png"
  fi
  echo "Capturing packaged WebKit cold start: $pass"
  xvfb-run -a -s "-screen 0 1280x900x24" dbus-run-session -- \
    bash "$0" --inside "$appimage" "$destination" "$work/$pass"
done

# The two captures must contain identical pixels at BOTH window sizes. This
# is a reproducibility gate, not an approved golden or cross-platform parity
# assertion. Compare decoded pixels (not PNG metadata/compression bytes).
for label in default compact; do
  original="$output"
  repeated="${output%.png}-repeat.png"
  if [[ "$label" == compact ]]; then
    original="${output%.png}-compact.png"
    repeated="${output%.png}-repeat-compact.png"
  fi
  dimensions="$(identify -format '%wx%h' "$original")"
  if [[ "$dimensions" != "$(identify -format '%wx%h' "$repeated")" ]]; then
    echo "Cold-start $label WebKit viewport dimensions changed." >&2
    exit 1
  fi
  set +e
  difference="$(compare -metric AE "$original" "$repeated" null: 2>&1)"
  compare_status=$?
  set -e
  # ImageMagick 6 reports "0"; ImageMagick 7 may report "0 (0)".
  # The first field is the exact absolute-error pixel count in both.
  difference="${difference%% *}"
  if [[ "$compare_status" -gt 1 || ! "$difference" =~ ^[0-9]+$ ]]; then
    echo "Could not compare cold-start $label WebKit pixels: $difference" >&2
    exit 1
  fi
  if [[ "$difference" -ne 0 ]]; then
    compare "$original" "$repeated" "${output%.png}-$label-cold-start-diff.png" || true
    echo "Cold-start $label WebKit pixels differ at $difference positions; see diff artifact." >&2
    exit 1
  fi
  echo "Cold-start $label packaged WebKit pixels identical at $dimensions."
done
