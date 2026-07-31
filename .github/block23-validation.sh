#!/usr/bin/env bash
set -euo pipefail

test -f .github/apply-block23.py
python3 .github/apply-block23.py
rm .github/apply-block23.py

rustup toolchain install "${RUST_VERSION}" --profile minimal --component clippy --component rustfmt
rustup default "${RUST_VERSION}"

sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential curl file libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev libwebkit2gtk-4.1-dev libxdo-dev patchelf time wget

cp rust/Cargo.lock /tmp/block23-rust.Cargo.lock
cp desktop/src-tauri/Cargo.lock /tmp/block23-desktop.Cargo.lock
cp desktop/package-lock.json /tmp/block23-package-lock.json

cargo fmt --manifest-path rust/Cargo.toml --all
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
(
  cd desktop
  npm ci
  npm run bindings:generate
  npm run format
)

cargo generate-lockfile --manifest-path rust/Cargo.toml
cargo generate-lockfile --manifest-path desktop/src-tauri/Cargo.toml
cmp /tmp/block23-rust.Cargo.lock rust/Cargo.lock
cmp /tmp/block23-desktop.Cargo.lock desktop/src-tauri/Cargo.lock
cmp /tmp/block23-package-lock.json desktop/package-lock.json

git diff --check
bash scripts/check-source-file-line-counts.sh

(
  cd desktop/src-tauri
  cargo test host_commands::tests -- --nocapture
  cargo test host_session_dto::tests -- --nocapture
  cargo test platform::storage_effect_runner_tests -- --nocapture
  cargo test platform::host_transport_admission_tests -- --nocapture
  cargo test platform::host_transport_tests -- --nocapture
)
(
  cd desktop
  npm test -- --run src/screens/HostSessionScreen.test.tsx
)

(
  cd rust
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --all-features
)

(
  cd desktop
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

"${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager" \
  "platforms;android-36" "build-tools;36.0.0" "ndk;28.2.13676358"
./gradlew assembleDebug assemblePocDebug assembleRelease assembleDebugAndroidTest \
  --stacktrace --console=plain
./gradlew test lintDebug --stacktrace --console=plain

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

echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' \
  | sudo tee /etc/udev/rules.d/99-kvm4all.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=kvm
./gradlew pixel2api29DebugAndroidTest \
  -Pandroid.testoptions.manageddevices.emulator.gpu=software \
  --stacktrace --console=plain

cargo generate-lockfile --manifest-path rust/Cargo.toml
cargo generate-lockfile --manifest-path desktop/src-tauri/Cargo.toml
cmp /tmp/block23-rust.Cargo.lock rust/Cargo.lock
cmp /tmp/block23-desktop.Cargo.lock desktop/src-tauri/Cargo.lock
cmp /tmp/block23-package-lock.json desktop/package-lock.json
git diff --check
bash scripts/check-source-file-line-counts.sh

python3 .github/block23-complete.py

git rm --ignore-unmatch \
  .github/workflows/desktop-block23.yml \
  .github/workflows/desktop-block23-inspect.yml \
  .github/block23-adapt.py \
  .github/block23-validation.sh \
  .github/block23-complete.py \
  .github/apply-block23.py.part-*

python3 - <<'PY'
allowed = {
    'desktop/src-tauri/src/app_state.rs',
    'desktop/src-tauri/src/bindings.rs',
    'desktop/src-tauri/src/host_commands.rs',
    'desktop/src-tauri/src/host_session_dto.rs',
    'desktop/src-tauri/src/lib.rs',
    'desktop/src-tauri/src/runtime_dto.rs',
    'desktop/src-tauri/src/shutdown.rs',
    'desktop/src-tauri/src/platform/effect_runner.rs',
    'desktop/src-tauri/src/platform/host_transport.rs',
    'desktop/src-tauri/src/platform/host_transport_tests.rs',
    'desktop/src-tauri/src/platform/host_transport_admission_tests.rs',
    'desktop/src-tauri/src/platform/mod.rs',
    'desktop/src-tauri/src/platform/network.rs',
    'desktop/src-tauri/src/platform/storage_effect_runner.rs',
    'desktop/src-tauri/src/platform/storage_effect_runner_tests.rs',
    'desktop/src/core/client.ts',
    'desktop/src/core/generated/desktop-bindings.ts',
    'desktop/src/screens/HostSessionScreen.tsx',
    'desktop/src/screens/HostSessionScreen.test.tsx',
    'desktop/src/screens/ListenerDetailScreen.tsx',
    'docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md',
    'docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md',
    'memory.md',
    '.github/workflows/desktop-block23.yml',
    '.github/workflows/desktop-block23-inspect.yml',
    '.github/block23-adapt.py',
    '.github/block23-validation.sh',
    '.github/block23-complete.py',
    '.github/apply-block23.py.part-00',
    '.github/apply-block23.py.part-01',
    '.github/apply-block23.py.part-02',
    '.github/apply-block23.py.part-03',
    '.github/apply-block23.py.part-04',
}
import subprocess
changed = set(subprocess.check_output(['git', 'status', '--short'], text=True).splitlines())
paths = {line[3:] for line in changed if len(line) > 3}
unexpected = sorted(paths - allowed)
if unexpected:
    raise SystemExit(f'unexpected Block 23 changes: {unexpected}')
PY

git add -A
git diff --cached --check
git config user.name github-actions[bot]
git config user.email 41898282+github-actions[bot]@users.noreply.github.com
git commit -m "Complete desktop listener management (Desktop Block 23)"
remote_sha="$(git ls-remote origin refs/heads/master | awk '{print $1}')"
test "${remote_sha}" = "$(git rev-parse HEAD^)"
git push origin HEAD:master
echo "BLOCK23_COMMIT=$(git rev-parse HEAD)" >> "${GITHUB_ENV}"
