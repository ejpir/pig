#!/usr/bin/env bash
# Actual GPUI + real temporary files. RPC is a metadata-only peer, never a model.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
mkdir -p artifacts
binary="${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target/linux}/debug/pi-desktop}"
binary="$(realpath "$binary")"
tmp="$(mktemp -d /tmp/pi-file-actions.XXXXXX)"
app=""
cleanup() { if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"; }
trap cleanup EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project"
chmod 700 "$XDG_RUNTIME_DIR"
printf 'KEEP EXISTING\n' > "$tmp/project/existing.txt"
# --printenv must work without a display, logger, Pi child, or user environment.
env -i PATH=/usr/bin:/bin PI_PRINTENV_TEST=probe "$binary" --printenv > "$tmp/env.json"
python3 - "$tmp/env.json" <<'PY'
import json,sys
value=json.load(open(sys.argv[1]))
assert value=={"PATH":"/usr/bin:/bin","PI_PRINTENV_TEST":"probe"},value.keys()
PY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log"
"$binary" --project "$tmp/project" --light > artifacts/file-actions-app.log 2>&1 &
app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || { echo 'App exited; see file-actions-app.log' >&2; exit 1; }
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click "${3:-1}"; sleep 0.25; }
shot() { import -window "$window" "artifacts/file-actions-$1.png"; }
wait_file() { for _ in $(seq 1 70); do [[ -e "$1" ]] && return; sleep 0.1; done; shot failure; echo "Missing expected path: $1" >&2; return 1; }
name() { xdotool key ctrl+a; xdotool type --clearmodifiers --delay 30 "$1"; xdotool key Return; sleep 0.6; }
click 944 55
sleep 2
shot browser
click 1258 135
name created.txt
wait_file "$tmp/project/created.txt"
shot created
# Existing files must not be truncated by New File.
click 1258 135
name existing.txt
[[ "$(< "$tmp/project/existing.txt")" == 'KEEP EXISTING' ]]
shot duplicate
xdotool key Escape
sleep 0.2
click 1258 135
name ../escape.txt
[[ ! -e "$tmp/escape.txt" ]]
shot containment
xdotool key Escape
sleep 0.2
# Rename via the context menu (New File, New Folder, Rename).
click 1140 173 3
sleep 0.3
shot menu
xdotool key Home Down Down Return
sleep 0.3
name renamed.txt
wait_file "$tmp/project/renamed.txt"
[[ ! -e "$tmp/project/created.txt" ]]
shot renamed
click 1285 135
name newdir
wait_file "$tmp/project/newdir"
click 1258 135
name nested.txt
wait_file "$tmp/project/newdir/nested.txt"
shot nested
# Unsaved editor content blocks directory deletion.
click 365 137
xdotool type --clearmodifiers --delay 30 UNSAVED_GUARD
sleep 0.4
[[ ! -s "$tmp/project/newdir/nested.txt" ]]
delete_folder() { click 1100 193 3; sleep 0.3; shot delete-menu; click 1140 351; sleep 0.4; }
delete_folder
shot unsaved-guard
tesseract artifacts/file-actions-unsaved-guard.png stdout 2>/dev/null | grep -qi 'Save or close unsaved'
[[ -d "$tmp/project/newdir" ]]
click 365 178
xdotool key ctrl+s
for _ in $(seq 1 50); do grep -q UNSAVED_GUARD "$tmp/project/newdir/nested.txt" && break; sleep 0.1; done
grep -q UNSAVED_GUARD "$tmp/project/newdir/nested.txt"
# Disk write completion precedes the foreground Saved notification/layout.
sleep 0.6
delete_folder
shot delete-confirm
python3 - <<'PY'
from PIL import Image
Image.open('artifacts/file-actions-delete-confirm.png').crop((1030,170,1330,330)).resize((900,480)).save('artifacts/file-actions-confirm-text.png')
PY
tesseract artifacts/file-actions-confirm-text.png stdout --psm 6 2>/dev/null | grep -qi 'Move this item'
xdotool key Escape
sleep 0.3
[[ -d "$tmp/project/newdir" ]]
delete_folder
xdotool key Return
for _ in $(seq 1 70); do [[ ! -e "$tmp/project/newdir" ]] && break; sleep 0.1; done
shot deleted
[[ ! -e "$tmp/project/newdir" ]]
find "$XDG_DATA_HOME/Trash/files" -name nested.txt -exec grep -l UNSAVED_GUARD {} \; | grep -q .
[[ "$(< "$tmp/project/existing.txt")" == 'KEEP EXISTING' ]]
if grep -Eq 'Unknown option: --printenv|Failed to load shell environment|panicked at' artifacts/file-actions-app.log; then echo 'Unexpected app error; see log' >&2; exit 1; fi
python3 - "$PI_FILES_COMMAND_LOG" <<'PY'
import sys
commands=set(open(sys.argv[1]).read().splitlines())
assert commands <= {'get_state','get_active_tools','get_messages','get_session_stats','list_sessions','get_backend_info','get_custom_entries','get_entries','get_settings','get_commands'},commands
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: printenv, create file/folder, no overwrite, path containment, rename, unsaved guard, save, cancel/delete to Trash, no model calls, clean exit.'; exit 0; fi
  sleep 0.1
done
echo 'App did not quit' >&2
exit 1
