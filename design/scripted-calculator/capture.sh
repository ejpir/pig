#!/usr/bin/env bash
# Native GPUI screenshots and a mouse/keyboard smoke check. Run on Linux/X11.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
binary="${PI_CALCULATOR_BINARY:-${CARGO_TARGET_DIR:-$here/native/target}/debug/pi-scripted-calculator-mock}"
binary="$(realpath "$binary")"
if [[ "${PI_CALCULATOR_CAPTURE_IN_XVFB:-}" != 1 ]]; then
    exec xvfb-run -a -s '-screen 0 1280x1050x24' env PI_CALCULATOR_CAPTURE_IN_XVFB=1 PI_CALCULATOR_BINARY="$binary" bash "$0"
fi
mkdir -p "$here/screens"
tmp="$(mktemp -d /tmp/pi-calculator-capture.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export XDG_RUNTIME_DIR="$tmp/runtime" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
mkdir -m 700 "$XDG_RUNTIME_DIR"
mkdir "$tmp/app"
cp "$here/app/ui.json" "$here/app/behavior.rhai" "$tmp/app/"
unset WAYLAND_DISPLAY
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --app "$tmp/app" > "$tmp/app.log" 2>&1 &
app=$!
window=""
for _ in $(seq 1 300); do
    if ! kill -0 "$app" 2>/dev/null; then cat "$tmp/app.log"; exit 1; fi
    window="$(xdotool search --onlyvisible --name 'Scripted calculator' 2>/dev/null | head -1 || true)"
    if [[ -n "$window" ]]; then break; fi
    sleep 0.1
done
[[ -n "$window" ]] || { cat "$tmp/app.log"; echo 'No native window appeared' >&2; exit 1; }
xdotool windowfocus --sync "$window"
sleep 2
# Real local calculations populate the script's history; these are not image labels.
xdotool type --clearmodifiers '84/4='
xdotool key Escape
xdotool type --clearmodifiers '128*3='
xdotool type --clearmodifiers '+16='
sleep 0.5
xdotool mousemove --window "$window" 1090 920
import -window "$window" "$here/screens/01-evening.png"
# Locate the real theme switch and keys from native text, not assumed geometry.
locate_word() {
    import -window "$window" "$tmp/locate.png"
    python3 - "$tmp/locate.png" "$1" <<'PY'
import csv, io, subprocess, sys
out = subprocess.check_output(['tesseract', sys.argv[1], 'stdout', '--psm', '11', 'tsv'], stderr=subprocess.DEVNULL, text=True)
rows = [r for r in csv.DictReader(io.StringIO(out), delimiter='\t') if r['text'] == sys.argv[2]]
assert len(rows) == 1, (sys.argv[2], rows)
r = rows[0]
print(int(r['left']) + int(r['width']) // 2, int(r['top']) + int(r['height']) // 2)
PY
}
read -r x y < <(locate_word Moonstone)
xdotool mousemove --window "$window" "$x" "$y" click 1
sleep 0.5
xdotool mousemove --window "$window" 1090 920
import -window "$window" "$here/screens/02-moonstone.png"
# Mouse action crosses JSON button -> GPUI click -> Rhai dispatch -> native display.
read -r x y < <(locate_word AC)
xdotool mousemove --window "$window" "$x" "$y" click 1
# The renderer's key height is 48 px with a 6 px row gap; 7 is below AC.
# Tiny numeral keys are not reliably OCR-readable, unlike the large output.
xdotool mousemove --window "$window" "$x" "$((y + 54))" click 1
sleep 0.3
import -window "$window" "$tmp/click.png"
python3 - "$tmp/click.png" "$tmp/display.png" <<'PY'
from PIL import Image
import sys
Image.open(sys.argv[1]).crop((395, 378, 727, 438)).resize((996, 180)).save(sys.argv[2])
PY
tesseract "$tmp/display.png" stdout --psm 7 -c tessedit_char_whitelist=0123456789 2>/dev/null > "$tmp/click.txt"
grep -qx '7' "$tmp/click.txt" || { cat "$tmp/click.txt"; echo 'Mouse dispatch did not update display' >&2; exit 1; }
# Edit both the UI and behavior without touching the binary or invoking cargo.
python3 - "$tmp/app" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
ui = json.loads((p / 'ui.json').read_text())
ui['title'] = 'Calculator revised'
(p / 'ui.json').write_text(json.dumps(ui))
(p / 'behavior.rhai').write_text('fn dispatch(s, key) { s.display = "42"; s }')
PY
read -r x y < <(locate_word Reload)
xdotool mousemove --window "$window" "$x" "$y" click 1
sleep 0.3
read -r x y < <(locate_word AC)
xdotool mousemove --window "$window" "$x" "$((y + 54))" click 1
sleep 0.3
import -window "$window" "$tmp/reload.png"
python3 - "$tmp/reload.png" "$tmp/display.png" <<'PY'
from PIL import Image
import sys
Image.open(sys.argv[1]).crop((395, 378, 727, 438)).resize((996, 180)).save(sys.argv[2])
PY
[[ "$(tesseract "$tmp/display.png" stdout --psm 7 -c tessedit_char_whitelist=0123456789 2>/dev/null)" == '42' ]]
tesseract "$tmp/reload.png" stdout --psm 11 2>/dev/null | grep -q 'Calculator revised'
echo 'PASS: native screenshots, arithmetic/history, themes, mouse dispatch and UI/script reload without compilation'
