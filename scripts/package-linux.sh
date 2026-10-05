#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Linux ]] || { echo 'Run this script on Linux.' >&2; exit 1; }
target="${PI_DESKTOP_TARGET:-$(rustc -vV | awk '/^host:/ {print $2}')}"
case "$target" in
  x86_64-unknown-linux-gnu) pi=linux-x64 ;;
  aarch64-unknown-linux-gnu) pi=linux-arm64 ;;
  *) echo "Unsupported Linux release target: $target" >&2; exit 1 ;;
esac
# The built-in pi, pi's own release binary (notices need npm): see docs/architecture.md.
pi_out="$PWD/artifacts/pi"
python3 scripts/fetch_pi.py --platform "$pi" --out "$pi_out" --notices
PI_DESKTOP_BACKEND_ARCHIVE="$pi_out/pi-$pi.tar.gz" \
  cargo build --locked --release -p pi-desktop -p pi_remote --target "$target" --features pi-desktop/bundled-backend,pi_remote/bundled-backend
python3 scripts/package_desktop.py --target "$target" \
  --binary "${CARGO_TARGET_DIR:-target}/$target/release/pi-desktop" \
  --notices "$pi_out/pi-notices.txt"
python3 scripts/package_remote.py --target "$target" \
  --binary "${CARGO_TARGET_DIR:-target}/$target/release/pi-desktop-remote"
