#!/usr/bin/env bash
set -euo pipefail

rustup toolchain install "${RUST_VERSION}" --profile minimal --component clippy --component rustfmt
rustup default "${RUST_VERSION}"

cargo fmt --manifest-path rust/Cargo.toml --all
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
cargo generate-lockfile --manifest-path rust/Cargo.toml
cargo generate-lockfile --manifest-path desktop/src-tauri/Cargo.toml
cp rust/Cargo.lock /tmp/block20-rust.Cargo.lock
cp desktop/src-tauri/Cargo.lock /tmp/block20-desktop.Cargo.lock

(
  cd rust
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
  printf '<!doctype html><title>Silent Disco Block 20 validation</title>\n' > dist/index.html
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
cmp /tmp/block20-rust.Cargo.lock rust/Cargo.lock
cmp /tmp/block20-desktop.Cargo.lock desktop/src-tauri/Cargo.lock
git diff --exit-code -- desktop/package-lock.json tools/decoder-spike/Cargo.lock

python3 .github/block20-complete.py

git rm --ignore-unmatch \
  .github/workflows/desktop-block20.yml \
  .github/workflows/desktop-block20-full.yml \
  .github/workflows/desktop-block20-final.yml \
  .github/block20-full-validation.sh \
  .github/block20-complete.py
git rm --ignore-unmatch .github/desktop-block20-*.py
git add \
  rust/Cargo.lock \
  desktop/src-tauri/Cargo.lock \
  docs/DESKTOP_BLOCK20_TRANSPORT_RUNTIME.md \
  docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md \
  docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md \
  memory.md
git diff --cached --check
git config user.name github-actions[bot]
git config user.email 41898282+github-actions[bot]@users.noreply.github.com
git commit -m "Complete shared transport runtime (Desktop Block 20)"
remote_sha="$(git ls-remote origin refs/heads/master | awk '{print $1}')"
test "${remote_sha}" = "$(git rev-parse HEAD^)"
git push origin HEAD:master
echo "BLOCK20_COMMIT=$(git rev-parse HEAD)" >> "${GITHUB_ENV}"
