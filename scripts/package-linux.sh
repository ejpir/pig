#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Linux ]] || { echo 'Run this script on Linux.' >&2; exit 1; }
target="${PI_DESKTOP_TARGET:-$(rustc -vV | awk '/^host:/ {print $2}')}"
case "$target" in
  x86_64-unknown-linux-gnu) backend=linux-x64 ;;
  aarch64-unknown-linux-gnu) backend=linux-arm64 ;;
  *) echo "Unsupported Linux release target: $target" >&2; exit 1 ;;
esac
# The built-in backend (needs Node.js 22.19+ and Bun): see docs/architecture.md.
backend_out="$PWD/artifacts/backend"
npm ci --prefix packages/pi-desktop-backend
node packages/pi-desktop-backend/scripts/build-binary.mjs --platform "$backend" --out "$backend_out"
PI_DESKTOP_BACKEND_ARCHIVE="$backend_out/pi-desktop-backend-$backend.tar.zst" \
  cargo build --locked --release -p pi-desktop --target "$target" --features bundled-backend
python3 scripts/package_desktop.py --target "$target" \
  --binary "${CARGO_TARGET_DIR:-target}/$target/release/pi-desktop" \
  --notices "$backend_out/pi-desktop-backend-notices.txt"
