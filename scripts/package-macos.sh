#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Darwin ]] || { echo 'Run on macOS with Xcode Command Line Tools installed.' >&2; exit 1; }
target="${PI_DESKTOP_TARGET:-aarch64-apple-darwin}"
[[ "$target" == aarch64-apple-darwin ]] || { echo "Unsupported macOS release target: $target" >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=12.0
# The built-in backend (needs Node.js 22.19+ and Bun): see docs/architecture.md.
# PI_DESKTOP_CODESIGN_IDENTITY signs it for the hardened runtime before it is embedded.
backend_out="$PWD/artifacts/backend"
npm ci --prefix packages/pi-desktop-backend
node packages/pi-desktop-backend/scripts/build-binary.mjs --platform darwin-arm64 --out "$backend_out"
PI_DESKTOP_BACKEND_ARCHIVE="$backend_out/pi-desktop-backend-darwin-arm64.tar.zst" \
  cargo build --locked --release -p pi-desktop --target "$target" --features bundled-backend
python3 scripts/package_desktop.py --target "$target" \
  --binary "${CARGO_TARGET_DIR:-target}/$target/release/pi-desktop" \
  --notices "$backend_out/pi-desktop-backend-notices.txt"
printf 'Packaged macOS arm64 (ad-hoc signed; not Developer ID signed or notarized).\n'
