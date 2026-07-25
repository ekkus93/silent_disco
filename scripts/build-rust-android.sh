#!/usr/bin/env bash
set -euo pipefail

readonly CARGO_NDK_VERSION="4.1.2"
readonly ANDROID_NDK_VERSION="28.2.13676358"
readonly ANDROID_PLATFORM="29"
readonly RUST_TOOLCHAIN="1.97.1"

usage() {
    echo "Usage: $0 <debug|release> <absolute-output-directory>" >&2
}

if [[ $# -ne 2 ]]; then
    usage
    exit 64
fi

profile="$1"
output_dir="$2"

case "$profile" in
    debug|release) ;;
    *)
        echo "Unsupported Rust Android profile: $profile" >&2
        usage
        exit 64
        ;;
esac

if [[ "$output_dir" != /* ]]; then
    echo "Rust Android output directory must be absolute: $output_dir" >&2
    exit 64
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rust_root="$repo_root/rust"
expected_output_root="$repo_root/app/build/generated/rustJniLibs"

case "$output_dir" in
    "$expected_output_root"/*) ;;
    *)
        echo "Refusing to write outside $expected_output_root: $output_dir" >&2
        exit 64
        ;;
esac

sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$sdk_root" && -f "$repo_root/local.properties" ]]; then
    sdk_root="$(sed -n 's/^sdk\.dir=//p' "$repo_root/local.properties" | tail -n 1 | sed 's/\\:/:/g; s/\\\\/\\/g')"
fi
if [[ -z "$sdk_root" ]]; then
    echo "ANDROID_SDK_ROOT or ANDROID_HOME must identify the Android SDK." >&2
    exit 69
fi

ndk_root="$sdk_root/ndk/$ANDROID_NDK_VERSION"
if [[ ! -d "$ndk_root" ]]; then
    echo "Required Android NDK $ANDROID_NDK_VERSION was not found at $ndk_root" >&2
    exit 69
fi

export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_HOME="$sdk_root"
export ANDROID_NDK_HOME="$ndk_root"
export ANDROID_NDK_ROOT="$ndk_root"

cd "$rust_root"
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal
rustup target add \
    --toolchain "$RUST_TOOLCHAIN" \
    aarch64-linux-android \
    armv7-linux-androideabi \
    i686-linux-android \
    x86_64-linux-android

tools_root="$repo_root/.gradle/rust-tools/cargo-ndk-$CARGO_NDK_VERSION"
cargo_ndk="$tools_root/bin/cargo-ndk"
if [[ ! -x "$cargo_ndk" ]]; then
    cargo +"$RUST_TOOLCHAIN" install \
        cargo-ndk \
        --version "$CARGO_NDK_VERSION" \
        --locked \
        --root "$tools_root"
fi

rm -rf "$output_dir"
mkdir -p "$output_dir"

cargo_args=(
    ndk
    --platform "$ANDROID_PLATFORM"
    -t armeabi-v7a
    -t arm64-v8a
    -t x86
    -t x86_64
    -o "$output_dir"
    build
    -p silent-disco-ffi
)
if [[ "$profile" == "release" ]]; then
    cargo_args+=(--release)
fi

PATH="$tools_root/bin:$PATH" cargo +"$RUST_TOOLCHAIN" "${cargo_args[@]}"

for abi in armeabi-v7a arm64-v8a x86 x86_64; do
    library="$output_dir/$abi/libsilent_disco_ffi.so"
    if [[ ! -f "$library" ]]; then
        echo "Rust Android build did not produce $library" >&2
        exit 70
    fi
done
