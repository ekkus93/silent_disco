#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: validate-block47-android-interoperability.sh \
  <bundle-dir> <android-apk> <device-a-serial> <device-b-serial> [evidence-dir]

Run this on a graphical Ubuntu 22.04 amd64 machine with two physical Android
listeners attached through adb. The runner records the complete Block 47 matrix
and exits non-zero unless every acceptance item is explicitly PASS.
USAGE
}

fail() { echo "Block 47 validation failed: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"; }

if [[ $# -lt 4 || $# -gt 5 ]]; then usage; exit 2; fi
bundle_dir="$1"
apk="$2"
device_a="$3"
device_b="$4"
evidence_dir="${5:-$HOME/silent-disco-block47-evidence-$(date -u +%Y%m%dT%H%M%SZ)}"
[[ "$device_a" != "$device_b" ]] || fail "device serials must be distinct"
for cmd in adb dpkg-deb find python3 sha256sum sudo apt-get; do need "$cmd"; done
[[ -d "$bundle_dir" ]] || fail "bundle directory not found: $bundle_dir"
[[ -f "$apk" ]] || fail "APK not found: $apk"
mkdir -p "$evidence_dir"
evidence_dir="$(cd "$evidence_dir" && pwd -P)"
steps="$evidence_dir/steps.jsonl"
commands="$evidence_dir/commands.txt"
: >"$steps"; : >"$commands"

# Block 46 established this release baseline; Block 47 inherits it.
# shellcheck disable=SC1091
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 22.04 ]] \
  || fail "requires Ubuntu 22.04; found ${ID:-unknown} ${VERSION_ID:-unknown}"
