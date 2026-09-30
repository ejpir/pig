#!/usr/bin/env bash
# Actual GPUI rendering of settled success/failure histories; no model/tool calls.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-activity-status.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export PI_DESKTOP_PI="$repo/fixtures/activity-rpc.py"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
mkdir -p artifacts
for mode in passed failed; do
  export HOME="$tmp/$mode/home" XDG_DATA_HOME="$tmp/$mode/data" XDG_CONFIG_HOME="$tmp/$mode/config" XDG_CACHE_HOME="$tmp/$mode/cache" XDG_RUNTIME_DIR="$tmp/$mode/runtime"
  mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/$mode/project"
  chmod 700 "$XDG_RUNTIME_DIR"
  export PI_ACTIVITY_FAILURE=0 PI_ACTIVITY_LOG="$tmp/$mode/commands.jsonl"
  [[ "$mode" != failed ]] || export PI_ACTIVITY_FAILURE=1
  "$binary" --project "$tmp/$mode/project" > "artifacts/activity-$mode-app.log" 2>&1 & app=$!
  window=""
  for _ in $(seq 1 100); do
    kill -0 "$app" 2>/dev/null || exit 1
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    [[ -n "$window" ]] && break
    sleep .1
  done
  [[ -n "$window" ]]
  xdotool windowfocus --sync "$window"
  sleep 3
  import -window "$window" "artifacts/activity-$mode-collapsed.png"
  python3 - "$mode" <<'PY' > "$tmp/point"
from PIL import Image
import csv, io, re, subprocess, sys
mode = sys.argv[1]
im = Image.open(f'artifacts/activity-{mode}-collapsed.png').crop((208, 92, 1016, 600))
im.resize((1616, 1016)).save('artifacts/activity-status-ocr.png')
args = ['tesseract', 'artifacts/activity-status-ocr.png', 'stdout', '--psm', '11']
text = subprocess.check_output(args, text=True, stderr=subprocess.DEVNULL).lower()
assert re.search(r'4\s*tool calls', text), text
assert ('1 failed' if mode == 'failed' else 'complete') in text, text
assert 'npm run' not in text, 'Settled tool rows must be hidden'
rows = list(csv.DictReader(io.StringIO(subprocess.check_output(args + ['tsv'], text=True, stderr=subprocess.DEVNULL)), delimiter='\t'))
row = next(row for row in rows if 'tool' in row['text'].lower())
print(400, 92 + (int(row['top']) + int(row['height']) // 2) // 2)
PY
  read -r x y < "$tmp/point"
  xdotool mousemove --window "$window" "$x" "$y"
  xdotool click 1
  xdotool mousemove --window "$window" 900 580
  sleep .5
  import -window "$window" "artifacts/activity-$mode-expanded.png"
  tesseract "artifacts/activity-$mode-expanded.png" stdout --psm 11 2>/dev/null | grep -q 'npm run'
  python3 - "$PI_ACTIVITY_LOG" <<'PY'
import json, sys
from pathlib import Path
allowed = {'get_state', 'get_messages', 'get_entries', 'get_session_stats', 'get_available_models', 'get_available_thinking_levels', 'get_settings', 'get_commands', 'list_sessions'}
commands = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert {command['type'] for command in commands} <= allowed, commands
PY
  xdotool key ctrl+q
  for _ in $(seq 1 50); do
    if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; break; fi
    sleep .1
  done
  [[ -z "$app" ]]
done
echo 'PASS: native settled success/failure groups collapse, retain outcome labels, and reopen original calls; metadata-only RPC.'
