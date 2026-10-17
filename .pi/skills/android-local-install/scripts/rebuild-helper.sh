#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: rebuild-helper.sh [--keep]

Builds the pinned production Durable runner with the controlled Bun version,
builds a full pi-desktop-remote helper with bundled Durable and stock Pi,
activates it content-addressably, and updates an existing scheduler without
stopping durable session daemons.
EOF
}

KEEP=0
while (($#)); do
  case "$1" in
    --keep)
      KEEP=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
  echo "rebuild-helper.sh requires an x86-64 Linux host" >&2
  exit 1
fi

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
VERSION=$(python - <<'PY'
import tomllib
print(tomllib.load(open('Cargo.toml', 'rb'))['workspace']['package']['version'])
PY
)
BUN_VERSION=$(python - <<'PY'
import json
print(json.load(open('backend/durable/package.json'))['engines']['bun'])
PY
)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/pi-helper-install.XXXXXX")
cleanup() {
  if ((KEEP)); then
    echo "Kept diagnostic helper build directory: $WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

STABLE="$HOME/.pi/desktop/bin/pi-desktop-remote"
WAS_RUNNING=false
HAD_AUTOSTART=false
if [[ -x "$STABLE" ]]; then
  STATUS=$($STABLE scheduler status 2>/dev/null || true)
  if [[ -n "$STATUS" ]]; then
    read -r WAS_RUNNING HAD_AUTOSTART < <(python - "$STATUS" <<'PY'
import json, sys
status = json.loads(sys.argv[1])
print(str(bool(status.get('running'))).lower(), str(bool(status.get('autostart_installed'))).lower())
PY
)
  fi
fi

echo "Installing host Bun $BUN_VERSION for the production Durable build"
npm install --prefix "$WORK/bun-host" --ignore-scripts --no-save "bun@$BUN_VERSION" >/dev/null
(cd "$WORK/bun-host/node_modules/bun" && node install.js)
HOST_BUN="$WORK/bun-host/node_modules/.bin/bun"
[[ $($HOST_BUN --version) == "$BUN_VERSION" ]]

npm ci --ignore-scripts --prefix backend/durable >/dev/null
mkdir "$WORK/durable"
(
  cd backend/durable
  PATH="$(dirname "$HOST_BUN"):$PATH" \
  PI_DESKTOP_DURABLE_OUTPUT_DIR="$WORK/durable" \
    "$HOST_BUN" run build
)
DURABLE="$WORK/durable/pi-desktop-durable"
[[ -x "$DURABLE" && -s "$WORK/durable/NOTICES.txt" ]]
readelf -h "$DURABLE" | grep -Eq 'Machine:[[:space:]]+Advanced Micro Devices X86-64'

echo "Fetching the pinned stock Pi backend"
python scripts/fetch_pi.py --platform linux-x64 --out "$WORK/pi" --notices
BACKEND="$WORK/pi/pi-linux-x64.tar.gz"
[[ -f "$BACKEND" && -s "$WORK/pi/pi-notices.txt" ]]

echo "Building pi-desktop-remote $VERSION with bundled Durable and stock Pi"
RUSTFLAGS="${RUSTFLAGS:--C link-arg=-fuse-ld=lld}" \
CARGO_TARGET_DIR="$WORK/target" \
PI_DESKTOP_DURABLE_BINARY="$DURABLE" \
PI_DESKTOP_BACKEND_ARCHIVE="$BACKEND" \
  cargo build --locked --release -p pi_remote --features bundled-durable,bundled-backend
HELPER="$WORK/target/release/pi-desktop-remote"
[[ -x "$HELPER" ]]
EXPECTED=$($HELPER --version)
[[ "$EXPECTED" =~ ^pi-desktop-remote\ $VERSION\ [0-9]+\ linux-amd64$ ]]
$HELPER --licenses >/dev/null
CAPABILITIES=$($HELPER --capabilities)
python - "$CAPABILITIES" <<'PY'
import json, sys
capabilities = json.loads(sys.argv[1])
if not capabilities.get('durable') or not capabilities.get('pi'):
    raise SystemExit(f'Incomplete helper capabilities: {capabilities}')
PY
HELPER_HASH=$(sha256sum "$HELPER" | awk '{print $1}')

echo "Activating content-addressed helper $HELPER_HASH"
"$HELPER" activate
ACTIVE=$(readlink -f "$STABLE")
EXPECTED_ACTIVE="$HOME/.pi/desktop/bin/$HELPER_HASH/pi-desktop-remote"
[[ "$ACTIVE" == "$EXPECTED_ACTIVE" ]]
[[ $($STABLE --version) == "$EXPECTED" ]]
[[ $(sha256sum "$ACTIVE" | awk '{print $1}') == "$HELPER_HASH" ]]

if [[ "$WAS_RUNNING" == true ]]; then
  "$STABLE" scheduler stop
fi
if [[ "$HAD_AUTOSTART" == true ]]; then
  "$STABLE" scheduler install --confirm
fi
if [[ "$WAS_RUNNING" == true ]]; then
  "$STABLE" scheduler start
fi

FINAL_STATUS=$($STABLE scheduler status)
python - "$FINAL_STATUS" "$WAS_RUNNING" "$HAD_AUTOSTART" <<'PY'
import json, sys
status = json.loads(sys.argv[1])
expected_running = sys.argv[2] == 'true'
expected_autostart = sys.argv[3] == 'true'
if bool(status.get('running')) != expected_running:
    raise SystemExit(f"Scheduler running state changed unexpectedly: {status}")
if bool(status.get('autostart_installed')) != expected_autostart:
    raise SystemExit(f"Scheduler autostart state changed unexpectedly: {status}")
PY
if [[ "$WAS_RUNNING" == true && "$HAD_AUTOSTART" == true ]]; then
  PID=$(systemctl --user show pi-desktop-scheduler.service -p MainPID --value)
  [[ $(readlink -f "/proc/$PID/exe") == "$EXPECTED_ACTIVE" ]]
fi

echo "Activated $EXPECTED at $HELPER_HASH"
echo "Existing durable session daemons were not stopped; fresh sessions use the rebuilt helper"
