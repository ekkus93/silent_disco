#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 <bundle-dir> <app-identifier> <main-binary>" >&2
  exit 2
fi

bundle_dir="$1"
app_identifier="$2"
main_binary="$3"

if [[ ! -d "${bundle_dir}" ]]; then
  echo "bundle directory does not exist: ${bundle_dir}" >&2
  exit 1
fi
bundle_dir="$(cd "${bundle_dir}" && pwd -P)"

mapfile -t debs < <(find "${bundle_dir}/deb" -maxdepth 1 -type f -name '*.deb' -print | sort)
mapfile -t appimages < <(find "${bundle_dir}/appimage" -maxdepth 1 -type f -name '*.AppImage' -print | sort)
if [[ "${#debs[@]}" -ne 1 || "${#appimages[@]}" -ne 1 ]]; then
  echo "expected exactly one .deb and one AppImage" >&2
  exit 1
fi

deb="${debs[0]}"
appimage="${appimages[0]}"
package="$(dpkg-deb -f "${deb}" Package)"
current_version="$(dpkg-deb -f "${deb}" Version)"

if dpkg-query -W -f='${Status}\n' "${package}" 2>/dev/null | grep -Fq 'install ok installed'; then
  echo "${package} was already installed; clean-install precondition failed" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
previous_root="${work}/previous-root"
previous_deb="${work}/${package}_0.0.0_block46_amd64.deb"
dpkg-deb -R "${deb}" "${previous_root}" >/dev/null
sed -i 's/^Version:.*/Version: 0.0.0/' "${previous_root}/DEBIAN/control"
dpkg-deb --root-owner-group -b "${previous_root}" "${previous_deb}" >/dev/null

sudo apt-get install --yes "${previous_deb}"
test -x "/usr/bin/${main_binary}"

data_root="${HOME}/.local/share/${app_identifier}"
sentinel="${data_root}/block46-package-lifecycle/sentinel.txt"
mkdir -p "$(dirname "${sentinel}")"
printf '%s\n' "preserve-across-upgrade-and-uninstall" > "${sentinel}"

sudo apt-get install --yes "${deb}"
installed_version="$(dpkg-query -W -f='${Version}' "${package}")"
if [[ "${installed_version}" != "${current_version}" ]]; then
  echo "upgrade installed ${installed_version}; expected ${current_version}" >&2
  exit 1
fi
grep -Fxq 'preserve-across-upgrade-and-uninstall' "${sentinel}"

desktop_entry="/usr/share/applications/Silent Disco.desktop"
icon="/usr/share/icons/hicolor/512x512/apps/${main_binary}.png"
test -f "${desktop_entry}"
test -f "${icon}"

launch_log="${work}/installed-launch.log"
set +e
timeout --signal=TERM 12s dbus-run-session -- xvfb-run -a "/usr/bin/${main_binary}" >"${launch_log}" 2>&1
launch_status="$?"
set -e
if [[ "${launch_status}" -ne 124 ]]; then
  cat "${launch_log}" >&2
  echo "installed package exited before the 12-second no-dev-server launch smoke (status ${launch_status})" >&2
  exit 1
fi

appimage_log="${work}/appimage-launch.log"
chmod +x "${appimage}"
set +e
APPIMAGE_EXTRACT_AND_RUN=1 timeout --signal=TERM 20s dbus-run-session -- xvfb-run -a "${appimage}" >"${appimage_log}" 2>&1
appimage_status="$?"
set -e
if [[ "${appimage_status}" -ne 124 ]]; then
  cat "${appimage_log}" >&2
  echo "AppImage exited before the 20-second no-dev-server launch smoke (status ${appimage_status})" >&2
  exit 1
fi

sudo apt-get remove --yes "${package}"
if dpkg-query -W -f='${Status}\n' "${package}" 2>/dev/null | grep -Fq 'install ok installed'; then
  echo "${package} remained installed after removal" >&2
  exit 1
fi
test ! -e "/usr/bin/${main_binary}"
test ! -e "${desktop_entry}"
test ! -e "${icon}"
grep -Fxq 'preserve-across-upgrade-and-uninstall' "${sentinel}"

rm -rf "${data_root}/block46-package-lifecycle"
printf 'Linux package lifecycle passed: clean install, synthetic-version upgrade, no-dev-server launch, uninstall, user-data preservation.\n'
