#!/usr/bin/env bash
set -euo pipefail

rustup toolchain install "${RUST_VERSION}" --profile minimal --component clippy --component rustfmt
rustup default "${RUST_VERSION}"

cargo fmt --manifest-path rust/Cargo.toml --all
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
cargo generate-lockfile --manifest-path rust/Cargo.toml
cargo generate-lockfile --manifest-path desktop/src-tauri/Cargo.toml
cp rust/Cargo.lock /tmp/block22-rust.Cargo.lock
cp desktop/src-tauri/Cargo.lock /tmp/block22-desktop.Cargo.lock

(
  cd rust
  cargo test -p silent-disco-core pending_control_peer_receives_hello_before_datagram_authorization -- --nocapture
  cargo test -p silent-disco-core transport -- --nocapture
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --all-features
)

bash scripts/check-source-file-line-counts.sh

"${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager" \
  "platforms;android-36" "build-tools;36.0.0" "ndk;28.2.13676358"
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential curl file libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev libwebkit2gtk-4.1-dev libxdo-dev patchelf time wget

echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' \
  | sudo tee /etc/udev/rules.d/99-kvm4all.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=kvm

(
  cd desktop
  mkdir -p dist
  printf '<!doctype html><title>Silent Disco Block 22 validation</title>\n' > dist/index.html
  npm ci
  npm run bindings:check
  npm run format:check
  npm run lint
  npm run typecheck
  npm test -- --run
  npm run build
)

(
  cd desktop/src-tauri
  cargo test platform::host_transport_tests -- --nocapture
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo check --all-features
)

(
  cd desktop
  npm run tauri build
)

./gradlew assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  --stacktrace --console=plain

for apk in \
  app/build/outputs/apk/debug/app-debug.apk \
  app/build/outputs/apk/pocDebug/app-pocDebug.apk \
  app/build/outputs/apk/release/app-release-unsigned.apk; do
  test -f "${apk}"
  for abi in armeabi-v7a arm64-v8a x86 x86_64; do
    unzip -Z1 "${apk}" "lib/${abi}/libsilent_disco_ffi.so" \
      | grep -Fx "lib/${abi}/libsilent_disco_ffi.so"
  done
done

./gradlew test lintDebug --stacktrace --console=plain
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain

cargo generate-lockfile --manifest-path rust/Cargo.toml
cargo generate-lockfile --manifest-path desktop/src-tauri/Cargo.toml
cmp /tmp/block22-rust.Cargo.lock rust/Cargo.lock
cmp /tmp/block22-desktop.Cargo.lock desktop/src-tauri/Cargo.lock
git diff --exit-code -- desktop/package-lock.json tools/decoder-spike/Cargo.lock

python3 .github/block22-complete.py

git rm --ignore-unmatch \
  .github/workflows/desktop-block22-final.yml \
  .github/block22-full-validation.sh \
  .github/block22-complete.py
git rm --ignore-unmatch .github/workflows/desktop-block22-*.yml
git rm --ignore-unmatch .github/apply-block22-*.py
git rm --ignore-unmatch .github/block22-runtime.py.gz.b64 .github/block22-runtime.py.part-00
git add \
  rust/Cargo.lock \
  desktop/src-tauri/Cargo.lock \
  docs/DESKTOP_BLOCK22_MANUAL_ENDPOINT_HOST_WORKFLOW.md \
  docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md \
  memory.md
git diff --cached --check
git config user.name github-actions[bot]
git config user.email 41898282+github-actions[bot]@users.noreply.github.com
git commit -m "Complete manual endpoint host workflow (Desktop Block 22)"
remote_sha="$(git ls-remote origin refs/heads/master | awk '{print $1}')"
test "${remote_sha}" = "$(git rev-parse HEAD^)"
git push origin HEAD:master
echo "BLOCK22_COMMIT=$(git rev-parse HEAD)" >> "${GITHUB_ENV}"
