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
for program in xvfb-run dbus-run-session xdotool import identify timeout; do
  command -v "$program" >/dev/null 2>&1 || { echo "Missing utility: $program" >&2; exit 2; }
done
mkdir -p "$(dirname "$output")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/data" "$work/config" "$work/cache"
# Fresh XDG roots prevent screenshots from exposing an owner's saved profiles.
xvfb-run -a -s "-screen 0 1280x900x24" dbus-run-session -- \
  bash "$0" --inside "$appimage" "$output" "$work"
