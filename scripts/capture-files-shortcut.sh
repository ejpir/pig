#!/usr/bin/env bash
# Native Files navigation and inspector styling in a disposable git/jj project.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-files-shortcut.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
printf 'KEEP FILE\n' > "$tmp/project/example.txt"
git -C "$tmp/project" init -q
git -C "$tmp/project" add example.txt
git -C "$tmp/project" -c user.name=Probe -c user.email=probe@example.com commit -qm Initial
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log" PI_FILES_COMMAND_CATALOG=1
unset PI_FILES_CRASH
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_DESKTOP_DEMO_WORKSPACE PI_FILES_OTHER_PROJECT
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --project "$tmp/project" --light > artifacts/files-shortcut-app.log 2>&1 & app=$!
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
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.2; xdotool click 1; sleep 0.5; }
shot() { import -window "$window" "artifacts/files-shortcut-$1.png"; }
shot before
tesseract artifacts/files-shortcut-before.png stdout --psm 11 2>/dev/null | grep -q 'inspect-offline'
# Only this disposable repository is opted into jj; no prompts are submitted.
click 940 130
for _ in $(seq 1 50); do [[ ! -d "$tmp/project/.jj" ]] || break; sleep 0.1; done
[[ -d "$tmp/project/.jj" ]]
sleep 1
shot enabled
# Hide the inspector, then reopen Files from the new toolbar icon.
click 1312 25
shot hidden
click 1272 55
sleep 1
shot browser
xdotool windowsize "$window" 1000 680
sleep 0.5
shot narrow
python3 - <<'PY'
from PIL import Image
import subprocess

def text(name, box):
    p=f'artifacts/files-shortcut-{name}-ocr.png'
    im=Image.open(f'artifacts/files-shortcut-{name}.png').crop(box)
    im.resize((im.width*3,im.height*3)).save(p)
    return subprocess.check_output(['tesseract',p,'stdout','--psm','6'],stderr=subprocess.DEVNULL,text=True).lower()
assert 'jj enabled' in text('enabled',(1030,380,1330,560))
for name, box in [('browser',(1030,65,1335,500)),('narrow',(680,65,995,480))]:
    output=text(name,box)
    assert 'files' in output and 'example.txt' in output,output
PY
[[ "$(<"$tmp/project/example.txt")" == 'KEEP FILE' ]]
python3 - "$tmp/commands.log" <<'PY'
import sys
commands=set(open(sys.argv[1]).read().splitlines())
assert commands <= {'get_state','get_active_tools','get_messages','get_session_stats','list_sessions','get_backend_info','get_custom_entries','get_entries','get_settings','get_commands'},commands
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: startup command catalog without slash, jj status, Files toolbar, hidden/narrow inspector navigation, file preserved, no model calls.'; exit 0; fi
  sleep 0.1
done
exit 1
