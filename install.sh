#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.local/share/pi-desktop-build/node_modules/.bin:$PATH"

sha256_file() {
  local file=$1
  local output hash

  if command -v sha256sum >/dev/null 2>&1; then
    output=$(sha256sum "$file") || {
      printf 'error: sha256sum failed for %s\n' "$file" >&2
      return 1
    }
  elif command -v shasum >/dev/null 2>&1; then
    output=$(shasum -a 256 "$file") || {
      printf 'error: shasum failed for %s\n' "$file" >&2
      return 1
    }
  else
    printf 'error: no SHA-256 utility found (tried sha256sum and shasum)\n' >&2
    return 1
  fi

  read -r hash _ <<< "$output"
  if [[ ! $hash =~ ^[0-9A-Fa-f]{64}$ ]]; then
    printf 'error: invalid SHA-256 output for %s\n' "$file" >&2
    return 1
  fi

  printf '%s\n' "$hash"
}

(cd backend/durable && bun run build)

PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
  cargo build --locked --release -p pi_remote --features bundled-durable

hash=$(sha256_file target/release/pi-desktop-remote)
dir="$HOME/.pi/desktop/bin/$hash"
(umask 077; mkdir -p "$dir")
test -f "$dir/pi-desktop-remote" ||
  install -m 700 target/release/pi-desktop-remote "$dir/pi-desktop-remote"

"$dir/pi-desktop-remote" --capabilities
