#!/usr/bin/env bash
# Real owned RPC children, but metadata-only: no prompts, tools or models.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-close.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" "$tmp/other" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
printf 'KEEP PROJECT\n' > "$tmp/project/sentinel.txt"
printf 'KEEP HISTORY\n' > "$tmp/other/saved.jsonl"
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log" PI_FILES_PID_LOG="$tmp/pids.jsonl" PI_FILES_OTHER_PROJECT="$tmp/other"
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_DESKTOP_DEMO_WORKSPACE
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --project "$tmp/project" --light > artifacts/close-app.log 2>&1 & app=$!
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
shot() { import -window "$window" "artifacts/close-$1.png"; }
last_pid() { python3 - "$tmp/pids.jsonl" <<'PY'
import json,sys
print(json.loads(open(sys.argv[1]).readlines()[-1])['pid'])
PY
}
wait_dead() { for _ in $(seq 1 50); do kill -0 "$1" 2>/dev/null || return 0; sleep 0.1; done; echo 'RPC child survived close' >&2; exit 1; }
wait_cwd() {
  local want="$1" n="$2"
  for _ in $(seq 1 50); do
    if python3 - "$tmp/pids.jsonl" "$want" "$n" <<'PY'
import json,sys
rows=[json.loads(l) for l in open(sys.argv[1])]
assert len(rows)==int(sys.argv[3]) and rows[-1]['cwd']==sys.argv[2]
PY
    then return; fi
    sleep 0.1
  done
  echo 'New session used the wrong project' >&2; exit 1
}
first="$(last_pid)"
xdotool type --clearmodifiers 'KEEP_DRAFT'
click 184 110
shot draft-confirm
xdotool key Escape
sleep 0.4
kill -0 "$first"
# Cancellation restores the composer; clear only the test's draft, not a file.
click 400 627
xdotool key ctrl+a BackSpace
click 184 110
wait_dead "$first"
shot empty-selected-project
# Last session is gone, but Ctrl+N must still use the selected project.
xdotool key ctrl+n
sleep 0.8
wait_cwd "$tmp/project" 2
second="$(last_pid)"
click 184 110
wait_dead "$second"
# Select the other project heading (no process starts on selection).
click 76 174
shot selected-other
xdotool key ctrl+n
sleep 0.8
wait_cwd "$tmp/other" 3
third="$(last_pid)"
shot other-session
click 184 110
wait_dead "$third"
# Remove the first project, cancel once, then confirm with the native prompt.
click 184 146
shot project-confirm
xdotool key Escape
sleep 0.3
click 184 146
shot project-confirm
# Cancel is the safe default. Tab selects the explicit close action.
xdotool key Tab Return
sleep 0.5
shot project-removed
python3 - <<'PY'
from PIL import Image
import subprocess,re
before=Image.open('artifacts/close-selected-other.png')
after=Image.open('artifacts/close-project-removed.png')
assert before.getpixel((300,300))==after.getpixel((300,300)), 'Confirmation was not dismissed'
def sidebar(image,name):
    path=f'artifacts/close-{name}-text.png'
    image.crop((8,130,200,242)).resize((576,336)).save(path)
    return subprocess.check_output(['tesseract',path,'stdout','--psm','6'],stderr=subprocess.DEVNULL,text=True).lower()
a,b=sidebar(before,'before'),sidebar(after,'after')
assert re.search(r'\bproject\b',a),a
assert not re.search(r'\bproject\b',b) and 'other' in b,b
PY
[[ "$(< "$tmp/project/sentinel.txt")" == 'KEEP PROJECT' ]]
[[ "$(< "$tmp/other/saved.jsonl")" == 'KEEP HISTORY' ]]
python3 - "$tmp/commands.log" <<'PY'
import sys
allowed={'get_state','get_messages','get_session_stats','list_sessions','get_commands','get_entries','get_settings'}
assert set(open(sys.argv[1]).read().splitlines())<=allowed
PY
! grep -q 'panicked at' artifacts/close-app.log
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: draft cancel, process cleanup, last-session close, selected-project New Session, project removal, files/history preserved, no model calls.'; exit 0; fi
  sleep 0.1
done
exit 1
