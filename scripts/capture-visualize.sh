#!/usr/bin/env bash
# Isolated native Visualize smoke check. No backend/provider or user's app data.
set -euo pipefail
cd "$(dirname "$0")/.."
output="$(realpath -m "${PI_VISUAL_CAPTURE_DIR:-artifacts/visualize-desktop/native}")"
mkdir -p "$output"
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-visual-capture.XXXXXX)"; app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" XDG_CACHE_HOME="$tmp/cache"
export XDG_RUNTIME_DIR="$tmp/runtime" PI_DESKTOP_CONFIG_DIR="$tmp/desktop" PI_CODING_AGENT_DIR="$tmp/agent"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$PI_DESKTOP_CONFIG_DIR" "$tmp/project"
chmod 700 "$XDG_RUNTIME_DIR"
printf '%s\n' '{"appearance":{"theme":"moonstone"}}' > "$PI_DESKTOP_CONFIG_DIR/settings.json"
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY PI_DESKTOP_DEMO_MEDIA PI_DESKTOP_DEMO_WORKBENCH
export PI_DESKTOP_DEMO_VISUALIZE=1 LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --demo --project "$tmp/project" > "$output/app.log" 2>&1 & app=$!
window=""
for _ in $(seq 1 300); do
    kill -0 "$app" 2>/dev/null || { cat "$output/app.log"; exit 1; }
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    [[ -z "$window" ]] || break
    sleep 0.1
done
[[ -n "$window" ]]
scale="${GPUI_X11_SCALE_FACTOR:-1}"
xdotool windowsize "$window" "$((1344 * scale))" "$((1100 * scale))"
xdotool windowfocus --sync "$window"
xdotool mousemove --window "$window" 100 100
sleep 2
import -window "$window" "$output/before.png"
read -r x y label_x label_y < <(python3 - "$output/before.png" "$scale" <<'PY'
import csv,io,subprocess,sys
result=subprocess.check_output(['tesseract',sys.argv[1],'stdout','--psm','11','tsv'],stderr=subprocess.DEVNULL,text=True)
rows=[r for r in csv.DictReader(io.StringIO(result),delimiter='\t') if r['text']=='Rotate']
assert len(rows)==1,rows
r=rows[0];scale=int(sys.argv[2])
# Slider track is below its label; choose a different, interior angle.
print(int(r['left'])+400*scale, int(r['top'])+int(r['height'])//2+34*scale, int(r['left']), int(r['top']))
PY
)
xdotool mousemove --window "$window" "$x" "$y" click 1
xdotool mousemove --window "$window" 100 100
sleep 1
import -window "$window" "$output/rotated.png"
python3 - "$output" "$label_x" "$label_y" "$scale" <<'PY'
from PIL import Image,ImageChops
from pathlib import Path
import json,sys
p=Path(sys.argv[1]);a=Image.open(p/'before.png').convert('RGB');b=Image.open(p/'rotated.png').convert('RGB')
x,y,scale=map(int,sys.argv[2:])
roi=(x,max(0,y-290*scale),min(a.width,x+700*scale),y-10*scale)
red,green,blue=ImageChops.difference(a.crop(roi),b.crop(roi)).split()
changed=sum(ImageChops.lighter(red,ImageChops.lighter(green,blue)).histogram()[21:])
assert changed>1500, f'Native scene did not visibly change: {changed}'
(p/'result.json').write_text(json.dumps({'native_scene_changed_pixels':changed,'size':a.size,'scene_roi':roi,'note':'Xvfb software-rendered UI/control smoke check; not device FPS measurement.'},indent=2)+'\n')
print(f'Native SVG control changed {changed} pixels.')
PY
