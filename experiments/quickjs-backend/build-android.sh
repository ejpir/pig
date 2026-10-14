#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to an installed NDK}"
target="${1:-aarch64-linux-android}"
case "$target" in
    aarch64-linux-android|x86_64-linux-android) ;;
    *) echo 'Supported targets: aarch64-linux-android, x86_64-linux-android' >&2; exit 2 ;;
esac
case "$(uname -s)" in
    Linux) host=linux-x86_64 ;;
    Darwin) host=darwin-x86_64 ;;
    *) echo 'Build host must be Linux or macOS' >&2; exit 2 ;;
esac
toolchain="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host"
cc="$toolchain/bin/${target}30-clang"
[[ -x "$cc" ]] || { echo "Missing NDK compiler: $cc" >&2; exit 1; }
key="${target//-/_}"
export "CC_${key}=$cc" "AR_${key}=$toolchain/bin/llvm-ar"
upper_key="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CARGO_TARGET_${upper_key}_LINKER=$cc"
export "BINDGEN_EXTRA_CLANG_ARGS_${key}=--sysroot=$toolchain/sysroot --target=${target}30"
export LIBCLANG_PATH="${LIBCLANG_PATH:-$toolchain/lib}"
cargo build --locked --release --target "$target"
