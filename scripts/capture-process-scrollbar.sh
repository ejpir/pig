#!/usr/bin/env bash
# Native overflow/drag/track geometry only: explicitly labelled demo, no Pi subprocesses.
set -euo pipefail
cd "$(dirname "$0")/.."
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-process-scroll.XXXXXX)"; app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
export PI_DESKTOP_CONFIG_DIR="$tmp/desktop" PI_CODING_AGENT_DIR="$tmp/agent"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts; chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --demo --light --project "$tmp/project" > artifacts/process-scrollbar-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break; sleep .1
done
[[ -n "$window" ]]; xdotool windowfocus --sync "$window"
# Wait for a painted chooser and its completion, not a cold runner's frame time.
find_create() {
  import -window "$window" artifacts/process-scrollbar-create.png
  python3 - <<'PY' > "$tmp/create" 2>/dev/null
from PIL import Image
im=Image.open('artifacts/process-scrollbar-create.png').convert('RGB')
rows=[]
for y in range(500,625):
 xs=[x for x in range(700,920) if (lambda c: c[2]-c[0]>20 and c[2]-c[1]>15)(im.getpixel((x,y)))]
 rows.append((len(xs),xs,y))
count,xs,y=max(rows,key=lambda row:row[0])
assert count>70, 'No accent-colored Create button'
print((min(xs)+max(xs))//2,y)
PY
}
create_session() {
  local ready=false closed=false x y
  for _ in $(seq 1 50); do
    # A key can arrive before the first frame's action context is mounted.
    # An already-open chooser ignores repeated New Session requests.
    xdotool key ctrl+n
    sleep .1
    if find_create; then ready=true; break; fi
  done
  "$ready" || { echo 'New-session chooser did not become ready' >&2; return 1; }
  read -r x y < "$tmp/create"
  xdotool mousemove --window "$window" "$x" "$y" click 1
  for _ in $(seq 1 50); do
    sleep .1
    if ! find_create; then closed=true; break; fi
  done
  "$closed" || { echo 'New-session chooser did not close after Create' >&2; return 1; }
}
for _ in $(seq 1 20); do create_session; done
xdotool mousemove --window "$window" 145 727 click 1; sleep .5
xdotool mousemove --window "$window" 10 52; sleep .6
import -window "$window" artifacts/process-scrollbar-top.png
# The reserved 14px gutter is separate from rows. Track click, then drag to the top.
xdotool mousemove --window "$window" 513 645 click 1; sleep .3
xdotool mousemove --window "$window" 10 52; sleep .6
import -window "$window" artifacts/process-scrollbar-bottom.png
python3 - <<'PY' > "$tmp/thumb"
from PIL import Image
im=Image.open('artifacts/process-scrollbar-bottom.png').convert('RGB')
ys=[y for y in range(115,670) if any(min(im.getpixel((x,y)))<225 for x in range(506,519))]
assert ys, 'No painted thumb in the reserved gutter'
print(513,(min(ys)+max(ys))//2)
PY
read -r sx sy < "$tmp/thumb"
xdotool mousemove --window "$window" "$sx" "$sy" mousedown 1 mousemove --window "$window" 513 117 mouseup 1; sleep .3
xdotool mousemove --window "$window" 10 52; sleep .6
import -window "$window" artifacts/process-scrollbar-drag-top.png
python3 - <<'PY'
from PIL import Image,ImageChops
import json
images={name:Image.open(f'artifacts/process-scrollbar-{name}.png').convert('RGB') for name in ['top','bottom','drag-top']}
# Popup header/footer do not move while its rows scroll.
for name in ['bottom','drag-top']:
 for bounds in [(29,82,500,108),(29,678,510,695)]:
  assert ImageChops.difference(images['top'].crop(bounds),images[name].crop(bounds)).getbbox() is None, (name,bounds)
assert ImageChops.difference(images['top'].crop((29,117,504,667)),images['bottom'].crop((29,117,504,667))).getbbox(), 'Track click did not scroll rows'
assert ImageChops.difference(images['bottom'].crop((29,117,504,667)),images['drag-top'].crop((29,117,504,667))).getbbox(), 'Thumb drag did not scroll rows'
json.dump({'native':True,'demo':True,'piSubprocesses':False,'trackClick':True,'thumbDrag':True,'fixedHeaderFooter':True},open('artifacts/process-scrollbar-validation.json','w'),indent=2)
PY
xdotool key Escape ctrl+q
for _ in $(seq 1 75); do
 if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: native demo overflow thumb, track click/drag, fixed popup header/footer; no Pi subprocess.'; exit 0; fi
 sleep .1
done
exit 1
