#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Darwin ]] || { echo 'Run on macOS with Xcode Command Line Tools installed.' >&2; exit 1; }
target="${PI_DESKTOP_TARGET:-aarch64-apple-darwin}"
[[ "$target" == aarch64-apple-darwin ]] || { echo "Unsupported macOS release target: $target" >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=12.0
# The built-in pi, pi's own release binary (notices need npm): see docs/architecture.md.
# PI_DESKTOP_CODESIGN_IDENTITY re-signs it for the hardened runtime before it is embedded.
pi_out="$PWD/artifacts/pi"
python3 scripts/fetch_pi.py --platform darwin-arm64 --out "$pi_out" --notices
PI_DESKTOP_BACKEND_ARCHIVE="$pi_out/pi-darwin-arm64.tar.gz" \
  cargo build --locked --release -p pi-desktop --target "$target" --features bundled-backend
python3 scripts/package_desktop.py --target "$target" \
  --binary "${CARGO_TARGET_DIR:-target}/$target/release/pi-desktop" \
  --notices "$pi_out/pi-notices.txt"
printf 'Packaged macOS arm64 (ad-hoc signed; not Developer ID signed or notarized).\n'
