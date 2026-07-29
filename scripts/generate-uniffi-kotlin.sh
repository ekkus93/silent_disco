#!/usr/bin/env bash
set -euo pipefail

readonly RUST_TOOLCHAIN="1.97.1"

usage() {
    echo "Usage: $0 <absolute-output-directory>" >&2
}

if [[ $# -ne 1 ]]; then
    usage
    exit 64
fi

output_dir="$1"
if [[ "$output_dir" != /* ]]; then
    echo "UniFFI output directory must be absolute: $output_dir" >&2
    exit 64
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rust_root="$repo_root/rust"
expected_output_root="$repo_root/app/build/generated/uniffiKotlin"

case "$output_dir" in
    "$expected_output_root"|"$expected_output_root"/*) ;;
    *)
        echo "Refusing to write outside $expected_output_root: $output_dir" >&2
        exit 64
        ;;
esac

rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal

cd "$rust_root"
cargo +"$RUST_TOOLCHAIN" build -p silent-disco-ffi

case "$(uname -s)" in
    Linux)
        library="$rust_root/target/debug/libsilent_disco_ffi.so"
        ;;
    Darwin)
        library="$rust_root/target/debug/libsilent_disco_ffi.dylib"
        ;;
    *)
        echo "UniFFI Kotlin generation is supported on Linux and macOS hosts." >&2
        exit 69
        ;;
esac

if [[ ! -f "$library" ]]; then
    echo "Host Rust build did not produce $library" >&2
    exit 70
fi

rm -rf "$output_dir"
mkdir -p "$output_dir"
cargo +"$RUST_TOOLCHAIN" run -p uniffi-bindgen -- \
    generate "$library" \
    --language kotlin \
    --out-dir "$output_dir"

if ! find "$output_dir" -type f -name '*.kt' -print -quit | grep -q .; then
    echo "UniFFI did not generate a Kotlin source file in $output_dir" >&2
    exit 70
fi
