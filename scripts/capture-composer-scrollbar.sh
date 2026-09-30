#!/usr/bin/env bash
# Native capped/expanded composer scrolling; draft only, never submits a prompt.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-input-scrollbar.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
export PI_DESKTOP_PI="$repo/fixtures/activity-rpc.py" PI_ACTIVITY_LOG="$tmp/commands.jsonl" PI_ACTIVITY_FAILURE=0
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --project "$tmp/project" --light > artifacts/input-scrollbar-app.log 2>&1 & app=$!
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
python3 - <<'PY' > "$tmp/draft"
print('\n'.join(f'DRAFT LINE {i:02} — multiline editor regression' for i in range(1, 41)), end='')
PY
/usr/bin/xclip -selection clipboard < "$tmp/draft"
xdotool key ctrl+v
sleep .7
shot() { xdotool mousemove --window "$window" 10 52; sleep .2; import -window "$window" "artifacts/input-scrollbar-$1.png"; }
assert_line() {
  python3 - "$1" "$2" <<'PY'
from PIL import Image
import re, subprocess, sys
im = Image.open(f'artifacts/input-scrollbar-{sys.argv[1]}.png').crop((228, 92, 990, 664))
im.resize((im.width * 2, im.height * 2)).save('artifacts/input-scrollbar-ocr.png')
text = subprocess.check_output(['tesseract', 'artifacts/input-scrollbar-ocr.png', 'stdout', '--psm', '6'], text=True, stderr=subprocess.DEVNULL)
assert re.search(r'DRAFT\s+LINE\s+' + sys.argv[2], text, re.I), text
PY
}
shot bottom
assert_line bottom 40
xdotool mousemove --window "$window" 948 638
xdotool mousedown 1
xdotool mousemove --window "$window" 948 500
xdotool mouseup 1
sleep .3
shot drag-top
assert_line drag-top 01
xdotool mousemove --window "$window" 948 650
xdotool click 1
sleep .3
shot track-bottom
assert_line track-bottom 40
xdotool mousemove --window "$window" 500 540
xdotool click --repeat 20 --delay 30 4
sleep .5
shot top
assert_line top 01
# A redraw/hover must not pull the viewport back to the insertion point.
sleep .5
shot top-still
assert_line top-still 01
xdotool key shift+alt+Escape
sleep .5
shot expanded
# Expanding also preserves the manually chosen top-of-draft position.
assert_line expanded 01
# The idle thumb must actually paint, not merely have a working invisible hitbox.
python3 - <<'PY'
from PIL import Image
for name, top in [('bottom', 496), ('top', 496), ('expanded', 344)]:
    im = Image.open(f'artifacts/input-scrollbar-{name}.png').convert('RGB')
    pixels = im.crop((940, top, 957, 657)).getdata()
    assert sum(min(pixel) < 245 for pixel in pixels) > 40, f'{name}: missing visible thumb'
PY
# Real clipboard equality, not a stale copy of the original paste payload.
printf 'clipboard sentinel' | /usr/bin/xclip -selection clipboard
xdotool key ctrl+a ctrl+c
sleep .3
/usr/bin/xclip -selection clipboard -o > artifacts/input-scrollbar-copy.txt
cmp "$tmp/draft" artifacts/input-scrollbar-copy.txt
xdotool key BackSpace
sleep .3
shot empty
python3 - "$PI_ACTIVITY_LOG" <<'PY'
import json, sys
from pathlib import Path
allowed = {'get_state', 'get_messages', 'get_entries', 'get_session_stats', 'get_available_models', 'get_available_thinking_levels', 'get_settings', 'get_commands', 'list_sessions'}
commands = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert {command['type'] for command in commands} <= allowed, commands
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then
    wait "$app"; app=""
    echo 'PASS: native capped/expanded composer has a visible thumb, drag/track/wheel scrolling without caret snap-back, and exact draft copy; no prompt or tool calls.'
    exit 0
  fi
  sleep .1
done
exit 1
