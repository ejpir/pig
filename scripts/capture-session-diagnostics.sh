#!/usr/bin/env bash
# Exercise a real failing subprocess without running Pi or making model calls.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-diagnostics.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log" PI_FILES_CRASH=1
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_FILES_OTHER_PROJECT
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --project "$tmp/project" --light > artifacts/session-diagnostics-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || exit 1
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
import -window "$window" artifacts/session-diagnostics-banner.png
xdotool key ctrl+shift+d
sleep 0.5
import -window "$window" artifacts/session-diagnostics-console.png
python3 - <<'PY' > "$tmp/copy"
from PIL import Image
import csv,io,subprocess
im=Image.open('artifacts/session-diagnostics-console.png').crop((1020,52,1344,715))
im.resize((648,1326)).save('artifacts/session-diagnostics-ocr.png')
text=subprocess.check_output(['tesseract','artifacts/session-diagnostics-ocr.png','stdout','--psm','6','tsv'],stderr=subprocess.DEVNULL,text=True)
r=next(r for r in csv.DictReader(io.StringIO(text),delimiter='\t') if r['text'].lower()=='copy')
print(1020+(int(r['left'])+int(r['width'])//2)//2,52+(int(r['top'])+int(r['height'])//2)//2)
PY
read -r x y < "$tmp/copy"
xdotool mousemove --window "$window" "$x" "$y"
sleep 0.2
xdotool click 1
sleep 0.3
timeout 5 /usr/bin/xclip -selection clipboard -o > artifacts/session-diagnostics-copy.txt
grep -q 'EACCES opening saved session' artifacts/session-diagnostics-copy.txt
grep -q 'exit status: 1' artifacts/session-diagnostics-copy.txt
grep -q 'get_commands' artifacts/session-diagnostics-copy.txt
python3 - "$tmp/commands.log" <<'PY'
import sys
assert set(open(sys.argv[1]).read().splitlines()) <= {'get_state','get_messages','get_session_stats','list_sessions','get_commands'}
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: process failure, diagnostics shortcut, full captured stderr tail copied, no model calls.'; exit 0; fi
  sleep 0.1
done
exit 1