[[ -n "${DISPLAY:-}" ]] || fail "DISPLAY is unset; use a graphical session"
[[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] \
  || fail "D-Bus user session is unavailable; secure-store acceptance would be invalid"

mapfile -t debs < <(find "$bundle_dir/deb" -maxdepth 1 -type f -name '*.deb' -print | sort)
mapfile -t appimages < <(find "$bundle_dir/appimage" -maxdepth 1 -type f -name '*.AppImage' -print | sort)
[[ ${#debs[@]} -eq 1 ]] || fail "expected exactly one .deb"
[[ ${#appimages[@]} -eq 1 ]] || fail "expected exactly one AppImage"
deb="${debs[0]}"; appimage="${appimages[0]}"
package="$(dpkg-deb -f "$deb" Package)"
desktop_version="$(dpkg-deb -f "$deb" Version)"
[[ "$(dpkg-deb -f "$deb" Architecture)" == amd64 ]] || fail "desktop package is not amd64"

run() {
  printf '%q ' "$@" >>"$commands"; printf '\n' >>"$commands"
  "$@"
}

prop() { adb -s "$1" shell getprop "$2" 2>/dev/null | tr -d '\r'; }
check_device() {
  local serial="$1" label="$2" state qemu model
  state="$(adb -s "$serial" get-state 2>/dev/null || true)"
  [[ "$state" == device ]] || fail "$serial is not an online adb device"
  qemu="$(prop "$serial" ro.kernel.qemu)"; model="$(prop "$serial" ro.product.model)"
  [[ "$qemu" != 1 && "$serial" != emulator-* && "$model" != sdk_gphone* ]] \
    || fail "$serial appears to be an emulator; physical hardware is required"
  {
    echo "serial=$serial"
    echo "manufacturer=$(prop "$serial" ro.product.manufacturer)"
    echo "model=$model"
    echo "android_version=$(prop "$serial" ro.build.version.release)"
    echo "api_level=$(prop "$serial" ro.build.version.sdk)"
    echo "abi=$(prop "$serial" ro.product.cpu.abi)"
    echo "fingerprint=$(prop "$serial" ro.build.fingerprint)"
  } >"$evidence_dir/device-$label.txt"
}
check_device "$device_a" a
check_device "$device_b" b

run sudo apt-get install --yes "$deb"
run adb -s "$device_a" install -r "$apk"
run adb -s "$device_b" install -r "$apk"

deb_sha="$(sha256sum "$deb" | awk '{print $1}')"
appimage_sha="$(sha256sum "$appimage" | awk '{print $1}')"
apk_sha="$(sha256sum "$apk" | awk '{print $1}')"
android_package="com.ekkus.silentdisco"
apk_version() {
  adb -s "$1" shell dumpsys package "$android_package" 2>/dev/null \
    | tr -d '\r' | awk -F= '/versionName=/{print $2; exit}'
}
apk_version_a="$(apk_version "$device_a")"; apk_version_b="$(apk_version "$device_b")"
[[ -n "$apk_version_a" && "$apk_version_a" == "$apk_version_b" ]] \
  || fail "installed Android versions are missing or differ"

read -r -p "Describe test LAN/topology (AP/router, bands/VLANs, desktop link): " topology
[[ -n "$topology" ]] || fail "network topology is required"
printf '%s\n' "$topology" >"$evidence_dir/network-topology.txt"

non_pass=0
record() {
  local id="$1" title="$2" instructions="$3" status note
  echo; echo "=== $id — $title ==="; echo "$instructions"
  while true; do
    read -r -p "Result [PASS/FAIL/BLOCKED/NOT RUN]: " status
    case "$status" in PASS|FAIL|BLOCKED|"NOT RUN") break;; *) echo "Use an exact status." >&2;; esac
  done
  read -r -p "Evidence/note: " note
  [[ -n "$note" ]] || fail "$id requires a note"
  python3 - "$steps" "$id" "$title" "$status" "$note" <<'PY'
import json,sys
path,ident,title,status,note=sys.argv[1:]
with open(path,"a",encoding="utf-8") as f:
    f.write(json.dumps({"id":ident,"title":title,"status":status,"note":note},sort_keys=True)+"\n")
PY
  [[ "$status" == PASS ]] || non_pass=$((non_pass+1))
}

record 47.manual_endpoint_join "Manual endpoint join" \
  "Launch the installed desktop package, create/use a real profile, stage a real WAV, start hosting, then have device A join by manual endpoint details."
record 47.mdns_discovery "mDNS discovery" \
  "Have device B find the packaged desktop through normal nearby discovery (not manual entry), request to join, and reach the pending/connected flow."
record 47.qr_invitation "QR invitation" \
  "Create a fresh desktop QR invitation; scan it on a physical Android listener and complete the join. Confirm stale/expired data is not silently reused."
record 47.approval_rejection "Approval and rejection" \
  "Reject a real pending listener and confirm it remains unauthorized; retry, approve it, and confirm the listener becomes connected."
record 47.one_listener_audio "One-listener audio" \
  "With one listener connected, play the staged source and confirm real network audio is audible/continuous without a local-file fallback."
record 47.two_listener_audio "Two-listener audio" \
  "With both physical listeners connected, confirm both hear the same host stream and remain acceptably synchronized side-by-side."
record 47.playback_controls "Pause/resume/stop/end" \
  "Exercise Pause, Resume, Stop, and End; both listeners must reflect each transition truthfully and together."
record 47.android_reconnect "Android disconnect/reconnect" \
  "Disrupt one Android listener only, confirm the disconnect is visible and the other listener stays truthful, then reconnect without restarting desktop."
record 47.desktop_interface_disruption "Desktop interface disruption" \
  "Reversibly disrupt the desktop LAN interface, verify failure is visible/no false delivery is claimed, restore it, and reconnect listeners."
record 47.host_source_failure "Host source failure" \
  "Make the selected staged source temporarily unavailable, attempt playback, and confirm a visible source/decode failure with no silent source substitution. Restore the source afterward."
record 47.local_monitor_failure "Local monitor failure with transmit policy" \
  "Enable local monitor, reversibly make desktop output unavailable, and confirm monitor failure is visible while Android transmission follows policy."
record 47.desktop_restart "Desktop restart" \
  "Close the installed desktop normally, relaunch the same packaged binary, and confirm the same profile opens without startup/bridge fallback."
record 47.clean_shutdown "Clean shutdown" \
  "Perform a controlled normal shutdown after the run; confirm no crash/hang and no profile/database/source loss."
record 47.reopen_profile_history "Reopen profile/session history" \
  "Reopen again and confirm the same profile and expected recent session/history evidence remain present rather than an empty replacement profile."

diagnostics="$evidence_dir/block47-diagnostics.json"
echo; echo "Export Diagnostics from the UI to exactly: $diagnostics"
read -r -p "Press Enter after the UI reports a successful export... " _
[[ -s "$diagnostics" ]] || fail "diagnostics export is missing"
python3 - "$diagnostics" "$desktop_version" <<'PY'
import json,os,sys
path,version=sys.argv[1:]
if os.path.getsize(path)>1024*1024: raise SystemExit("diagnostics exceeds 1 MiB")
with open(path,encoding="utf-8") as f: p=json.load(f)
if (p.get("versions") or {}).get("appVersion")!=version: raise SystemExit("diagnostics appVersion mismatch")
listeners=p.get("listeners")
if not isinstance(listeners,list) or len(listeners)<2: raise SystemExit("diagnostics lacks two listener records")
if p.get("listenersTruncated") is True: raise SystemExit("diagnostics listener list is truncated")
if any(not x.get("syncConfidence") for x in listeners): raise SystemExit("listener diagnostics lacks syncConfidence")
PY
diagnostics_sha="$(sha256sum "$diagnostics" | awk '{print $1}')"

read -r -p "Record measured synchronization (offset/RTT/drift and/or audible delta): " measured_sync
[[ -n "$measured_sync" ]] || fail "synchronization measurement is required"
read -r -p "Known limitations (use 'none' only if genuinely none): " limitations
[[ -n "$limitations" ]] || fail "known limitations are required"
printf '%s\n' "$measured_sync" >"$evidence_dir/measured-synchronization.txt"
printf '%s\n' "$limitations" >"$evidence_dir/known-limitations.txt"

python3 - "$evidence_dir/block47-android-interoperability-evidence.json" "$steps" \
  "$desktop_version" "$deb_sha" "$appimage_sha" "$apk_version_a" "$apk_sha" \
  "$device_a" "$device_b" "$topology" "$measured_sync" "$limitations" "$diagnostics_sha" <<'PY'
import json,sys
(out,steps,desktop_version,deb_sha,appimage_sha,apk_version,apk_sha,a,b,topology,sync,limits,diag_sha)=sys.argv[1:]
with open(steps,encoding="utf-8") as f: rows=[json.loads(x) for x in f if x.strip()]
counts={k:sum(r["status"]==k for r in rows) for k in ("PASS","FAIL","BLOCKED","NOT RUN")}
payload={"schemaVersion":1,"baseline":"ubuntu-22.04-amd64","desktopPackage":{"version":desktop_version,"debSha256":deb_sha,"appImageSha256":appimage_sha},"androidPackage":{"version":apk_version,"apkSha256":apk_sha,"deviceSerials":[a,b]},"networkTopology":topology,"measuredSynchronization":sync,"knownLimitations":limits,"diagnosticsSha256":diag_sha,"resultCounts":counts,"steps":rows}
with open(out,"w",encoding="utf-8") as f: json.dump(payload,f,indent=2,sort_keys=True); f.write("\n")
PY

echo "Evidence: $evidence_dir"
if [[ $non_pass -ne 0 ]]; then
  echo "Block 47 NOT accepted: $non_pass matrix item(s) were not PASS." >&2
  exit 1
fi
echo "All Block 47 matrix items are PASS."
