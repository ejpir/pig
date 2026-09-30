#!/usr/bin/env bash
# Synthetic historical tool result, real native layout/clipboard; no prompts or writes.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-wrap.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log" PI_FILES_LONG_CHANGE=1
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_FILES_OTHER_PROJECT PI_FILES_CRASH
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --project "$tmp/project" --light > artifacts/wrapping-app.log 2>&1 & app=$!
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
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click 1; sleep 0.3; }
click 330 72
import -window "$window" artifacts/wrapping-on.png
# Locate the controls from actual rendered text, avoiding font-specific baselines.
python3 - <<'PY' > "$tmp/controls"
from PIL import Image
import subprocess,csv,io
im=Image.open('artifacts/wrapping-on.png')
for name,area in [('Wrap',(448,92,1016,716)),('Copy',(1016,52,1344,716))]:
    path=f'artifacts/wrapping-{name.lower()}-ocr.png'
    crop=im.crop(area); crop.resize((crop.width*2,crop.height*2)).save(path)
    tsv=subprocess.check_output(['tesseract',path,'stdout','--psm','11','tsv'],stderr=subprocess.DEVNULL,text=True)
    r=next(r for r in csv.DictReader(io.StringIO(tsv),delimiter='\t') if r['text'].lower().split(':')[0]==name.lower())
    print(area[0]+(int(r['left'])+int(r['width'])//2)//2,area[1]+(int(r['top'])+int(r['height'])//2)//2)
PY
mapfile -t controls < "$tmp/controls"
read -r x y <<< "${controls[0]}"
click "$x" "$y"
xdotool mousemove --window "$window" 410 600
sleep 0.2
import -window "$window" artifacts/wrapping-off.png
read -r x y <<< "${controls[1]}"
click "$x" "$y"
sleep 0.2
timeout 5 /usr/bin/xclip -selection clipboard -o > artifacts/wrapping-copy.txt
python3 - "$tmp/commands.log" <<'PY'
from PIL import Image
from pathlib import Path
import subprocess,sys
for mode in ['on','off']:
    p=f'artifacts/wrapping-{mode}-body-ocr.png'
    # Remove the pale syntax-highlight background for OCR only; retain the
    # untouched native captures above as visual evidence.
    im=Image.open(f'artifacts/wrapping-{mode}.png').crop((448,194,1016,716)).convert('L')
    im=im.point(lambda value: 0 if value < 180 else 255)
    im.resize((2272,2088)).save(p)
    text=subprocess.check_output(['tesseract',p,'stdout','--psm','6'],stderr=subprocess.DEVNULL,text=True).upper()
    assert ('WRAP_END' in text)==(mode=='on'),(mode,text)
raw=Path('artifacts/wrapping-copy.txt').read_text()
assert 'call_'+'very-long-id-'*24+'終点' in raw
assert 'WRAP_START '+'A deliberately long fixture line with preserved spaces. '*8+'WRAP_END' in raw
assert set(Path(sys.argv[1]).read_text().splitlines()) <= {'get_state','get_messages','get_session_stats','list_sessions','get_commands','get_entries','get_settings'}
PY
[[ ! -e "$tmp/project/long.txt" ]]
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: default wrapping, wrap toggle, full raw line and Unicode call ID copied, no model calls or file writes.'; exit 0; fi
  sleep 0.1
done
exit 1
