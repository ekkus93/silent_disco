#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: validate-block46-fresh-machine.sh <bundle-dir> [evidence-dir]

Runs the Block 46.3 fresh-machine acceptance sequence against the packaged
Linux desktop. This is intentionally interactive because one acceptance step
requires a physical Android listener and two steps use native file dialogs.
Run it from a graphical Ubuntu 22.04 desktop/VM with a Secret Service provider.
USAGE
}

fail() {
  echo "Block 46.3 validation failed: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

require_pass() {
  local prompt="$1"
  local answer
  printf '%s\nType PASS to record this manual acceptance step: ' "${prompt}"
  IFS= read -r answer
  [[ "${answer}" == "PASS" ]] || fail "manual acceptance was not confirmed"
}

if [[ "$#" -lt 1 || "$#" -gt 2 ]]; then
  usage
  exit 2
fi

bundle_dir="$1"
evidence_dir="${2:-block46-fresh-machine-evidence}"

for command in dpkg-deb dpkg-query find python3 sha256sum uname; do
  require_command "${command}"
done
require_command sudo
require_command apt-get

[[ -d "${bundle_dir}" ]] || fail "bundle directory does not exist: ${bundle_dir}"
bundle_dir="$(cd "${bundle_dir}" && pwd -P)"
mkdir -p "${evidence_dir}"
evidence_dir="$(cd "${evidence_dir}" && pwd -P)"

mapfile -t debs < <(find "${bundle_dir}/deb" -maxdepth 1 -type f -name '*.deb' -print | sort)
mapfile -t appimages < <(find "${bundle_dir}/appimage" -maxdepth 1 -type f -name '*.AppImage' -print | sort)
[[ "${#debs[@]}" -eq 1 ]] || fail "expected exactly one .deb in ${bundle_dir}/deb"
[[ "${#appimages[@]}" -eq 1 ]] || fail "expected exactly one AppImage in ${bundle_dir}/appimage"

deb="${debs[0]}"
appimage="${appimages[0]}"
package="$(dpkg-deb -f "${deb}" Package)"
version="$(dpkg-deb -f "${deb}" Version)"
architecture="$(dpkg-deb -f "${deb}" Architecture)"
main_binary="silent-disco-desktop"
app_identifier="com.ekkus.silentdisco.desktop"

# Block 46 selected Ubuntu 22.04 as the package/fresh-machine baseline. An
# exploratory run elsewhere is useful, but it must not silently count as the
# release acceptance run.
# shellcheck disable=SC1091
source /etc/os-release
if [[ "${ID:-}" != "ubuntu" || "${VERSION_ID:-}" != "22.04" ]]; then
  fail "acceptance baseline is Ubuntu 22.04; found ${ID:-unknown} ${VERSION_ID:-unknown}"
fi
[[ "${architecture}" == "amd64" ]] || fail "acceptance package must be amd64; got ${architecture}"
[[ -n "${DISPLAY:-}" ]] || fail "DISPLAY is unset; run from a graphical desktop session"
[[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || fail "D-Bus session is unavailable"

if dpkg-query -W -f='${Status}\n' "${package}" 2>/dev/null | grep -Fq 'install ok installed'; then
  fail "${package} is already installed; fresh-machine clean-install precondition is not met"
fi

# Isolate application file state from any ordinary user profile while still
# using the machine's real graphical session and secure-store provider.
app_xdg_data_home="${evidence_dir}/xdg-data"
app_xdg_config_home="${evidence_dir}/xdg-config"
app_xdg_cache_home="${evidence_dir}/xdg-cache"
mkdir -p "${app_xdg_data_home}" "${app_xdg_config_home}" "${app_xdg_cache_home}"
app_data_root="${app_xdg_data_home}/${app_identifier}"
profile_root="${app_data_root}/profiles/main"
profile_database="${profile_root}/silent-disco.sqlite3"
sources_dir="${profile_root}/sources"
diagnostics_export="${evidence_dir}/block46-diagnostics.json"

[[ ! -e "${profile_root}" ]] || fail "isolated main profile already exists: ${profile_root}"
[[ ! -e "${diagnostics_export}" ]] || fail "diagnostics destination already exists: ${diagnostics_export}"

source_wav="${evidence_dir}/block46-source.wav"
python3 - "${source_wav}" <<'PY'
import math
import struct
import sys
import wave

path = sys.argv[1]
sample_rate = 48_000
frames = sample_rate
with wave.open(path, "wb") as output:
    output.setnchannels(2)
    output.setsampwidth(2)
    output.setframerate(sample_rate)
    for index in range(frames):
        sample = int(0.15 * 32767 * math.sin(2 * math.pi * 440 * index / sample_rate))
        output.writeframesraw(struct.pack("<hh", sample, sample))
PY
source_sha256="$(sha256sum "${source_wav}" | awk '{print $1}')"

deb_sha256="$(sha256sum "${deb}" | awk '{print $1}')"
appimage_sha256="$(sha256sum "${appimage}" | awk '{print $1}')"
{
  printf 'timestamp_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'package=%s\n' "${package}"
  printf 'version=%s\n' "${version}"
  printf 'architecture=%s\n' "${architecture}"
  printf 'deb_sha256=%s\n' "${deb_sha256}"
  printf 'appimage_sha256=%s\n' "${appimage_sha256}"
  printf 'source_sha256=%s\n' "${source_sha256}"
  printf 'os=%s %s\n' "${ID}" "${VERSION_ID}"
  uname -a
} > "${evidence_dir}/environment.txt"

sudo apt-get install --yes "${deb}"
dpkg-query -W -f='${Status}\n' "${package}" | grep -Fqx 'install ok installed' \
  || fail "package manager did not report ${package} installed"
[[ -x "/usr/bin/${main_binary}" ]] || fail "installed main binary is missing"

launch_app() {
  local log="$1"
  env \
    XDG_DATA_HOME="${app_xdg_data_home}" \
    XDG_CONFIG_HOME="${app_xdg_config_home}" \
    XDG_CACHE_HOME="${app_xdg_cache_home}" \
    "/usr/bin/${main_binary}" >"${log}" 2>&1 &
  APP_PID="$!"
}

wait_for_profile_open() {
  local log="$1"
  for _ in $(seq 1 120); do
    if ! kill -0 "${APP_PID}" 2>/dev/null; then
      status=0
      wait "${APP_PID}" || status="$?"
      cat "${log}" >&2 || true
      fail "packaged app exited before the main profile opened (status ${status})"
    fi
    if [[ -s "${profile_database}" && -d "${sources_dir}" ]]; then
      return 0
    fi
    sleep 0.25
  done
  cat "${log}" >&2 || true
  fail "main profile did not open within 30 seconds"
}

wait_for_controlled_exit() {
  local log="$1"
  for _ in $(seq 1 120); do
    if ! kill -0 "${APP_PID}" 2>/dev/null; then
      status=0
      wait "${APP_PID}" || status="$?"
      [[ "${status}" -eq 0 ]] || {
        cat "${log}" >&2 || true
        fail "packaged app returned ${status} during controlled shutdown"
      }
      return 0
    fi
    sleep 0.25
  done
  cat "${log}" >&2 || true
  fail "packaged app did not exit within 30 seconds after window close"
}

first_launch_log="${evidence_dir}/first-launch.log"
launch_app "${first_launch_log}"
wait_for_profile_open "${first_launch_log}"

echo
echo "Fresh profile opened successfully at the production app-local-data layout."
echo "Generated validation source: ${source_wav}"
echo "In Silent Disco, click 'Select audio file' and choose that exact WAV."
read -r -p "Press Enter after the UI reports the source as selected and authoritative... " _

mapfile -t staged_sources < <(find "${sources_dir}" -maxdepth 1 -type f ! -name '.*' -print | sort)
[[ "${#staged_sources[@]}" -eq 1 ]] || fail "expected exactly one committed staged source; found ${#staged_sources[@]}"
staged_sha256="$(sha256sum "${staged_sources[0]}" | awk '{print $1}')"
[[ "${staged_sha256}" == "${source_sha256}" ]] \
  || fail "staged source bytes do not match the selected validation WAV"

require_pass "Create a host session, join it from a physical Android listener, approve it if required, and confirm the desktop reports that listener as connected."

cat <<INSTRUCTIONS
Open Diagnostics in the desktop app and export diagnostics to this exact path:
  ${diagnostics_export}
INSTRUCTIONS
read -r -p "Press Enter after the diagnostics save operation reports success... " _
[[ -s "${diagnostics_export}" ]] || fail "diagnostics export was not created"
python3 - "${diagnostics_export}" <<'PY'
import json
import os
import sys

path = sys.argv[1]
if os.path.getsize(path) > 1024 * 1024:
    raise SystemExit("diagnostics export exceeds the 1 MiB Block 35 bound")
with open(path, encoding="utf-8") as handle:
    payload = json.load(handle)
if not isinstance(payload, dict):
    raise SystemExit("diagnostics export is not a JSON object")
PY
diagnostics_sha256="$(sha256sum "${diagnostics_export}" | awk '{print $1}')"

read -r -p "Close the Silent Disco window normally, then press Enter... " _
wait_for_controlled_exit "${first_launch_log}"
[[ -s "${profile_database}" ]] || fail "profile database disappeared after shutdown"
[[ -s "${staged_sources[0]}" ]] || fail "staged source disappeared after shutdown"

second_launch_log="${evidence_dir}/second-launch.log"
launch_app "${second_launch_log}"
wait_for_profile_open "${second_launch_log}"
require_pass "Confirm the reopened app reaches Host setup without a bridge/profile startup error."
read -r -p "Close the reopened Silent Disco window normally, then press Enter... " _
wait_for_controlled_exit "${second_launch_log}"

sudo apt-get remove --yes "${package}"
if dpkg-query -W -f='${Status}\n' "${package}" 2>/dev/null | grep -Fq 'install ok installed'; then
  fail "${package} remained installed after removal"
fi
[[ ! -e "/usr/bin/${main_binary}" ]] || fail "main binary remained after uninstall"
[[ -s "${profile_database}" ]] || fail "uninstall removed the profile database"
[[ -s "${staged_sources[0]}" ]] || fail "uninstall removed the staged source"

python3 - \
  "${evidence_dir}/block46-fresh-machine-evidence.json" \
  "${version}" \
  "${deb_sha256}" \
  "${appimage_sha256}" \
  "${source_sha256}" \
  "${staged_sha256}" \
  "${diagnostics_sha256}" <<'PY'
import json
import sys
from datetime import datetime, timezone

(
    output,
    version,
    deb_sha256,
    appimage_sha256,
    source_sha256,
    staged_sha256,
    diagnostics_sha256,
) = sys.argv[1:]
payload = {
    "schemaVersion": 1,
    "completedAtUtc": datetime.now(timezone.utc).isoformat(),
    "baseline": "ubuntu-22.04-amd64",
    "packageVersion": version,
    "debSha256": deb_sha256,
    "appImageSha256": appimage_sha256,
    "profileCreated": True,
    "sourceStaged": True,
    "sourceSha256": source_sha256,
    "stagedSourceSha256": staged_sha256,
    "physicalAndroidListenerConfirmed": True,
    "diagnosticsExported": True,
    "diagnosticsSha256": diagnostics_sha256,
    "controlledShutdownCompleted": True,
    "profileReopenConfirmed": True,
    "uninstallPreservedProfileData": True,
}
with open(output, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY

cat <<DONE
Block 46.3 fresh-machine validation completed.
Evidence directory:
  ${evidence_dir}
Evidence ledger:
  ${evidence_dir}/block46-fresh-machine-evidence.json
DONE
