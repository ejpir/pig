#!/usr/bin/env bash
# Opt-in, real RPC bootstrap/resume validation. Never sends a prompt.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
scratch=$(mktemp -d /tmp/pi-desktop-rpc-XXXXXX)
app=""
cleanup() {
    if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi
    rm -rf "$scratch"
}
trap cleanup EXIT
export PI_CODING_AGENT_DIR="$scratch/config"
export PI_DESKTOP_LIVE_TEST_ENTRY="${PI_DESKTOP_RPC_ENTRY:-$PWD/scripts/pi-rpc.mjs}"
export PI_DESKTOP_RPC_ENTRY="$scratch/entry.mjs"
export LIVE_TEST_PROJECT="$scratch/project"
python3 - <<'PY'
import json, os
from pathlib import Path
project = Path(os.environ['LIVE_TEST_PROJECT'])
project.mkdir()
sessions = Path(os.environ['PI_CODING_AGENT_DIR']) / 'sessions' / 'test-project'
sessions.mkdir(parents=True)
records = [
    {'type':'session','version':3,'id':'11111111-1111-4111-8111-111111111111','cwd':str(project),'timestamp':'2026-09-28T09:41:00Z'},
    {'type':'message','id':'a1b2c3d4','parentId':None,'timestamp':'2026-09-28T09:41:00Z','message':{'role':'user','content':'hello','timestamp':1790588460000}},
]
(sessions / 'hello.jsonl').write_text(''.join(json.dumps(record) + '\n' for record in records))
Path(os.environ['PI_DESKTOP_RPC_ENTRY']).write_text('''import {spawn} from 'node:child_process';
const child = spawn(process.execPath, [process.env.PI_DESKTOP_LIVE_TEST_ENTRY, '--offline', '--no-extensions', '--no-skills', '--no-prompt-templates', ...process.argv.slice(2)], {stdio:'inherit'});
child.on('error', e => { console.error(e.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
''')
PY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/pi-desktop-runtime-$UID}"
mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY
binary="${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}"
"$binary" --light --project "$LIVE_TEST_PROJECT" > artifacts/refactor-live-app.log 2>&1 &
app=$!
window=""
for _ in $(seq 1 100); do
    kill -0 "$app" 2>/dev/null || { printf 'App exited; see artifacts/refactor-live-app.log\n' >&2; exit 1; }
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    [[ -n "$window" ]] && break
    sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
# Poll the real window for the saved session supplied by the isolated fixture.
position=""
for _ in $(seq 1 30); do
    import -window "$window" artifacts/thread-live.png
    tesseract artifacts/thread-live.png stdout --psm 11 tsv 2>/dev/null > "$scratch/ocr.tsv"
    position=$(awk -F '\t' 'tolower($12)=="hello" { print $7+$9/2, $8+$10/2; exit }' "$scratch/ocr.tsv")
    [[ -n "$position" ]] && break
    sleep 0.5
done
[[ -n "$position" ]] || { printf 'Saved session did not load\n' >&2; exit 1; }
read -r x y <<< "$position"
xdotool mousemove --window "$window" "${x%.*}" "${y%.*}" click 1
for _ in $(seq 1 30); do
    import -window "$window" artifacts/thread-live-resumed.png
    tesseract artifacts/thread-live-resumed.png artifacts/thread-live-resumed --psm 11 2>/dev/null
    # Upscale the small status line; full-frame OCR can read the numeral 1 as L.
    python3 - <<'PY'
from PIL import Image
image = Image.open('artifacts/thread-live-resumed.png')
image.crop((1030, 590, 1320, 613)).resize((870, 69)).save('artifacts/live-stats-crop.png')
PY
    tesseract artifacts/live-stats-crop.png artifacts/live-stats-crop --psm 7 2>/dev/null
    if grep -qi '2 processes' artifacts/thread-live-resumed.txt && grep -Eqi '1[^[:alnum:]]*messages' artifacts/live-stats-crop.txt; then break; fi
    sleep 0.5
done
grep -qi '2 processes' artifacts/thread-live-resumed.txt
grep -Eqi '1[^[:alnum:]]*messages' artifacts/live-stats-crop.txt
# The selected thread/inspector title and loaded message all retain the fallback title.
[[ $(grep -io 'hello' artifacts/thread-live-resumed.txt | wc -l) -ge 3 ]]
! grep -qi 'setup failed\|did not resume' artifacts/thread-live-resumed.txt
xdotool key ctrl+q
for _ in $(seq 1 100); do
    if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; printf 'PASS: real RPC bootstrap, saved-session resume, retained title, two owned processes, clean exit; no prompts sent.\n'; exit 0; fi
    sleep 0.1
done
printf 'Application did not quit\n' >&2
exit 1
