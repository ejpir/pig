#!/usr/bin/env bash
# Native grouping and Context-card regression; offline data, no model calls.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="$(mktemp -d /tmp/pi-readability.XXXXXX)"
export PI_DESKTOP_DEMO_READABILITY=1
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE
"${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}" --demo --light > artifacts/readability-app.log 2>&1 & app=$!
trap 'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; rm -rf "$XDG_RUNTIME_DIR"' EXIT
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
shot() { import -window "$window" "artifacts/readability-$1.png"; }
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click 1; sleep 0.5; }
# Window mapping can precede the first rendered frame under CPU/GPU contention.
for _ in $(seq 1 25); do
  shot collapsed
  if python3 - <<'PY'
from PIL import Image
im=Image.open('artifacts/readability-collapsed.png').convert('RGB')
raise SystemExit(0 if im.getcolors(40) is None else 1)
PY
  then break; fi
  sleep 0.2
done
sleep 0.2
shot status-frame
python3 - <<'PY'
from PIL import Image, ImageChops
region=(1035,110,1051,126)
a=Image.open('artifacts/readability-collapsed.png').crop(region)
b=Image.open('artifacts/readability-status-frame.png').crop(region)
assert ImageChops.difference(a.convert('RGB'),b.convert('RGB')).getbbox(), 'Inspector running mark must animate'
PY
python3 - <<'PY' > "$XDG_RUNTIME_DIR/group"
from PIL import Image
import subprocess,io,csv,re
im=Image.open('artifacts/readability-collapsed.png').crop((208,92,1016,580))
im.resize((1616,976)).save('artifacts/readability-collapsed-ocr.png')
p='artifacts/readability-collapsed-ocr.png'
text=subprocess.check_output(['tesseract',p,'stdout','--psm','6'],stderr=subprocess.DEVNULL,text=True).lower()
assert re.search(r'4\s*tool calls',text) and 'complete' in text,text
assert 'gib' in text and 'snapshot tests pass' in text,text
assert text.count('thinking')==1,text
assert 'npm run' not in text,text
rows=csv.DictReader(io.StringIO(subprocess.check_output(['tesseract',p,'stdout','--psm','6','tsv'],stderr=subprocess.DEVNULL,text=True)),delimiter='\t')
r=next(r for r in rows if r['text'].strip().startswith('4'))
print(400,92+(int(r['top'])+int(r['height'])//2)//2)
PY
read -r x y < "$XDG_RUNTIME_DIR/group"
click "$x" "$y"
xdotool mousemove --window "$window" 900 580
sleep 0.2
shot expanded
tesseract artifacts/readability-expanded.png stdout --psm 11 2>/dev/null | grep -q 'npm run'
click "$x" "$y"
shot recollapsed
xdotool windowmove "$window" 0 0
xdotool windowsize "$window" 1600 900
sleep 0.5
shot wide
click 493 72
shot context
xdotool windowsize "$window" 1000 680
sleep 0.5
shot context-narrow
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; echo 'PASS: completed activity folded, original calls expandable, answer preserved, one live status, animated inspector; captured wide transcript and Context tiles.'; exit 0; fi
  sleep 0.1
done
exit 1
